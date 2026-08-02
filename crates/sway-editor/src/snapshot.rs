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
    CompiledGraph, EdgeFrom, EdgeTo, EditorPos, FeedsEdge, GraphNode, NodeId, NodePlan,
    NodeTypeRegistry, ParamEdge, ParentEdge, PortArena, PortKind,
};

/// Which kind of edge this is. `ParentEdge` is deliberately absent: the tree
/// pane shows parenting already, and drawing it twice makes the canvas harder
/// to read for no gain (design §9).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EdgeKind {
    Param,
    Feeds,
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
}

/// One edge, with both endpoints resolved to indices into
/// [`WorldSnapshot::nodes`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeView {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// The source port's current value, when it is a continuous port holding
    /// an `f32`.
    ///
    /// `None` for every event edge, every `Feeds` edge, and every continuous
    /// edge carrying something other than an `f32` (a colour, a vector).
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
    /// `ParamEdge` / `FeedsEdge` / `ParentEdge` entities.
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
    let index: HashMap<Entity, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.entity, i))
        .collect();
    let edges = capture_edges(world, &index);
    let tree = capture_tree(world);
    WorldSnapshot { tree, nodes, edges }
}

/// Node order: the compiled topological order when a `CompiledGraph` exists,
/// with any node missing from it appended in `NodeId` order; plain `NodeId`
/// order otherwise. Deterministic either way, which matters because the
/// canvas's fallback grid position is indexed by this order (design §5).
fn capture_nodes(world: &World) -> Vec<NodeView> {
    let registry = world.get_resource::<NodeTypeRegistry>();

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
            Some(NodeView {
                entity,
                id: node.id,
                name,
                pos: world
                    .get::<EditorPos>(entity)
                    .map(|p| Point::new(p.0.x as f64, p.0.y as f64)),
            })
        })
        .collect()
}

fn capture_edges(world: &World, index: &HashMap<Entity, usize>) -> Vec<EdgeView> {
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
        let (Some(EdgeFrom(source)), Some(EdgeTo(target))) =
            (entity_ref.get::<EdgeFrom>(), entity_ref.get::<EdgeTo>())
        else {
            continue;
        };
        let (Some(&from), Some(&to)) = (index.get(source), index.get(target)) else {
            continue;
        };

        if let Some(param) = entity_ref.get::<ParamEdge>() {
            let activity = match param.kind {
                PortKind::Continuous => continuous_value(&plans, arena, *source, param.source_port),
                PortKind::Event => None,
            };
            edges.push(EdgeView { from, to, kind: EdgeKind::Param, activity });
        } else if entity_ref.get::<FeedsEdge>().is_some() {
            edges.push(EdgeView { from, to, kind: EdgeKind::Feeds, activity: None });
        }
        // `ParentEdge` is intentionally skipped -- see `EdgeKind`.
    }
    edges
}

/// The source node's output port slot, downcast to `f32`.
///
/// The arena slot for a port ordinal is `continuous_base + ordinal`; the
/// compiler uses exactly this arithmetic when it builds `continuous_copies`.
fn continuous_value(
    plans: &HashMap<Entity, &NodePlan>,
    arena: Option<&PortArena>,
    source: Entity,
    source_port: u16,
) -> Option<f32> {
    let slot = plans.get(&source)?.continuous_base + source_port as usize;
    arena?
        .continuous
        .get(slot)?
        .try_downcast_ref::<f32>()
        .copied()
}

fn group_of(world: &World, entity: Entity) -> TreeGroup {
    if world.get::<Transform>(entity).is_some() {
        TreeGroup::Scene
    } else if world.get::<GraphNode>(entity).is_some() {
        TreeGroup::Graph
    } else if world.get::<ParamEdge>(entity).is_some()
        || world.get::<FeedsEdge>(entity).is_some()
        || world.get::<ParentEdge>(entity).is_some()
    {
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
        Emit, Recv, app, connect, recompile, spawn_emit, spawn_named_spatial, spawn_recv,
        spawn_spatial,
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
    fn a_param_edge_indexes_into_the_node_list() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.edges.len(), 1);
        let from = &snap.nodes[snap.edges[0].from];
        let to = &snap.nodes[snap.edges[0].to];
        assert_eq!(from.entity, emit);
        assert_eq!(to.entity, recv);
        assert_eq!(snap.edges[0].kind, EdgeKind::Param);
    }

    #[test]
    fn a_continuous_f32_edge_reports_the_source_ports_live_value() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        // One tick, so `Emit::tick` has actually written its output port.
        app.update();

        assert_eq!(capture(app.world()).edges[0].activity, Some(0.75));
    }

    #[test]
    fn capture_before_compilation_yields_nodes_but_no_activity() {
        // A graph that has not been compiled has no `CompiledGraph` and an
        // empty arena. The editor must still draw it rather than panic.
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);

        let snap = capture(app.world());

        assert_eq!(snap.nodes.len(), 2);
        assert_eq!(snap.edges.len(), 1);
        assert_eq!(snap.edges[0].activity, None);
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
