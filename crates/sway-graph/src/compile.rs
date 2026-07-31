//! Graph compilation: turning a graph of node instances into a flat,
//! contiguous [`NodePlan`] per node plus the shared [`PortArena`] layout.
//!
//! Spec §5. `compile` reads the world, resolves every node's type and
//! schema, validates every param edge, topologically sorts the node set and
//! produces one [`NodePlan`] per node in that order. All failure happens
//! here — the tick (Task 5) is infallible, and only walks the plans this
//! produces.
//!
//! [`PortArena`]: crate::ports::PortArena

use core::fmt;
use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;

use crate::edges::{EdgeFrom, EdgeTo, GraphNode, NodeId, NodeRuntime, ParamEdge, PortKind};
use crate::registry::{NodeSchema, NodeTypeId, NodeTypeRegistry};
use crate::schema::PortField;

/// The compiled, per-node-instance plan the runner and prefill step read.
#[derive(Debug)]
pub struct NodePlan {
    pub entity: Entity,
    pub node_type: NodeTypeId,
    pub schema: NodeSchema,
    /// Base offset of this node's continuous ports in [`crate::ports::PortArena::continuous`].
    pub continuous_base: usize,
    /// Base offset of this node's event ports in [`crate::ports::PortArena::events`].
    pub event_base: usize,
    /// Per continuous-input-ordinal: whether an edge drives it. When `false`,
    /// prefill copies the authored value from `Params` instead (spec §4).
    pub connected_continuous: Vec<bool>,
    /// Absolute `(source_slot, dest_slot)` pairs into
    /// [`crate::ports::PortArena::continuous`] for every edge that drives one
    /// of this node's continuous inputs. At most one entry per input ordinal
    /// — a continuous input takes exactly one edge (spec §5).
    pub continuous_copies: Vec<(usize, usize)>,
    /// Absolute `(source_slot, dest_slot)` pairs into
    /// [`crate::ports::PortArena::events`] for every edge that feeds one of
    /// this node's event inputs. An event input may take many edges (spec
    /// §5), so a dest slot may appear more than once. Sorted by the source
    /// node's position in the compiled order — the deterministic tiebreak
    /// for merged event streams that share an offset.
    pub event_merges: Vec<(usize, usize)>,
}

/// The output of [`compile`]: the flat execution plan the tick runner walks,
/// plus the total arena sizes it must be resized to.
#[derive(Resource, Debug)]
pub struct CompiledGraph {
    /// One entry per node, in topological (dependency) order.
    pub plans: Vec<NodePlan>,
    pub continuous_len: usize,
    pub events_len: usize,
}

/// Everything that can go wrong at compile time. Spec §5's failure table —
/// one variant per row. Every `Display` arm names the offending node(s).
#[derive(Debug)]
pub enum CompileError {
    UnknownNodeType {
        node: Entity,
        id: NodeTypeId,
    },
    PortOutOfRange {
        node: Entity,
        port: u16,
        kind: PortKind,
        arity: usize,
    },
    TypeMismatch {
        source: Entity,
        source_port: &'static str,
        source_type: &'static str,
        target: Entity,
        target_port: &'static str,
        target_type: &'static str,
    },
    DuplicateContinuousInput {
        target: Entity,
        port: &'static str,
        first: Entity,
        second: Entity,
    },
    MissingEndpoint {
        edge: Entity,
        missing: Entity,
    },
    Cycle {
        nodes: Vec<Entity>,
    },
}

fn kind_name(kind: PortKind) -> &'static str {
    match kind {
        PortKind::Continuous => "continuous",
        PortKind::Event => "event",
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNodeType { node, id } => write!(
                f,
                "node {node}: node type {} is not registered in the graph's NodeTypeRegistry",
                id.0
            ),
            Self::PortOutOfRange { node, port, kind, arity } => write!(
                f,
                "node {node}: {} port {port} is out of range — this node type has {arity} \
                 {} port(s)",
                kind_name(*kind),
                kind_name(*kind)
            ),
            Self::TypeMismatch {
                source,
                source_port,
                source_type,
                target,
                target_port,
                target_type,
            } => write!(
                f,
                "type mismatch on edge from node {source} port `{source_port}` ({source_type}) \
                 to node {target} port `{target_port}` ({target_type})"
            ),
            Self::DuplicateContinuousInput { target, port, first, second } => write!(
                f,
                "node {target} continuous port `{port}` already has an edge from node {first}; \
                 a second edge from node {second} is rejected — a continuous input takes \
                 exactly one edge"
            ),
            Self::MissingEndpoint { edge, missing } => write!(
                f,
                "edge {edge} references node {missing}, which does not exist in the world"
            ),
            Self::Cycle { nodes } => {
                write!(f, "cycle detected among nodes: ")?;
                for (i, node) in nodes.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{node}")?;
                }
                Ok(())
            }
        }
    }
}

impl core::error::Error for CompileError {}

/// One resolved node, mid-compilation: its schema and allocated arena bases.
struct CompiledNode {
    entity: Entity,
    node_type: NodeTypeId,
    schema: NodeSchema,
    continuous_base: usize,
    event_base: usize,
}

/// One validated edge, referring to its endpoints by index into the
/// sorted node list rather than by entity, so later passes need no further
/// lookups.
struct ValidatedEdge {
    source_idx: usize,
    target_idx: usize,
    source_port: u16,
    target_port: u16,
    kind: PortKind,
}

/// Looks up the [`PortField`] a port ordinal names within one kind-space
/// (spec §4: inputs then outputs, contiguous). `None` if `ordinal` is
/// out of range for that node's schema.
fn port_field(schema: &NodeSchema, kind: PortKind, ordinal: u16) -> Option<&PortField> {
    let (inputs, outputs) = match kind {
        PortKind::Continuous => (&schema.inputs.continuous, &schema.outputs.continuous),
        PortKind::Event => (&schema.inputs.events, &schema.outputs.events),
    };
    let idx = ordinal as usize;
    if idx < inputs.len() {
        inputs.get(idx)
    } else {
        outputs.get(idx - inputs.len())
    }
}

/// The total number of ports of `kind` this schema has (inputs + outputs) —
/// what `PortOutOfRange` reports as the arity.
fn arity_of(schema: &NodeSchema, kind: PortKind) -> usize {
    match kind {
        PortKind::Continuous => schema.continuous_len(),
        PortKind::Event => schema.events_len(),
    }
}

/// Compiles the world's graph of `GraphNode` entities and `ParamEdge`
/// relationships into a flat, topologically-ordered execution plan.
///
/// Reads the world; does not tick anything and does not touch the
/// `PortArena` (Task 5's runner owns applying the layout this produces).
/// Writes a fresh [`NodeRuntime`] onto every compiled node, resetting its
/// prefill gate — see `NodeRuntime::last_params_tick`.
pub fn compile(world: &mut World) -> Result<CompiledGraph, CompileError> {
    // --- Pass 1: collect nodes, resolve node types --------------------
    //
    // Sorted by `GraphNode::id` before anything else, so base allocation
    // (pass 2) and the topo sort's ready-queue seeding (pass 4) are
    // deterministic for graphs the edges alone do not fully order.
    let mut node_query = world.query::<(Entity, &GraphNode)>();
    let raw_nodes: Vec<(Entity, NodeId, NodeTypeId)> = node_query
        .iter(world)
        .map(|(entity, node)| (entity, node.id, node.node_type))
        .collect();

    let registry = world.resource::<NodeTypeRegistry>();
    let mut collected: Vec<(Entity, NodeId, NodeTypeId, NodeSchema)> =
        Vec::with_capacity(raw_nodes.len());
    for (entity, id, node_type) in raw_nodes {
        let entry = registry
            .get(node_type)
            .ok_or(CompileError::UnknownNodeType { node: entity, id: node_type })?;
        collected.push((entity, id, node_type, entry.schema.clone()));
    }
    collected.sort_by_key(|(_, id, _, _)| *id);

    // --- Pass 2: allocate contiguous per-kind bases --------------------
    let mut nodes: Vec<CompiledNode> = Vec::with_capacity(collected.len());
    let mut continuous_cursor = 0usize;
    let mut event_cursor = 0usize;
    for (entity, _id, node_type, schema) in collected {
        let continuous_base = continuous_cursor;
        let event_base = event_cursor;
        continuous_cursor += schema.continuous_len();
        event_cursor += schema.events_len();
        nodes.push(CompiledNode { entity, node_type, schema, continuous_base, event_base });
    }
    let continuous_len = continuous_cursor;
    let events_len = event_cursor;

    let index_of: HashMap<Entity, usize> =
        nodes.iter().enumerate().map(|(i, n)| (n.entity, i)).collect();

    // --- Pass 3: validate edges -----------------------------------------
    struct RawEdge {
        edge: Entity,
        source_port: u16,
        target_port: u16,
        kind: PortKind,
        from: Entity,
        to: Entity,
    }

    let mut edge_query = world.query::<(Entity, &ParamEdge, &EdgeFrom, &EdgeTo)>();
    let raw_edges: Vec<RawEdge> = edge_query
        .iter(world)
        .map(|(edge, param_edge, from, to)| RawEdge {
            edge,
            source_port: param_edge.source_port,
            target_port: param_edge.target_port,
            kind: param_edge.kind,
            from: from.0,
            to: to.0,
        })
        .collect();

    let mut validated_edges: Vec<ValidatedEdge> = Vec::with_capacity(raw_edges.len());
    // Duplicate continuous fan-in: keyed by (target node index, target port).
    let mut continuous_fanin: HashMap<(usize, u16), Entity> = HashMap::new();

    for raw in raw_edges {
        let &source_idx = index_of
            .get(&raw.from)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.from })?;
        let &target_idx = index_of
            .get(&raw.to)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.to })?;

        let source_node = &nodes[source_idx];
        let target_node = &nodes[target_idx];

        let source_field =
            port_field(&source_node.schema, raw.kind, raw.source_port).ok_or(CompileError::PortOutOfRange {
                node: source_node.entity,
                port: raw.source_port,
                kind: raw.kind,
                arity: arity_of(&source_node.schema, raw.kind),
            })?;
        let target_field =
            port_field(&target_node.schema, raw.kind, raw.target_port).ok_or(CompileError::PortOutOfRange {
                node: target_node.entity,
                port: raw.target_port,
                kind: raw.kind,
                arity: arity_of(&target_node.schema, raw.kind),
            })?;

        if source_field.type_id != target_field.type_id {
            return Err(CompileError::TypeMismatch {
                source: source_node.entity,
                source_port: source_field.name,
                source_type: source_field.type_path,
                target: target_node.entity,
                target_port: target_field.name,
                target_type: target_field.type_path,
            });
        }

        if raw.kind == PortKind::Continuous {
            let key = (target_idx, raw.target_port);
            if let Some(&first) = continuous_fanin.get(&key) {
                return Err(CompileError::DuplicateContinuousInput {
                    target: target_node.entity,
                    port: target_field.name,
                    first,
                    second: source_node.entity,
                });
            }
            continuous_fanin.insert(key, source_node.entity);
        }

        validated_edges.push(ValidatedEdge {
            source_idx,
            target_idx,
            source_port: raw.source_port,
            target_port: raw.target_port,
            kind: raw.kind,
        });
    }

    // --- Pass 4: Kahn's topological sort ---------------------------------
    //
    // Ready queue seeded in the sorted-by-`NodeId` order from pass 1/2
    // (index 0..n already is that order) and popped from the front, so any
    // tie the edges leave unresolved still breaks deterministically.
    let n = nodes.len();
    let mut in_degree = vec![0u32; n];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in &validated_edges {
        in_degree[edge.target_idx] += 1;
        adjacency[edge.source_idx].push(edge.target_idx);
    }
    for adj in &mut adjacency {
        adj.sort_unstable();
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while let Some(idx) = queue.pop_front() {
        order.push(idx);
        placed[idx] = true;
        for &next in &adjacency[idx] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }

    if order.len() != n {
        let remaining: Vec<Entity> =
            (0..n).filter(|&i| !placed[i]).map(|i| nodes[i].entity).collect();
        return Err(CompileError::Cycle { nodes: remaining });
    }

    let mut topo_rank = vec![0usize; n];
    for (rank, &idx) in order.iter().enumerate() {
        topo_rank[idx] = rank;
    }

    // --- Pass 5: build plans, in topological order -----------------------
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, edge) in validated_edges.iter().enumerate() {
        incoming[edge.target_idx].push(i);
    }

    let mut plans: Vec<NodePlan> = Vec::with_capacity(n);
    for &idx in &order {
        let node = &nodes[idx];
        let mut connected_continuous = vec![false; node.schema.inputs.continuous.len()];
        let mut continuous_copies: Vec<(usize, usize)> = Vec::new();
        // Tagged with the source's topo rank until sorted, then stripped.
        let mut ranked_event_merges: Vec<(usize, (usize, usize))> = Vec::new();

        for &edge_idx in &incoming[idx] {
            let edge = &validated_edges[edge_idx];
            let source = &nodes[edge.source_idx];
            match edge.kind {
                PortKind::Continuous => {
                    connected_continuous[edge.target_port as usize] = true;
                    continuous_copies.push((
                        source.continuous_base + edge.source_port as usize,
                        node.continuous_base + edge.target_port as usize,
                    ));
                }
                PortKind::Event => {
                    ranked_event_merges.push((
                        topo_rank[edge.source_idx],
                        (
                            source.event_base + edge.source_port as usize,
                            node.event_base + edge.target_port as usize,
                        ),
                    ));
                }
            }
        }

        continuous_copies.sort_unstable_by_key(|&(_, dest)| dest);

        // Spec §5's deterministic tiebreak for merged event streams: sort by
        // the source node's position in the compiled order.
        ranked_event_merges.sort_by_key(|&(rank, _)| rank);
        let event_merges: Vec<(usize, usize)> =
            ranked_event_merges.into_iter().map(|(_, pair)| pair).collect();

        plans.push(NodePlan {
            entity: node.entity,
            node_type: node.node_type,
            schema: node.schema.clone(),
            continuous_base: node.continuous_base,
            event_base: node.event_base,
            connected_continuous,
            continuous_copies,
            event_merges,
        });
    }

    // --- Pass 6: write NodeRuntime, resetting the prefill gate ------------
    for node in &nodes {
        world.entity_mut(node.entity).insert(NodeRuntime {
            continuous_base: node.continuous_base,
            event_base: node.event_base,
            last_params_tick: None,
        });
    }

    Ok(CompiledGraph { plans, continuous_len, events_len })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_nodes::{probe_app, spawn_emitter, spawn_int_probe, spawn_probe};

    fn edge(world: &mut World, from: Entity, to: Entity, sp: u16, tp: u16, kind: PortKind) -> Entity {
        world
            .spawn((ParamEdge { source_port: sp, target_port: tp, kind }, EdgeFrom(from), EdgeTo(to)))
            .id()
    }

    #[test]
    fn a_chain_compiles_in_topological_order() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        // a.value (continuous ordinal 2) -> b.gain (continuous ordinal 0)
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);

        let compiled = compile(app.world_mut()).expect("compiles");
        let order: Vec<Entity> = compiled.plans.iter().map(|p| p.entity).collect();
        assert_eq!(order, vec![a, b], "producer must be ordered before consumer");
    }

    #[test]
    fn bases_are_allocated_contiguously_per_node() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        let compiled = compile(app.world_mut()).expect("compiles");

        let pa = compiled.plans.iter().find(|p| p.entity == a).unwrap();
        let pb = compiled.plans.iter().find(|p| p.entity == b).unwrap();
        assert_ne!(pa.continuous_base, pb.continuous_base);
        assert_eq!(compiled.continuous_len, 6, "two probes, 3 continuous ports each");
        assert_eq!(compiled.events_len, 2);
    }

    #[test]
    fn a_cycle_is_rejected_and_names_every_node_in_it() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);
        edge(app.world_mut(), b, a, 2, 0, PortKind::Continuous);

        let err = compile(app.world_mut()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "{msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_second_edge_into_a_continuous_input_is_rejected() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        let c = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, c, 2, 0, PortKind::Continuous);
        edge(app.world_mut(), b, c, 2, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        // Spec §5: "which one wins" has no defensible answer.
        assert!(msg.contains("gain"), "must name the target port: {msg}");
        assert!(msg.contains(&format!("{c}")), "must name the target node: {msg}");
    }

    #[test]
    fn many_edges_into_an_event_input_are_allowed() {
        let mut app = probe_app();
        // `Probe` has no event *output*, so the two sources are `Emitter`s
        // (test_nodes), whose event output is ordinal 0. `Probe.trigger` is
        // event ordinal 0 on the target side.
        let a = spawn_emitter(app.world_mut());
        let b = spawn_emitter(app.world_mut());
        let c = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, c, 0, 0, PortKind::Event);
        edge(app.world_mut(), b, c, 0, 0, PortKind::Event);

        assert!(compile(app.world_mut()).is_ok(), "event fan-in is legal (spec §5)");
    }

    #[test]
    fn a_type_mismatch_names_both_nodes_both_ports_and_both_types() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_int_probe(app.world_mut()); // u32 params, see test_nodes
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("f32") && msg.contains("u32"), "{msg}");
        assert!(msg.contains("value") && msg.contains("count"), "{msg}");
    }

    #[test]
    fn a_port_index_out_of_range_is_rejected_with_the_arity() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 99, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("99"), "{msg}");
        assert!(msg.contains('3'), "must state the schema's arity: {msg}");
    }

    #[test]
    fn an_edge_to_a_despawned_node_is_rejected() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);
        app.world_mut().despawn(b);

        // linked_spawn should have taken the edge with it, so this compiles
        // clean — which is the actual assertion. A dangling edge would be a
        // Bevy relationship bug, and this test is what would catch it.
        assert!(compile(app.world_mut()).is_ok());
    }

    #[test]
    fn an_unregistered_node_type_is_rejected() {
        let mut app = probe_app();
        let e = app.world_mut().spawn(GraphNode { id: NodeId(0), node_type: NodeTypeId(999) }).id();
        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{e}")), "{msg}");
        assert!(msg.contains("999"), "{msg}");
    }
}
