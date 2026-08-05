//! The graph-to-UI read path. One pure function of `&World` per frame.

use std::collections::HashSet;

use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::name::Name;
use bevy_ecs::world::World;
use bevy_transform::components::Transform;
use kurbo::Point;
use sway_graph::order::{GraphOrder, Step};
use sway_graph::{EditorPos, GraphDiagnostics, TransportTime, WireRegistry};

/// The editor's display key for a node box.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn of(entity: Entity) -> Self {
        Self(entity.index().index())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub entity: Entity,
    pub id: NodeId,
    pub name: String,
    pub pos: Option<Point>,
    pub inlets: Vec<u16>,
    pub outlets: u16,
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

fn capture_nodes(world: &World) -> Vec<NodeView> {
    let Some(registry) = world.get_resource::<WireRegistry>() else {
        return Vec::new();
    };
    graph_entities(world)
        .into_iter()
        .map(|entity| {
            let inlets = registry
                .entries
                .iter()
                .filter(|entry| (entry.has_target)(world, entity))
                .count();
            let outlets = registry
                .entries
                .iter()
                .filter(|entry| (entry.has_source)(world, entity))
                .count() as u16;
            NodeView {
                entity,
                id: NodeId::of(entity),
                name: world
                    .get::<Name>(entity)
                    .map(|name| name.as_str().to_string())
                    .unwrap_or_else(|| format!("Entity {}", entity.index())),
                pos: world
                    .get::<EditorPos>(entity)
                    .map(|pos| Point::new(pos.0.x as f64, pos.0.y as f64)),
                inlets: vec![1; inlets],
                outlets,
            }
        })
        .collect()
}

fn capture_edges(world: &World) -> Vec<EdgeView> {
    let Some(order) = world.get_resource::<GraphOrder>() else {
        return Vec::new();
    };
    order
        .steps
        .iter()
        .filter_map(|step| match *step {
            Step::Propagate { src, dst, wire, .. } => Some(EdgeView {
                from: NodeId::of(src),
                from_field: 0,
                from_index: 0,
                to: NodeId::of(dst),
                to_field: 0,
                to_index: 0,
                wire,
                activity: None,
            }),
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
        Emit, Recv, app, connect, fixture_with_parenting, recompile, spawn_emit,
        spawn_named_spatial, spawn_recv, spawn_spatial,
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
}
