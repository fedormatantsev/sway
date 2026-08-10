//! The graph-to-UI read path. One pure function of `&World` per frame.

use std::collections::HashSet;

use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::name::Name;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::{PartialReflect, ReflectRef};
use bevy_transform::components::Transform;
use kurbo::Point;
use sway_graph::order::{GraphOrder, Step};
use sway_graph::{ComponentDocRegistry, EditorPos, GraphDiagnostics, TransportTime, WireRegistry};

/// The editor's display key for a node box.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn of(entity: Entity) -> Self {
        Self(entity.index().index())
    }
}

/// One inlet socket: a registered wire type this entity could consume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InletView {
    pub wire: &'static str,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub entity: Entity,
    pub id: NodeId,
    pub name: String,
    pub pos: Option<Point>,
    pub inlets: Vec<InletView>,
    /// 0 or 1. Architecture §2: an outlet is a *component*, and no node in the
    /// current set carries two distinct source component types (spec M6-6).
    pub outlets: u16,
}

/// What kind of editor a field wants. The widget layer switches on this; the
/// snapshot layer never builds a widget.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Float,
    Int,
    Bool,
    /// Every variant name, current one included.
    Enum(Vec<String>),
    Str,
    Vec3,
    /// Anything the walk could not classify — rendered read-only, and the
    /// signal that a type wants editor `TypeData`.
    Opaque,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeView {
    pub from: NodeId,
    pub from_field: u16,
    pub from_index: u16,
    pub to: NodeId,
    pub to_field: u16,
    pub to_index: u16,
    pub wire: &'static str,
    pub activity: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct WorldSnapshot {
    pub tree: Vec<TreeRow>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
    pub diagnostics: GraphDiagnostics,
    pub transport: TransportView,
    pub inspector: InspectorView,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorField {
    /// A struct field's reflected name, or a tuple-struct field's index as a
    /// string — which is why this is owned and `InspectorComponent::name` is not.
    pub name: String,
    pub value: String,
    pub kind: FieldKind,
}

/// One component's authored fields, flattened for display.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorComponent {
    /// `ComponentEntry::name` verbatim, so Task 8 can put it straight into an
    /// `EditorCommand::SetField` without leaking a `String`.
    pub name: &'static str,
    pub fields: Vec<InspectorField>,
}

/// What the inspector pane shows for the current selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorView {
    pub entity: Option<Entity>,
    pub components: Vec<InspectorComponent>,
}

/// The authorable components on `entity`, walked by reflection.
///
/// The same walk the project format's reader and emitter perform, which is the
/// point: this is what finally exercises editor `TypeData` (parent spec §7).
pub fn inspect(world: &World, entity: Entity) -> InspectorView {
    let (Some(docs), Some(registry)) = (
        world.get_resource::<ComponentDocRegistry>(),
        world.get_resource::<AppTypeRegistry>(),
    ) else {
        return InspectorView::default();
    };
    let registry = registry.read();
    let Ok(entity_ref) = world.get_entity(entity) else {
        return InspectorView::default();
    };

    let mut components = Vec::new();
    for entry in &docs.entries {
        let Some(registration) = registry.get(entry.type_id) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(value) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        components.push(InspectorComponent {
            name: entry.name,
            fields: fields_of(value.as_partial_reflect()),
        });
    }

    InspectorView { entity: Some(entity), components }
}

fn fields_of(value: &dyn PartialReflect) -> Vec<InspectorField> {
    match value.reflect_ref() {
        ReflectRef::Struct(s) => (0..s.field_len())
            .map(|i| {
                let field = s.field_at(i).expect("index in range");
                InspectorField {
                    name: s.name_at(i).unwrap_or("?").to_string(),
                    value: format_value(field),
                    kind: kind_of(field),
                }
            })
            .collect(),
        ReflectRef::TupleStruct(t) => (0..t.field_len())
            .map(|i| {
                let field = t.field(i).expect("index in range");
                InspectorField {
                    name: i.to_string(),
                    value: format_value(field),
                    kind: kind_of(field),
                }
            })
            .collect(),
        ReflectRef::Enum(e) => vec![InspectorField {
            name: "variant".to_string(),
            value: e.variant_name().to_string(),
            kind: enum_kind(value),
        }],
        _ => vec![InspectorField {
            name: String::new(),
            value: format_value(value),
            kind: kind_of(value),
        }],
    }
}

fn kind_of(value: &dyn PartialReflect) -> FieldKind {
    if value.try_downcast_ref::<f32>().is_some() || value.try_downcast_ref::<f64>().is_some() {
        return FieldKind::Float;
    }
    if value.try_downcast_ref::<i64>().is_some()
        || value.try_downcast_ref::<i32>().is_some()
        || value.try_downcast_ref::<u32>().is_some()
        || value.try_downcast_ref::<usize>().is_some()
    {
        return FieldKind::Int;
    }
    if value.try_downcast_ref::<bool>().is_some() {
        return FieldKind::Bool;
    }
    if value.try_downcast_ref::<String>().is_some() {
        return FieldKind::Str;
    }
    if value.try_downcast_ref::<bevy_math::Vec3>().is_some() {
        return FieldKind::Vec3;
    }
    if matches!(value.reflect_ref(), ReflectRef::Enum(_)) {
        return enum_kind(value);
    }
    FieldKind::Opaque
}

/// Every variant name of an enum field, from its `TypeInfo` — the current
/// value only tells us one of them, and the editor needs the whole list.
fn enum_kind(value: &dyn PartialReflect) -> FieldKind {
    let Some(bevy_reflect::TypeInfo::Enum(info)) = value.get_represented_type_info() else {
        return FieldKind::Opaque;
    };
    FieldKind::Enum(
        info.iter()
            .map(|variant| variant.name().to_string())
            .collect(),
    )
}

/// Renders the types a set actually uses; anything else falls back to its
/// debug form, which is the signal that the type wants editor `TypeData`.
fn format_value(value: &dyn PartialReflect) -> String {
    if let Some(v) = value.try_downcast_ref::<f32>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<f64>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<bool>() {
        return v.to_string();
    }
    if let Some(v) = value.try_downcast_ref::<u32>() {
        return v.to_string();
    }
    if let Some(v) = value.try_downcast_ref::<String>() {
        return v.clone();
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec2>() {
        return format!("{:.2}, {:.2}", v.x, v.y);
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec3>() {
        return format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z);
    }
    if let ReflectRef::Enum(e) = value.reflect_ref() {
        return e.variant_name().to_string();
    }
    format!("{value:?}")
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TransportView {
    pub playing: bool,
    pub bpm: f32,
    pub position: String,
    pub locked: bool,
}

fn capture_transport(world: &World) -> TransportView {
    let Some(time) = world.get_resource::<bevy_time::Time<sway_graph::Transport>>() else {
        return TransportView::default();
    };
    TransportView {
        playing: time.is_playing(),
        bpm: time.bpm() as f32,
        position: time.position().to_string(),
        locked: time.transport().locked,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum TreeGroup {
    Scene,
    Graph,
    Edges,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub entity: Entity,
    pub group: TreeGroup,
    pub depth: usize,
    pub label: String,
    pub node_id: Option<NodeId>,
}

pub fn short_type_name(path: &str) -> String {
    fn last_segment(s: &str) -> &str {
        match s.rfind("::") {
            Some(i) => &s[i + 2..],
            None => s,
        }
    }

    let mut out = String::with_capacity(path.len());
    let mut segment_start = 0;
    for (i, ch) in path.char_indices() {
        if matches!(ch, '<' | '>' | ',' | ' ') {
            out.push_str(last_segment(&path[segment_start..i]));
            out.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
    out.push_str(last_segment(&path[segment_start..]));
    out
}

pub fn capture(world: &World) -> WorldSnapshot {
    WorldSnapshot {
        tree: capture_tree(world),
        nodes: capture_nodes(world),
        edges: capture_edges(world),
        diagnostics: world
            .get_resource::<GraphDiagnostics>()
            .cloned()
            .unwrap_or_default(),
        transport: capture_transport(world),
        inspector: InspectorView::default(),
    }
}

fn graph_entities(world: &World) -> Vec<Entity> {
    let Some(order) = world.get_resource::<GraphOrder>() else {
        return Vec::new();
    };
    let mut entities = Vec::new();
    for step in &order.steps {
        match *step {
            Step::Propagate { src, dst, .. } => {
                entities.push(src);
                entities.push(dst);
            }
            Step::Run { entity, .. } => entities.push(entity),
        }
    }
    entities.sort();
    entities.dedup();
    entities
}

/// Every entity the canvas draws: those carrying `EditorPos` (spec M6-4).
fn canvas_entities(world: &World) -> Vec<Entity> {
    let mut entities: Vec<Entity> = world
        .iter_entities()
        .filter(|entity| entity.contains::<EditorPos>())
        .map(|entity| entity.id())
        .collect();
    entities.sort();
    entities
}

/// The inlet sockets of one entity, in `WireRegistry` order — which is
/// registration order, fixed at startup, so a socket's ordinal is stable.
fn inlets_of(world: &World, registry: &WireRegistry, entity: Entity) -> Vec<InletView> {
    registry
        .entries
        .iter()
        .filter(|entry| (entry.has_target)(world, entity))
        .map(|entry| InletView {
            wire: entry.name,
            connected: (entry.read)(world, entity).is_some(),
        })
        .collect()
}

fn capture_nodes(world: &World) -> Vec<NodeView> {
    let Some(registry) = world.get_resource::<WireRegistry>() else {
        return Vec::new();
    };
    canvas_entities(world)
        .into_iter()
        .map(|entity| NodeView {
            entity,
            id: NodeId::of(entity),
            name: world
                .get::<Name>(entity)
                .map(|name| name.as_str().to_string())
                .unwrap_or_else(|| format!("Entity {}", entity.index())),
            pos: world
                .get::<EditorPos>(entity)
                .map(|pos| Point::new(pos.0.x as f64, pos.0.y as f64)),
            inlets: inlets_of(world, registry, entity),
            outlets: registry
                .entries
                .iter()
                .any(|entry| (entry.has_source)(world, entity)) as u16,
        })
        .collect()
}

fn capture_edges(world: &World) -> Vec<EdgeView> {
    let (Some(order), Some(registry)) = (
        world.get_resource::<GraphOrder>(),
        world.get_resource::<WireRegistry>(),
    ) else {
        return Vec::new();
    };
    order
        .steps
        .iter()
        .filter_map(|step| match *step {
            Step::Propagate { src, dst, wire, .. } => {
                // The inlet ordinal, so the edge lands on its own socket
                // rather than always on socket 0 (spec M6-6).
                let to_field = inlets_of(world, registry, dst)
                    .iter()
                    .position(|inlet| inlet.wire == wire)
                    .unwrap_or(0) as u16;
                Some(EdgeView {
                    from: NodeId::of(src),
                    from_field: 0,
                    from_index: 0,
                    to: NodeId::of(dst),
                    to_field,
                    to_index: 0,
                    wire,
                    activity: None,
                })
            }
            Step::Run { .. } => None,
        })
        .collect()
}

fn group_of(world: &World, graph: &HashSet<Entity>, entity: Entity) -> TreeGroup {
    if world.get::<Transform>(entity).is_some() {
        TreeGroup::Scene
    } else if graph.contains(&entity) {
        TreeGroup::Graph
    } else {
        TreeGroup::Other
    }
}

fn row_label(world: &World, entity: Entity) -> String {
    if let Some(name) = world.get::<Name>(entity) {
        return name.to_string();
    }
    let components: Vec<String> = world
        .inspect_entity(entity)
        .map(|infos| {
            infos
                .take(3)
                .map(|info| short_type_name(&info.name().shortname().to_string()))
                .collect()
        })
        .unwrap_or_default();
    if components.is_empty() {
        format!("e{}", entity.index())
    } else {
        format!("e{} [{}]", entity.index(), components.join(", "))
    }
}

fn capture_tree(world: &World) -> Vec<TreeRow> {
    let graph: HashSet<Entity> = graph_entities(world).into_iter().collect();
    let mut rows = Vec::new();
    let mut scene_roots: Vec<Entity> = world
        .iter_entities()
        .filter(|entity| entity.contains::<Transform>())
        .filter(|entity| match entity.get::<ChildOf>() {
            Some(ChildOf(parent)) => world.get::<Transform>(*parent).is_none(),
            None => true,
        })
        .map(|entity| entity.id())
        .collect();
    scene_roots.sort_unstable();
    for root in scene_roots {
        push_scene_subtree(world, &graph, root, 0, &mut rows);
    }

    for group in [TreeGroup::Graph, TreeGroup::Edges, TreeGroup::Other] {
        let mut members: Vec<Entity> = world
            .iter_entities()
            .map(|entity| entity.id())
            .filter(|entity| group_of(world, &graph, *entity) == group)
            .collect();
        members.sort_unstable();
        rows.extend(members.into_iter().map(|entity| TreeRow {
            entity,
            group,
            depth: 0,
            label: row_label(world, entity),
            node_id: graph.contains(&entity).then(|| NodeId::of(entity)),
        }));
    }
    rows
}

fn push_scene_subtree(
    world: &World,
    graph: &HashSet<Entity>,
    entity: Entity,
    depth: usize,
    rows: &mut Vec<TreeRow>,
) {
    rows.push(TreeRow {
        entity,
        group: TreeGroup::Scene,
        depth,
        label: row_label(world, entity),
        node_id: graph.contains(&entity).then(|| NodeId::of(entity)),
    });
    if let Some(children) = world.get::<Children>(entity) {
        let mut spatial: Vec<Entity> = children
            .iter()
            .copied()
            .filter(|child| world.get::<Transform>(*child).is_some())
            .collect();
        spatial.sort_unstable();
        for child in spatial {
            push_scene_subtree(world, graph, child, depth + 1, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_graph::{
        Emit, Recv, app, connect, fixture_with_parenting, recompile, spawn_double_source,
        spawn_double_target, spawn_emit, spawn_named_spatial, spawn_recv, spawn_spatial,
    };
    use bevy_math::Vec2;

    fn rows_of(snapshot: &WorldSnapshot, group: TreeGroup) -> Vec<&TreeRow> {
        snapshot
            .tree
            .iter()
            .filter(|row| row.group == group)
            .collect()
    }

    #[test]
    fn short_type_name_strips_module_paths() {
        assert_eq!(short_type_name("sway_nodes::lfo::LFO"), "LFO");
        assert_eq!(short_type_name("bevy::asset::Handle<Mesh>"), "Handle<Mesh>");
    }

    #[test]
    fn nodes_use_entity_ids_names_and_authored_positions() {
        let mut app = app();
        let entity = spawn_emit(app.world_mut(), 7, Some(Vec2::new(20.0, 140.0)));
        let recv = spawn_recv(app.world_mut(), 8, None);
        connect(app.world_mut(), entity, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snapshot = capture(app.world());
        let node = snapshot.nodes.iter().find(|node| node.entity == entity).unwrap();
        assert_eq!(node.id, NodeId::of(entity));
        assert_eq!(node.name, "Emit");
        assert_eq!(node.pos, Some(Point::new(20.0, 140.0)));
        assert_eq!(node.outlets, 1);
    }

    #[test]
    fn edges_carry_wire_names_and_entity_endpoints() {
        let (app, ids) = fixture_with_parenting();
        let snapshot = capture(app.world());
        let parent = snapshot
            .edges
            .iter()
            .find(|edge| edge.wire == "parent")
            .expect("parenting wire");
        assert_eq!(parent.from, NodeId::of(ids.parent));
        assert_eq!(parent.to, NodeId::of(ids.child));
        assert!(snapshot.edges.iter().any(|edge| edge.wire == "amount"));
    }

    #[test]
    fn diagnostics_are_copied_into_the_snapshot() {
        let mut app = app();
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<GraphDiagnostics>().cycles.push(entity);

        assert_eq!(capture(app.world()).diagnostics.cycles, vec![entity]);
    }

    #[test]
    fn rows_are_emitted_in_group_order() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);
        let groups: Vec<_> = capture(app.world()).tree.iter().map(|row| row.group).collect();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted);
    }

    #[test]
    fn scene_rows_nest_by_child_of() {
        let mut app = app();
        let parent = spawn_spatial(app.world_mut(), 0, None);
        let child = spawn_spatial(app.world_mut(), 1, Some(parent));
        recompile(&mut app);
        let snapshot = capture(app.world());
        let scene = rows_of(&snapshot, TreeGroup::Scene);
        let parent_index = scene.iter().position(|row| row.entity == parent).unwrap();
        let child_index = scene.iter().position(|row| row.entity == child).unwrap();
        assert!(parent_index < child_index);
        assert_eq!(scene[parent_index].depth, 0);
        assert_eq!(scene[child_index].depth, 1);
    }

    #[test]
    fn a_name_component_wins_for_tree_labels() {
        let mut app = app();
        let named = spawn_named_spatial(app.world_mut(), "key light");
        let snapshot = capture(app.world());
        let row = snapshot.tree.iter().find(|row| row.entity == named).unwrap();
        assert_eq!(row.label, "key light");
    }

    #[test]
    fn the_snapshot_carries_the_transport_readout() {
        let mut app = app();
        {
            let mut time = app
                .world_mut()
                .resource_mut::<bevy_time::Time<sway_graph::Transport>>();
            time.transport_mut().state = sway_graph::TransportState::Playing;
            time.transport_mut().secs_per_beat = 60.0 / 128.0;
            time.transport_mut().locked = true;
            time.advance_by(core::time::Duration::from_secs_f64(17.5));
            time.reposition(17.5);
        }
        let snapshot = capture(app.world());
        assert!(snapshot.transport.playing);
        assert!(snapshot.transport.locked);
        assert!((snapshot.transport.bpm - 128.0).abs() < 0.01);
        assert_eq!(snapshot.transport.position, "005.2.3");
    }

    #[test]
    fn every_entity_gets_exactly_one_tree_row() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);
        let snapshot = capture(app.world());
        assert_eq!(snapshot.tree.len(), app.world().iter_entities().count());
        let mut entities: Vec<_> = snapshot.tree.iter().map(|row| row.entity).collect();
        entities.sort();
        let before = entities.len();
        entities.dedup();
        assert_eq!(entities.len(), before);
    }

    #[test]
    fn the_inspector_lists_authorable_components_and_their_fields() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app
            .world_mut()
            .spawn(sway_nodes::Lfo { beats: 4.0, shape: sway_nodes::Waveform::Saw, phase: 0.25, amplitude: 0.5 })
            .id();

        let view = inspect(app.world(), entity);

        let lfo = view
            .components
            .iter()
            .find(|c| c.name == "Lfo")
            .expect("Lfo is authorable and present");
        assert_eq!(lfo.fields.len(), 4);
        assert_eq!(lfo.fields[0].name, "beats");
        assert_eq!(lfo.fields[0].value, "4.000");
        assert!(lfo.fields.iter().any(|f| f.name == "shape" && f.value == "Saw"));
    }

    #[test]
    fn a_component_the_entity_does_not_have_is_not_listed() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app.world_mut().spawn(sway_nodes::FloatOut(1.0)).id();

        let view = inspect(app.world(), entity);

        assert_eq!(view.components.len(), 1);
        assert_eq!(view.components[0].name, "FloatOut");
    }

    #[test]
    fn inspecting_a_dead_entity_is_empty_not_a_panic() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin);
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(entity);

        assert_eq!(inspect(app.world(), entity), InspectorView::default());
    }

    #[test]
    fn a_node_with_no_wires_still_appears_on_the_canvas() {
        // M6-4. Before M6 the canvas drew only entities in GraphOrder, so a
        // camera or light was structurally invisible.
        let mut app = app();
        let lonely = app.world_mut().spawn(EditorPos(Vec2::new(3.0, 4.0))).id();
        recompile(&mut app);

        let snapshot = capture(app.world());

        assert!(
            snapshot.nodes.iter().any(|node| node.entity == lonely),
            "an EditorPos entity is a canvas node whether or not anything wires to it",
        );
    }

    #[test]
    fn an_entity_without_an_editor_pos_is_not_a_canvas_node() {
        let mut app = app();
        let runtime_owned = app.world_mut().spawn_empty().id();
        recompile(&mut app);

        assert!(!capture(app.world()).nodes.iter().any(|n| n.entity == runtime_owned));
    }

    #[test]
    fn an_entity_sourcing_several_wires_reports_exactly_one_outlet() {
        // M6-6. Counting per wire drew seven dots on an Lfo; only socket 0 has
        // ever had an edge attached, because capture_edges hardcodes from_field.
        // `spawn_double_source` sources both `amount` and `parent`, so the old
        // `count()` reported 2 here and the new `any()` reports 1.
        let mut app = app();
        let both = spawn_double_source(app.world_mut());
        recompile(&mut app);

        let node = capture(app.world()).nodes.into_iter().find(|n| n.entity == both).unwrap();
        assert_eq!(node.outlets, 1);
    }

    #[test]
    fn inlets_carry_their_wire_name_and_whether_they_are_connected() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let node = capture(app.world()).nodes.into_iter().find(|n| n.entity == recv).unwrap();
        let amount = node.inlets.iter().find(|i| i.wire == "amount").expect("named inlet");
        assert!(amount.connected);
    }

    #[test]
    fn an_edge_lands_on_its_own_wires_inlet_ordinal_not_on_socket_zero() {
        // The to_field defect M6-6 names: every inbound edge used to draw into
        // socket 0 whatever wire it was.
        //
        // This needs a target with more than one inlet, or the assertion is
        // vacuous — `spawn_double_target` carries both `Gain` (target of
        // `amount`) and `Transform` (target of `parent`), and `WireRegistry`
        // order is registration order, so its inlets are `[amount, parent]`.
        // The `parent` edge must therefore land on ordinal 1, which is exactly
        // what the hardcoded `to_field: 0` got wrong.
        let mut app = app();
        let grandparent = spawn_spatial(app.world_mut(), 1, None);
        let both = spawn_double_target(app.world_mut(), Some(grandparent));
        recompile(&mut app);

        let snapshot = capture(app.world());
        let node = snapshot.nodes.iter().find(|n| n.entity == both).unwrap();
        assert_eq!(
            node.inlets.iter().map(|i| i.wire).collect::<Vec<_>>(),
            vec!["amount", "parent"],
            "inlet ordinals come from WireRegistry order",
        );

        let edge = snapshot
            .edges
            .iter()
            .find(|e| e.wire == "parent" && e.to == NodeId::of(both))
            .expect("the parenting edge");
        assert_eq!(edge.to_field, 1);
    }

    #[test]
    fn inspector_fields_carry_a_kind_the_widget_layer_can_switch_on() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app
            .world_mut()
            .spawn(sway_nodes::Lfo {
                beats: 4.0,
                shape: sway_nodes::Waveform::Saw,
                phase: 0.25,
                amplitude: 0.5,
            })
            .id();

        let view = inspect(app.world(), entity);
        let lfo = view.components.iter().find(|c| c.name == "Lfo").unwrap();

        let beats = lfo.fields.iter().find(|f| f.name == "beats").unwrap();
        assert_eq!(beats.kind, FieldKind::Float);

        let shape = lfo.fields.iter().find(|f| f.name == "shape").unwrap();
        match &shape.kind {
            FieldKind::Enum(variants) => {
                assert!(variants.iter().any(|v| v == "Saw"), "got {variants:?}");
                assert!(variants.len() > 1, "every variant is offered, not just the current one");
            }
            other => panic!("expected an enum kind, got {other:?}"),
        }
    }
}
