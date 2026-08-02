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
use bevy_ecs::world::World;
use kurbo::Point;
use sway_graph::{
    CompiledGraph, EdgeFrom, EdgeTo, EditorPos, FeedsEdge, GraphNode, NodeId, NodePlan,
    NodeTypeRegistry, ParamEdge, PortArena, PortKind,
};

/// Which kind of edge this is. `ParentEdge` is deliberately absent: the tree
/// pane shows parenting already, and drawing it twice makes the canvas harder
/// to read for no gain (design §9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

/// Placeholder until Task 3. Rows of the world hierarchy pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub entity: Entity,
    pub depth: usize,
    pub label: String,
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
    WorldSnapshot { tree: Vec::new(), nodes, edges }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_graph::{Emit, Recv, app, connect, recompile, spawn_emit, spawn_recv};
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
}
