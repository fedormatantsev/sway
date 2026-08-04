//! The graph -> UI read path. One pure function of `&World` per frame.
//!
//! Nothing is pushed here. Main design §2.11: "The editor likewise reads
//! rather than receives: live port values come from the arena and live node
//! values from components, with nothing pushed to it."
//!
//! Everything in this module is masonry-free by design -- `capture` is
//! testable against a headless `App` with no widget tree at all, which is
//! where the bulk of this feature's tests live.

use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::name::Name;
use bevy_ecs::world::World;
use bevy_transform::components::Transform;
use kurbo::Point;
use sway_graph::{
    CompiledGraph, Edge, EdgeFrom, EdgeTo, EditorPos, FieldKind, GraphNode, NodeId, NodePlan,
    NodeTypeRegistry, PortArena,
};

/// What an edge carries, derived from the type of the inlet it lands on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EdgeKind {
    Value,
    Events,
    Product,
    /// A product edge whose capability is `Spatial` -- parenting.
    Spatial,
}

/// One graph node, as the canvas needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub entity: Entity,
    pub id: NodeId,
    /// The registered type name, shortened by [`short_type_name`].
    pub name: String,
    /// The authored [`EditorPos`], if any. The canvas treats this as a seed:
    /// read when a node box first appears and never again (design §5).
    pub pos: Option<Point>,
    /// Per inlet field, in order: how many slots it has -- 1, or a `Vec`
    /// field's instance length. A slot count, not a node-type property,
    /// because a `Vec` inlet's length is per instance (`NodePlan::field_lens`,
    /// which `compile` derives from the node's own `Inlets`).
    pub inlets: Vec<u16>,
    /// How many outlet fields this node has. Always a plain count, never
    /// per-slot: an outlet can't be a `Vec` (design §12, enforced at
    /// registration), so every outlet field is exactly one slot.
    pub outlets: u16,
}

/// One edge, with both endpoints addressed by node and field/index -- the
/// same coordinates `Edge` itself uses (spec §5), so the canvas can key a
/// socket by `(node, field, index)` without inventing its own scheme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeView {
    pub from: NodeId,
    pub from_field: u16,
    pub from_index: u16,
    pub to: NodeId,
    pub to_field: u16,
    pub to_index: u16,
    pub kind: EdgeKind,
    /// The source slot's value, when it downcasts to `f32`. Events and
    /// products get none: an event occupies one tick and a frame-rate
    /// sampler would observe it at random, and a product is a reference.
    ///
    /// Event edges are `None` **by design**, not by omission: an event
    /// occupies exactly one tick, so a frame-rate sampler observes roughly
    /// half of them and a MIDI note would pulse at random -- a worse signal
    /// than no signal. The alternative, an accumulator written by
    /// `graph_tick`, would put an editor-only write path in the hot tick,
    /// against §2.11. Design §4; revisit at M7.
    pub activity: Option<f32>,
}

/// Everything one frame of the editor reads out of the world.
#[derive(Clone, Debug, Default)]
pub struct WorldSnapshot {
    pub tree: Vec<TreeRow>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
}

/// Which section of the tree pane a row belongs to.
///
/// Grouping is what makes "all entities" readable: a flat forest of several
/// hundred roots is not. `Ord` is derived and load-bearing -- `capture`
/// emits rows in this order so the widget can insert a section header
/// whenever the group changes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum TreeGroup {
    /// Has a `Transform`; nested by `ChildOf`.
    Scene,
    /// A `GraphNode` without a `Transform` -- geometry operators, signal nodes.
    Graph,
    /// `Edge` entities.
    Edges,
    /// Everything else, including Bevy's own internals.
    Other,
}

/// One row of the world hierarchy pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub entity: Entity,
    pub group: TreeGroup,
    /// Indentation level. Always 0 outside [`TreeGroup::Scene`], which is the
    /// only group that nests.
    pub depth: usize,
    pub label: String,
    /// `Some` when this entity is a graph node, which is what lets a tree
    /// selection highlight a node box in the canvas.
    pub node_id: Option<NodeId>,
}

/// Shortens a Rust type path to its last segment, preserving generics.
///
/// `sway_nodes::lfo::LFO` -> `LFO`, and
/// `sway_nodes::material::MaterialNode<bevy_pbr::StandardMaterial>` ->
/// `MaterialNode<StandardMaterial>`.
///
/// Temporary. `NodeTypeEntry::name` is `core::any::type_name::<N>()` today;
/// M4 introduces short registered names in the project format for exactly
/// this reason, and this function is deleted when it does.
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

/// Reads one frame's worth of graph state out of the world.
///
/// Pure: takes `&World`, touches nothing. Safe to call at any point,
/// including before the graph has ever been compiled -- a world with no
/// `CompiledGraph` yields nodes and edges with no activity rather than a
/// panic, which is the state the editor is in on the very first frame.
pub fn capture(world: &World) -> WorldSnapshot {
    let nodes = capture_nodes(world);
    let edges = capture_edges(world);
    let tree = capture_tree(world);
    WorldSnapshot { tree, nodes, edges }
}

/// Node order: the compiled topological order when a `CompiledGraph` exists,
/// with any node missing from it appended in `NodeId` order; plain `NodeId`
/// order otherwise. Deterministic either way, which matters because the
/// canvas's fallback grid position is indexed by this order (design §5).
fn capture_nodes(world: &World) -> Vec<NodeView> {
    let registry = world.get_resource::<NodeTypeRegistry>();
    let plans: HashMap<Entity, &NodePlan> = world
        .get_resource::<CompiledGraph>()
        .map(|compiled| {
            compiled
                .plans
                .iter()
                .map(|plan| (plan.entity, plan))
                .collect()
        })
        .unwrap_or_default();

    let mut ordered: Vec<Entity> = Vec::new();
    if let Some(compiled) = world.get_resource::<CompiledGraph>() {
        ordered.extend(compiled.plans.iter().map(|plan| plan.entity));
    }
    let seen: Vec<Entity> = ordered.clone();

    let mut leftovers: Vec<(NodeId, Entity)> = world
        .iter_entities()
        .filter_map(|entity_ref| {
            let node = entity_ref.get::<GraphNode>()?;
            (!seen.contains(&entity_ref.id())).then_some((node.id, entity_ref.id()))
        })
        .collect();
    leftovers.sort_unstable();
    ordered.extend(leftovers.into_iter().map(|(_, entity)| entity));

    ordered
        .into_iter()
        .filter_map(|entity| {
            let node = world.get::<GraphNode>(entity)?;
            let name = registry
                .and_then(|reg| reg.get(node.node_type))
                .map(|entry| short_type_name(entry.name))
                .unwrap_or_else(|| format!("<type {}>", node.node_type.0));
            // Slot counts come from this node's own `NodePlan`, when it has
            // one -- a node absent from the last compile (freshly spawned,
            // or the graph never compiled at all) draws with no sockets
            // rather than guessing, same "degrade, don't panic" rule as
            // `capture_edges`.
            let (inlets, outlets) = plans
                .get(&entity)
                .map(|plan| {
                    let inlets: Vec<u16> = plan.field_lens[..plan.inlet_field_count]
                        .iter()
                        .map(|&len| len as u16)
                        .collect();
                    let outlets = (plan.fields.len() - plan.inlet_field_count) as u16;
                    (inlets, outlets)
                })
                .unwrap_or_default();
            Some(NodeView {
                entity,
                id: node.id,
                name,
                pos: world
                    .get::<EditorPos>(entity)
                    .map(|p| Point::new(p.0.x as f64, p.0.y as f64)),
                inlets,
                outlets,
            })
        })
        .collect()
}

fn capture_edges(world: &World) -> Vec<EdgeView> {
    let plans: HashMap<Entity, &NodePlan> = world
        .get_resource::<CompiledGraph>()
        .map(|compiled| {
            compiled
                .plans
                .iter()
                .map(|plan| (plan.entity, plan))
                .collect()
        })
        .unwrap_or_default();
    let arena = world.get_resource::<PortArena>();

    let mut edges = Vec::new();
    for entity_ref in world.iter_entities() {
        let (Some(edge), Some(EdgeFrom(source)), Some(EdgeTo(target))) = (
            entity_ref.get::<Edge>(),
            entity_ref.get::<EdgeFrom>(),
            entity_ref.get::<EdgeTo>(),
        ) else {
            continue;
        };
        let (Some(from), Some(to)) = (
            world.get::<GraphNode>(*source).map(|node| node.id),
            world.get::<GraphNode>(*target).map(|node| node.id),
        ) else {
            continue;
        };

        let kind = edge_kind(&plans, *target, edge.to.field);
        let activity = (kind == EdgeKind::Value)
            .then(|| source_f32(&plans, arena, *source, edge.from.field, edge.from.index))
            .flatten();

        edges.push(EdgeView {
            from,
            from_field: edge.from.field,
            from_index: edge.from.index,
            to,
            to_field: edge.to.field,
            to_index: edge.to.index,
            kind,
            activity,
        });
    }
    edges
}

/// What an edge carries, read off the target inlet's `FieldSpec` -- what an
/// edge *does* is decided by the type of the inlet it lands on, never by the
/// edge itself (design §2).
///
/// Falls back to [`EdgeKind::Value`] when the graph has not been compiled
/// yet: there is no schema to classify against, and the editor must still
/// draw something rather than panic (design §2.11).
fn edge_kind(plans: &HashMap<Entity, &NodePlan>, target: Entity, to_field: u16) -> EdgeKind {
    plans
        .get(&target)
        .and_then(|plan| plan.fields.get(to_field as usize))
        .map(|field| match field.kind {
            FieldKind::Value => EdgeKind::Value,
            FieldKind::Events { .. } => EdgeKind::Events,
            FieldKind::Product { spatial: true, .. } => EdgeKind::Spatial,
            FieldKind::Product { spatial: false, .. } => EdgeKind::Product,
        })
        .unwrap_or(EdgeKind::Value)
}

/// The source outlet's slot, downcast to `f32`. `None` when the graph has
/// not been compiled, the field/index is out of range, or the slot holds
/// anything other than an `f32` -- never a panic (design §2.11).
fn source_f32(
    plans: &HashMap<Entity, &NodePlan>,
    arena: Option<&PortArena>,
    source: Entity,
    field: u16,
    index: u16,
) -> Option<f32> {
    let plan = plans.get(&source)?;
    let offset = *plan.field_offsets.get(field as usize)?;
    let slot = plan.base + offset + index as usize;
    arena?.values.get(slot)?.try_downcast_ref::<f32>().copied()
}

fn group_of(world: &World, entity: Entity) -> TreeGroup {
    if world.get::<Transform>(entity).is_some() {
        TreeGroup::Scene
    } else if world.get::<GraphNode>(entity).is_some() {
        TreeGroup::Graph
    } else if world.get::<Edge>(entity).is_some() {
        TreeGroup::Edges
    } else {
        TreeGroup::Other
    }
}

/// Best-effort row label: a `Name` wins; then a `GraphNode`'s shortened type
/// name and `NodeId`; then the entity index and its first three component
/// names, shortened the same way.
fn row_label(world: &World, entity: Entity) -> String {
    if let Some(name) = world.get::<Name>(entity) {
        return name.to_string();
    }
    if let Some(node) = world.get::<GraphNode>(entity) {
        let type_name = world
            .get_resource::<NodeTypeRegistry>()
            .and_then(|reg| reg.get(node.node_type))
            .map(|entry| short_type_name(entry.name))
            .unwrap_or_else(|| format!("<type {}>", node.node_type.0));
        return format!("{type_name} #{}", node.id.0);
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
    let mut rows: Vec<TreeRow> = Vec::new();

    // Scene: roots first, then their `Children` depth-first. A `Transform`
    // entity whose parent has no `Transform` is a root here too -- it has
    // nowhere else to nest.
    let mut scene_roots: Vec<Entity> = world
        .iter_entities()
        .filter(|entity_ref| entity_ref.contains::<Transform>())
        .filter(|entity_ref| match entity_ref.get::<ChildOf>() {
            Some(ChildOf(parent)) => world.get::<Transform>(*parent).is_none(),
            None => true,
        })
        .map(|entity_ref| entity_ref.id())
        .collect();
    scene_roots.sort_unstable();
    for root in scene_roots {
        push_scene_subtree(world, root, 0, &mut rows);
    }

    // The flat groups, each sorted by entity for a stable order across frames.
    for group in [TreeGroup::Graph, TreeGroup::Edges, TreeGroup::Other] {
        let mut members: Vec<Entity> = world
            .iter_entities()
            .map(|entity_ref| entity_ref.id())
            .filter(|&entity| group_of(world, entity) == group)
            .collect();
        members.sort_unstable();
        rows.extend(members.into_iter().map(|entity| TreeRow {
            entity,
            group,
            depth: 0,
            label: row_label(world, entity),
            node_id: world.get::<GraphNode>(entity).map(|node| node.id),
        }));
    }

    rows
}

fn push_scene_subtree(world: &World, entity: Entity, depth: usize, rows: &mut Vec<TreeRow>) {
    rows.push(TreeRow {
        entity,
        group: TreeGroup::Scene,
        depth,
        label: row_label(world, entity),
        node_id: world.get::<GraphNode>(entity).map(|node| node.id),
    });
    if let Some(children) = world.get::<Children>(entity) {
        let mut spatial: Vec<Entity> = children
            .iter()
            .copied()
            .filter(|&child| world.get::<Transform>(child).is_some())
            .collect();
        spatial.sort_unstable();
        for child in spatial {
            push_scene_subtree(world, child, depth + 1, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_graph::{
        Emit, Recv, app, connect, fixture_with_parenting, recompile, spawn_emit,
        spawn_named_spatial, spawn_recv, spawn_spatial,
    };
    use bevy_math::Vec2;
    use kurbo::Point;

    #[test]
    fn short_type_name_strips_module_paths() {
        assert_eq!(short_type_name("sway_nodes::lfo::LFO"), "LFO");
        assert_eq!(
            short_type_name("sway_nodes::material::MaterialNode<bevy_pbr::StandardMaterial>"),
            "MaterialNode<StandardMaterial>"
        );
        assert_eq!(short_type_name("f32"), "f32");
    }

    #[test]
    fn nodes_carry_their_id_short_name_and_authored_position() {
        let mut app = app();
        spawn_emit(app.world_mut(), 7, Some(Vec2::new(20.0, 140.0)));
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.nodes.len(), 1);
        assert_eq!(snap.nodes[0].id.0, 7);
        assert_eq!(snap.nodes[0].name, "Emit");
        assert_eq!(snap.nodes[0].pos, Some(Point::new(20.0, 140.0)));
    }

    #[test]
    fn a_node_without_editor_pos_has_no_position() {
        let mut app = app();
        spawn_emit(app.world_mut(), 0, None);
        recompile(&mut app);

        assert_eq!(capture(app.world()).nodes[0].pos, None);
    }

    #[test]
    fn every_edge_carries_both_of_its_endpoints() {
        let (app, ids) = fixture_with_parenting();
        let snap = capture(app.world());

        let parenting = snap
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Spatial)
            .expect("a parenting edge must appear in the snapshot");
        assert_eq!(parenting.from, ids.child);
        assert_eq!(parenting.to, ids.parent);
        // The canvas needs a socket at each end; before this milestone
        // parenting had neither and was dropped from the snapshot entirely.
        assert_eq!(parenting.to_index, 0, "children[0]");
    }

    #[test]
    fn edge_kinds_distinguish_what_an_edge_carries() {
        let (app, _) = fixture_with_parenting();
        let snap = capture(app.world());
        let kinds: std::collections::HashSet<_> = snap.edges.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EdgeKind::Value));
        assert!(kinds.contains(&EdgeKind::Events));
        assert!(kinds.contains(&EdgeKind::Product));
        assert!(kinds.contains(&EdgeKind::Spatial));
    }

    #[test]
    fn activity_is_some_only_for_an_f32_value_edge() {
        let (app, _) = fixture_with_parenting();
        let snap = capture(app.world());
        for edge in &snap.edges {
            match edge.kind {
                EdgeKind::Value => {}
                _ => assert!(
                    edge.activity.is_none(),
                    "only value edges carry a sampled value"
                ),
            }
        }
    }

    #[test]
    fn nodes_follow_compiled_topological_order() {
        // `recv` is spawned first but depends on `emit`, so the compiled order
        // puts `emit` first. The snapshot must follow that order, because the
        // fallback grid position is indexed by it (design §5).
        let mut app = app();
        let recv = spawn_recv(app.world_mut(), 1, None);
        let emit = spawn_emit(app.world_mut(), 0, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.nodes[0].entity, emit);
        assert_eq!(snap.nodes[1].entity, recv);
    }

    fn rows_of(snap: &WorldSnapshot, group: TreeGroup) -> Vec<&TreeRow> {
        snap.tree.iter().filter(|row| row.group == group).collect()
    }

    #[test]
    fn rows_are_emitted_in_group_order() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let groups: Vec<TreeGroup> = capture(app.world())
            .tree
            .iter()
            .map(|row| row.group)
            .collect();

        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "rows must be emitted grouped, never interleaved");
    }

    #[test]
    fn a_spatial_node_is_in_scene_and_a_signal_node_is_in_graph() {
        let mut app = app();
        let spatial = spawn_spatial(app.world_mut(), 0, None);
        let signal = spawn_emit(app.world_mut(), 1, None);
        recompile(&mut app);

        let snap = capture(app.world());

        assert!(rows_of(&snap, TreeGroup::Scene).iter().any(|r| r.entity == spatial));
        assert!(rows_of(&snap, TreeGroup::Graph).iter().any(|r| r.entity == signal));
    }

    #[test]
    fn scene_rows_nest_by_child_of() {
        let mut app = app();
        let parent = spawn_spatial(app.world_mut(), 0, None);
        let child = spawn_spatial(app.world_mut(), 1, Some(parent));
        recompile(&mut app);

        let snap = capture(app.world());
        let scene = rows_of(&snap, TreeGroup::Scene);
        let parent_idx = scene.iter().position(|r| r.entity == parent).unwrap();
        let child_idx = scene.iter().position(|r| r.entity == child).unwrap();

        assert!(parent_idx < child_idx, "a parent must precede its child");
        assert_eq!(scene[parent_idx].depth, 0);
        assert_eq!(scene[child_idx].depth, 1);
    }

    #[test]
    fn a_name_component_wins_over_the_node_type() {
        let mut app = app();
        let named = spawn_named_spatial(app.world_mut(), "key light");

        let snap = capture(app.world());
        let row = snap.tree.iter().find(|r| r.entity == named).unwrap();

        assert_eq!(row.label, "key light");
        assert_eq!(row.node_id, None);
    }

    #[test]
    fn a_graph_node_row_is_labelled_by_type_and_node_id() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 7, None);
        recompile(&mut app);

        let snap = capture(app.world());
        let row = snap.tree.iter().find(|r| r.entity == emit).unwrap();

        assert_eq!(row.label, "Emit #7");
        assert_eq!(row.node_id.map(|id| id.0), Some(7));
    }

    #[test]
    fn edge_entities_are_grouped_under_edges() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());
        assert_eq!(rows_of(&snap, TreeGroup::Edges).len(), 1);
    }

    #[test]
    fn every_entity_in_the_world_gets_exactly_one_row() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        spawn_emit(app.world_mut(), 1, None);
        recompile(&mut app);

        let snap = capture(app.world());
        let entity_count = app.world().iter_entities().count();

        assert_eq!(snap.tree.len(), entity_count);
        let mut entities: Vec<_> = snap.tree.iter().map(|r| r.entity).collect();
        entities.sort();
        let before = entities.len();
        entities.dedup();
        assert_eq!(entities.len(), before, "no entity may appear twice");
    }
}
