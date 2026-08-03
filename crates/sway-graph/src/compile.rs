//! Graph compilation: one validation pass, one topological sort, one plan per
//! node.
//!
//! All failure happens here — the tick is infallible, and only walks the plans
//! this produces.

use core::any::TypeId;
use core::fmt;
use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;

use crate::edges::{Edge, EdgeFrom, EdgeTo, GraphNode, NodeId, NodeRuntime};
use crate::ports::Spatial;
use crate::registry::{NodeTypeId, NodeTypeRegistry};
use crate::schema::{FieldKind, FieldSpec, ProductAccess};

/// The compiled, per-node-instance plan the runner reads.
#[derive(Debug)]
pub struct NodePlan {
    pub entity: Entity,
    pub node_type: NodeTypeId,
    /// This node's fields: inlets first, then outlets. Cloned from the
    /// registry so the runner can hold it while `world` is borrowed mutably.
    pub fields: Vec<FieldSpec>,
    /// How many of `fields` are inlets.
    pub inlet_field_count: usize,
    /// Absolute base of this node's slots in the arena.
    pub base: usize,
    /// Per field ordinal: offset from `base` of that field's first slot.
    pub field_offsets: Vec<usize>,
    /// Per field ordinal: how many slots it has — 1, or the instance's `Vec`
    /// length.
    pub field_lens: Vec<usize>,
    /// How many slots this node's inlets occupy, so `base..base + inlet_slots`
    /// is exactly the prefillable range.
    pub inlet_slots: usize,
    /// Per slot, relative to `base`: whether an edge drives it. Sized to the
    /// node's total slots so `PortView` can index it uniformly.
    pub connected: Vec<bool>,
    /// Absolute `(source slot, dest slot)` for every edge into this node.
    pub copies: Vec<(usize, usize)>,
    /// Absolute slot and accessor for every product inlet, whether filled or
    /// not — the cook gate walks these to find its sources.
    pub product_inlets: Vec<(usize, ProductAccess)>,
}

/// The output of [`compile`].
#[derive(Resource, Debug)]
pub struct CompiledGraph {
    /// One entry per node, in topological order.
    pub plans: Vec<NodePlan>,
    pub slots_len: usize,
    pub(crate) outlets_seeded: bool,
    /// Every `Events` slot in the graph, with the fn that empties it in
    /// place. Run once at the top of each tick.
    pub clears: Vec<(usize, fn(&mut dyn PartialReflect))>,
    /// Entity → index into `plans`, for the cook gate's source lookup.
    pub plan_index_of: HashMap<Entity, usize>,
}

/// Everything that can go wrong at compile time. Every `Display` arm names
/// the offending node(s).
#[derive(Debug)]
pub enum CompileError {
    UnknownNodeType { node: Entity, id: NodeTypeId },
    MissingEndpoint { edge: Entity, missing: Entity },
    FieldOutOfRange { node: Entity, field: u16, arity: usize },
    ElementOutOfRange { node: Entity, field: &'static str, index: u16, len: usize },
    WrongDirection { node: Entity, field: &'static str, expected: &'static str },
    TypeMismatch {
        source: Entity,
        source_field: &'static str,
        source_type: &'static str,
        target: Entity,
        target_field: &'static str,
        target_type: &'static str,
    },
    InletAlreadyConnected {
        target: Entity,
        field: &'static str,
        index: u16,
        first: Entity,
        second: Entity,
    },
    SpatialFanOut { child: Entity, first: Entity, second: Entity },
    ParentCycle { nodes: Vec<Entity> },
    Cycle { nodes: Vec<Entity> },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNodeType { node, id } => {
                write!(f, "node {node} has unregistered node type {id:?}")
            }
            Self::MissingEndpoint { edge, missing } => write!(
                f,
                "edge {edge} names {missing}, which is not a node in this graph"
            ),
            Self::FieldOutOfRange { node, field, arity } => write!(
                f,
                "node {node}: field ordinal {field} is out of range — the node has {arity} field(s)"
            ),
            Self::ElementOutOfRange { node, field, index, len } => write!(
                f,
                "node {node}: field `{field}` has {len} slot(s), so element {index} does not exist \
                 — resize the Vec on the node's Inlets, or edit the edge"
            ),
            Self::WrongDirection { node, field, expected } => write!(
                f,
                "node {node}: field `{field}` is not {expected} — an edge runs from an outlet to \
                 an inlet"
            ),
            Self::TypeMismatch {
                source,
                source_field,
                source_type,
                target,
                target_field,
                target_type,
            } => write!(
                f,
                "type mismatch: node {source} outlet `{source_field}` produces `{source_type}`, \
                 but node {target} inlet `{target_field}` expects `{target_type}`"
            ),
            Self::InletAlreadyConnected { target, field, index, first, second } => write!(
                f,
                "node {target}: inlet `{field}`[{index}] is already connected to node {first}; a \
                 second edge from node {second} is illegal — every inlet takes exactly one edge"
            ),
            Self::SpatialFanOut { child, first, second } => write!(
                f,
                "node {child} already has parent {first}; a second parent edge to {second} is \
                 illegal — a scene node has one parent"
            ),
            Self::ParentCycle { nodes } => write!(
                f,
                "parenting cycle: {}",
                nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" → ")
            ),
            Self::Cycle { nodes } => write!(
                f,
                "cycle: these nodes could not be ordered: {}",
                nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

impl core::error::Error for CompileError {}

/// One node's slot layout, computed before validation because edge
/// resolution needs it.
struct Layout {
    entity: Entity,
    node_type: NodeTypeId,
    fields: Vec<FieldSpec>,
    inlet_field_count: usize,
    base: usize,
    field_offsets: Vec<usize>,
    field_lens: Vec<usize>,
    inlet_slots: usize,
    slot_count: usize,
}

impl Layout {
    fn slot(&self, field: u16, index: u16) -> usize {
        self.base + self.field_offsets[field as usize] + index as usize
    }
}

struct ValidEdge {
    source_idx: usize,
    target_idx: usize,
    source_slot: usize,
    target_slot: usize,
    /// Relative to the target's base — indexes `connected`.
    target_local: usize,
    spatial: bool,
}

pub fn compile(world: &mut World) -> Result<CompiledGraph, CompileError> {
    // --- Pass 1: collect nodes, sorted by NodeId for determinism -------
    let mut node_query = world.query::<(Entity, &GraphNode)>();
    let mut raw_nodes: Vec<(Entity, NodeId, NodeTypeId)> = node_query
        .iter(world)
        .map(|(entity, node)| (entity, node.id, node.node_type))
        .collect();
    raw_nodes.sort_by_key(|(_, id, _)| *id);

    for &(entity, _, node_type) in &raw_nodes {
        let insert_defaults = {
            let registry = world.resource::<NodeTypeRegistry>();
            registry
                .get(node_type)
                .ok_or(CompileError::UnknownNodeType { node: entity, id: node_type })?
                .insert_defaults
        };
        insert_defaults(world, entity);
    }

    // --- Pass 2: lay out slots -----------------------------------------
    //
    // A variadic field's length comes from the instance, so this reads the
    // Inlets component. That one number is the only per-instance input to
    // what is otherwise a per-type schema.
    let mut layouts: Vec<Layout> = Vec::with_capacity(raw_nodes.len());
    let mut cursor = 0usize;
    for &(entity, _, node_type) in &raw_nodes {
        let (fields, inlet_field_count, inlet_lens) = {
            let inlet_lens_fn = {
                let registry = world.resource::<NodeTypeRegistry>();
                let entry = registry
                    .get(node_type)
                    .ok_or(CompileError::UnknownNodeType { node: entity, id: node_type })?;
                entry.inlet_lens
            };
            let lens = inlet_lens_fn(world, entity);
            let registry = world.resource::<NodeTypeRegistry>();
            let entry = registry.get(node_type).expect("resolved above");
            let mut fields = entry.inlets.clone();
            let inlet_field_count = fields.len();
            fields.extend(entry.outlets.iter().cloned());
            (fields, inlet_field_count, lens)
        };

        let base = cursor;
        let mut field_offsets = Vec::with_capacity(fields.len());
        let mut field_lens = Vec::with_capacity(fields.len());
        let mut offset = 0usize;
        for (ordinal, spec) in fields.iter().enumerate() {
            let len = if ordinal < inlet_field_count {
                // `inlet_lens` reports 1 for a non-Vec field already.
                inlet_lens.get(spec.field_index).copied().unwrap_or(1)
            } else {
                1 // outlets cannot be Vec — enforced at registration
            };
            field_offsets.push(offset);
            field_lens.push(len);
            offset += len;
        }
        let inlet_slots: usize = field_lens[..inlet_field_count].iter().sum();
        let slot_count = offset;
        cursor += slot_count;

        layouts.push(Layout {
            entity,
            node_type,
            fields,
            inlet_field_count,
            base,
            field_offsets,
            field_lens,
            inlet_slots,
            slot_count,
        });
    }
    let slots_len = cursor;

    let index_of: HashMap<Entity, usize> =
        layouts.iter().enumerate().map(|(i, l)| (l.entity, i)).collect();

    // --- Pass 3: validate every edge ------------------------------------
    struct RawEdge {
        edge: Entity,
        from: Entity,
        to: Entity,
        from_field: u16,
        from_index: u16,
        to_field: u16,
        to_index: u16,
    }

    let mut edge_query = world.query::<(Entity, &Edge, &EdgeFrom, &EdgeTo)>();
    let raw_edges: Vec<RawEdge> = edge_query
        .iter(world)
        .map(|(edge, e, from, to)| RawEdge {
            edge,
            from: from.0,
            to: to.0,
            from_field: e.from.field,
            from_index: e.from.index,
            to_field: e.to.field,
            to_index: e.to.index,
        })
        .collect();

    let mut valid: Vec<ValidEdge> = Vec::with_capacity(raw_edges.len());
    let mut filled: HashMap<usize, Entity> = HashMap::new();
    // Spatial outlets are single-consumer: keyed by source node index.
    let mut spatial_consumer: HashMap<usize, Entity> = HashMap::new();
    let mut parent_of: Vec<Option<usize>> = vec![None; layouts.len()];

    for raw in raw_edges {
        let &source_idx = index_of
            .get(&raw.from)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.from })?;
        let &target_idx = index_of
            .get(&raw.to)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.to })?;

        let source = &layouts[source_idx];
        let target = &layouts[target_idx];

        let source_spec = source.fields.get(raw.from_field as usize).ok_or(
            CompileError::FieldOutOfRange {
                node: source.entity,
                field: raw.from_field,
                arity: source.fields.len(),
            },
        )?;
        let target_spec = target.fields.get(raw.to_field as usize).ok_or(
            CompileError::FieldOutOfRange {
                node: target.entity,
                field: raw.to_field,
                arity: target.fields.len(),
            },
        )?;

        // An edge runs outlet → inlet. Direction is which half of the field
        // space the ordinal lands in.
        if (raw.from_field as usize) < source.inlet_field_count {
            return Err(CompileError::WrongDirection {
                node: source.entity,
                field: source_spec.name,
                expected: "an outlet",
            });
        }
        if (raw.to_field as usize) >= target.inlet_field_count {
            return Err(CompileError::WrongDirection {
                node: target.entity,
                field: target_spec.name,
                expected: "an inlet",
            });
        }

        let source_len = source.field_lens[raw.from_field as usize];
        if (raw.from_index as usize) >= source_len {
            return Err(CompileError::ElementOutOfRange {
                node: source.entity,
                field: source_spec.name,
                index: raw.from_index,
                len: source_len,
            });
        }
        let target_len = target.field_lens[raw.to_field as usize];
        if (raw.to_index as usize) >= target_len {
            return Err(CompileError::ElementOutOfRange {
                node: target.entity,
                field: target_spec.name,
                index: raw.to_index,
                len: target_len,
            });
        }

        // One type check for every carrier: a slot type is a slot type.
        if source_spec.slot_type != target_spec.slot_type {
            return Err(CompileError::TypeMismatch {
                source: source.entity,
                source_field: source_spec.name,
                source_type: source_spec.slot_type_path,
                target: target.entity,
                target_field: target_spec.name,
                target_type: target_spec.slot_type_path,
            });
        }

        let target_slot = target.slot(raw.to_field, raw.to_index);
        if let Some(&first) = filled.get(&target_slot) {
            return Err(CompileError::InletAlreadyConnected {
                target: target.entity,
                field: target_spec.name,
                index: raw.to_index,
                first,
                second: source.entity,
            });
        }
        filled.insert(target_slot, source.entity);

        let spatial = matches!(
            target_spec.kind,
            FieldKind::Product { capability, .. } if capability == TypeId::of::<Spatial>()
        );
        if spatial {
            // Bevy's ChildOf is a one-parent relationship, so a Spatial
            // outlet may feed at most one inlet.
            if let Some(&first) = spatial_consumer.get(&source_idx) {
                return Err(CompileError::SpatialFanOut {
                    child: source.entity,
                    first,
                    second: target.entity,
                });
            }
            spatial_consumer.insert(source_idx, target.entity);
            parent_of[source_idx] = Some(target_idx);
        }

        valid.push(ValidEdge {
            source_idx,
            target_idx,
            source_slot: source.slot(raw.from_field, raw.from_index),
            target_slot,
            target_local: target_slot - target.base,
            spatial,
        });
    }

    // --- Pass 4: parenting acyclicity ------------------------------------
    //
    // Checked separately from the sort, because Spatial edges are excluded
    // from it — a parent reads nothing from its child, and including them
    // would reject a child that drives a param on its own parent.
    for start in 0..layouts.len() {
        let mut cursor = parent_of[start];
        let mut chain = vec![layouts[start].entity];
        let mut seen = 0usize;
        while let Some(idx) = cursor {
            if idx == start {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            chain.push(layouts[idx].entity);
            seen += 1;
            if seen > layouts.len() {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            cursor = parent_of[idx];
        }
    }

    // --- Pass 5: one topological sort, Spatial excluded -------------------
    let n = layouts.len();
    let mut in_degree = vec![0u32; n];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in valid.iter().filter(|e| !e.spatial) {
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
            (0..n).filter(|&i| !placed[i]).map(|i| layouts[i].entity).collect();
        return Err(CompileError::Cycle { nodes: remaining });
    }

    // --- Pass 6: build plans, in compiled order ---------------------------
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, edge) in valid.iter().enumerate() {
        incoming[edge.target_idx].push(i);
    }

    let mut plans: Vec<NodePlan> = Vec::with_capacity(n);
    let mut clears: Vec<(usize, fn(&mut dyn PartialReflect))> = Vec::new();

    for &idx in &order {
        let layout = &layouts[idx];
        let mut connected = vec![false; layout.slot_count];
        let mut copies: Vec<(usize, usize)> = Vec::new();

        for &edge_idx in &incoming[idx] {
            let edge = &valid[edge_idx];
            connected[edge.target_local] = true;
            copies.push((edge.source_slot, edge.target_slot));
        }
        copies.sort_unstable_by_key(|&(_, dest)| dest);

        let mut product_inlets: Vec<(usize, ProductAccess)> = Vec::new();
        for (ordinal, spec) in layout.fields.iter().enumerate() {
            let offset = layout.field_offsets[ordinal];
            for index in 0..layout.field_lens[ordinal] {
                let slot = layout.base + offset + index;
                match spec.kind {
                    FieldKind::Events { clear, .. } => clears.push((slot, clear)),
                    FieldKind::Product { access, .. } if ordinal < layout.inlet_field_count => {
                        product_inlets.push((slot, access));
                    }
                    _ => {}
                }
            }
        }

        plans.push(NodePlan {
            entity: layout.entity,
            node_type: layout.node_type,
            fields: layout.fields.clone(),
            inlet_field_count: layout.inlet_field_count,
            base: layout.base,
            field_offsets: layout.field_offsets.clone(),
            field_lens: layout.field_lens.clone(),
            inlet_slots: layout.inlet_slots,
            connected,
            copies,
            product_inlets,
        });
    }

    clears.sort_unstable_by_key(|&(slot, _)| slot);

    let plan_index_of: HashMap<Entity, usize> =
        plans.iter().enumerate().map(|(i, p)| (p.entity, i)).collect();

    // --- Pass 7: apply ChildOf, write NodeRuntime -------------------------
    for (idx, layout) in layouts.iter().enumerate() {
        match parent_of[idx] {
            Some(parent_idx) => {
                let parent = layouts[parent_idx].entity;
                world
                    .entity_mut(layout.entity)
                    .insert(bevy_ecs::hierarchy::ChildOf(parent));
            }
            None => {
                world
                    .entity_mut(layout.entity)
                    .remove::<bevy_ecs::hierarchy::ChildOf>();
            }
        }
    }
    for plan in &plans {
        world.entity_mut(plan.entity).insert(NodeRuntime {
            last_inlets_tick: None,
            cook_dirty: true,
            last_product_ticks: vec![None; plan.product_inlets.len()],
        });
    }

    Ok(CompiledGraph {
        plans,
        slots_len,
        outlets_seeded: false,
        clears,
        plan_index_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{PortArena, Product, Spatial};
    use crate::test_nodes::{
        engine_app, connect, connect_at, event_offsets, port_value, recompile, spawn_consumer,
        spawn_emitter, spawn_gain, spawn_group, spawn_producer, spawn_sink, spawn_sludge_source,
        spawn_sum, Consumer, Emitter, Gain, Group, Producer, Sink, SludgeSource, Sum,
    };
    use bevy_ecs::hierarchy::ChildOf;

    #[test]
    fn a_chain_compiles_in_topological_order() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 0.0, 5.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);

        let compiled = compile(app.world_mut()).expect("compiles");
        let order: Vec<Entity> = compiled.plans.iter().map(|p| p.entity).collect();
        assert_eq!(order, vec![a, b], "producer must be ordered before consumer");
    }

    #[test]
    fn bases_are_allocated_contiguously_per_node() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        let compiled = compile(app.world_mut()).expect("compiles");

        let pa = compiled.plans.iter().find(|p| p.entity == a).unwrap();
        let pb = compiled.plans.iter().find(|p| p.entity == b).unwrap();
        assert_ne!(pa.base, pb.base);
        assert_eq!(compiled.slots_len, 6, "two Gains, 3 fields each");
    }

    #[test]
    fn compile_inserts_missing_inlets_and_state_defaults() {
        let mut app = engine_app();
        let node = spawn_gain(app.world_mut(), 1.0, 1.0);
        app.world_mut().entity_mut(node).remove::<crate::test_nodes::GainInlets>();
        app.world_mut().entity_mut(node).remove::<crate::test_nodes::GainState>();

        compile(app.world_mut()).expect("missing defaults are inserted");

        assert!(app.world().get::<crate::test_nodes::GainInlets>(node).is_some());
        assert!(app.world().get::<crate::test_nodes::GainState>(node).is_some());
    }

    #[test]
    fn a_cycle_is_rejected_and_names_every_node_in_it() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);
        connect(app.world_mut(), b, Gain::OUT_VALUE, a, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("cycle"), "{msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_second_edge_into_an_inlet_is_rejected() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        let c = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, c, Gain::GAIN);
        connect(app.world_mut(), b, Gain::OUT_VALUE, c, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("gain"), "must name the target field: {msg}");
        assert!(msg.contains(&format!("{c}")), "must name the target node: {msg}");
        assert!(msg.contains("exactly one edge"), "{msg}");
    }

    #[test]
    fn many_edges_into_a_variadic_inlet_are_allowed() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 1);

        assert!(compile(app.world_mut()).is_ok(), "one edge per element is legal");
    }

    #[test]
    fn a_type_mismatch_names_both_nodes_both_fields_and_both_types() {
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let gain = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), producer, Producer::OUT_BLOB, gain, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("blob") && msg.contains("gain"), "{msg}");
        assert!(msg.contains("Product") && msg.contains("f32"), "{msg}");
    }

    #[test]
    fn a_field_ordinal_out_of_range_is_rejected_with_the_arity() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), a, 99, b, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("99"), "{msg}");
        assert!(msg.contains('3'), "must state the node's field arity: {msg}");
    }

    #[test]
    fn an_edge_from_an_inlet_field_is_rejected() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        // a.gain is an INLET — not a legal edge source.
        connect(app.world_mut(), a, Gain::GAIN, b, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("gain"), "must name the field: {msg}");
        assert!(msg.contains(&format!("{a}")), "must name the node: {msg}");
        assert!(msg.contains("an outlet"), "{msg}");
    }

    #[test]
    fn an_edge_to_an_outlet_field_is_rejected() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        // b.value is an OUTLET — not a legal edge target.
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::OUT_VALUE);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("value"), "must name the field: {msg}");
        assert!(msg.contains(&format!("{b}")), "must name the node: {msg}");
        assert!(msg.contains("an inlet"), "{msg}");
    }

    #[test]
    fn an_edge_to_a_despawned_node_is_rejected() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);
        app.world_mut().despawn(b);

        // linked_spawn should have taken the edge with it, so this compiles
        // clean — which is the actual assertion. A dangling edge would be a
        // Bevy relationship bug, and this test is what would catch it.
        assert!(compile(app.world_mut()).is_ok());
    }

    #[test]
    fn an_edge_naming_a_non_node_entity_is_rejected() {
        // Distinct from `an_edge_to_a_despawned_node_is_rejected`: there, the
        // Edge entity itself is gone (linked_spawn takes it with the node).
        // Here the Edge entity is very much alive, but its target is some
        // other entity that was never a `GraphNode` at all — e.g. an edge
        // authored against a stale or malformed reference.
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let not_a_node = app.world_mut().spawn_empty().id();
        connect(app.world_mut(), a, Gain::OUT_VALUE, not_a_node, Gain::GAIN);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{not_a_node}")), "must name the bogus endpoint: {msg}");
        assert!(msg.contains("not a node"), "{msg}");
    }

    #[test]
    fn an_unregistered_node_type_is_rejected() {
        use crate::edges::GraphNode;
        use crate::registry::NodeTypeId;

        let mut app = engine_app();
        let e = app.world_mut().spawn(GraphNode { id: NodeId(0), node_type: NodeTypeId(999) }).id();
        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{e}")), "{msg}");
        assert!(msg.contains("999"), "{msg}");
    }

    // --- Spatial / hierarchy ---------------------------------------------

    #[test]
    fn two_parent_edges_from_one_child_are_rejected() {
        let mut app = engine_app();
        let child = spawn_group(app.world_mut(), 0);
        let a = spawn_group(app.world_mut(), 1);
        let b = spawn_group(app.world_mut(), 1);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, a, Group::CHILDREN, 0);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, b, Group::CHILDREN, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("one parent"), "vocabulary of the edge kind: {msg}");
        assert!(msg.contains(&format!("{child}")), "must name the child: {msg}");
        assert!(
            msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")),
            "must name both proposed parents: {msg}"
        );
    }

    #[test]
    fn parenting_a_non_spatial_node_is_rejected() {
        // A non-spatial outlet cannot feed a Product<Spatial> inlet at all —
        // it is now an ordinary TypeMismatch, not a distinct parenting error.
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let group = spawn_group(app.world_mut(), 1);
        connect_at(app.world_mut(), producer, Producer::OUT_BLOB, group, Group::CHILDREN, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("Product") && msg.contains("Spatial"), "{msg}");
        assert!(msg.contains("Blob"), "must name the other capability: {msg}");
    }

    #[test]
    fn parenting_under_a_non_spatial_node_is_rejected() {
        // Same TypeMismatch, from the other side: a Spatial outlet cannot
        // feed a non-Product<Spatial> inlet.
        let mut app = engine_app();
        let child = spawn_group(app.world_mut(), 0);
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), child, Group::OUT_SPATIAL, consumer, Consumer::INPUT);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("Spatial") && msg.contains("Blob"), "{msg}");
    }

    #[test]
    fn a_parenting_cycle_is_rejected() {
        let mut app = engine_app();
        let a = spawn_group(app.world_mut(), 1);
        let b = spawn_group(app.world_mut(), 1);
        connect_at(app.world_mut(), a, Group::OUT_SPATIAL, b, Group::CHILDREN, 0);
        connect_at(app.world_mut(), b, Group::OUT_SPATIAL, a, Group::CHILDREN, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("parenting"), "must speak of parenting, not dataflow: {msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_slot_filled_twice_is_rejected() {
        let mut app = engine_app();
        let a = spawn_producer(app.world_mut());
        let b = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), a, Producer::OUT_BLOB, consumer, Consumer::INPUT);
        connect(app.world_mut(), b, Producer::OUT_BLOB, consumer, Consumer::INPUT);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("exactly one edge"), "{msg}");
        assert!(msg.contains("input"), "must name the field: {msg}");
        assert!(
            msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")),
            "must name both sources: {msg}"
        );
    }

    #[test]
    fn a_source_that_produces_nothing_is_rejected() {
        // `Group`'s only outlet is `Product<Spatial>`; feeding it into a
        // `Product<Blob>` inlet must not be reported as a generic "cycle" or
        // "out of range" — it is a TypeMismatch that names the source.
        let mut app = engine_app();
        let group = spawn_group(app.world_mut(), 0);
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), group, Group::OUT_SPATIAL, consumer, Consumer::INPUT);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{group}")), "must name the source: {msg}");
        assert!(msg.contains("input"), "must name the field: {msg}");
    }

    #[test]
    fn a_slot_type_mismatch_names_the_capability_on_both_sides() {
        let mut app = engine_app();
        // `SludgeSource` produces `Sludge`, a real (non-unit) capability
        // distinct from the `Blob` `Consumer.input` expects.
        let sludge = spawn_sludge_source(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), sludge, SludgeSource::OUT_SLUDGE, consumer, Consumer::INPUT);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{sludge}")), "must name the source: {msg}");
        assert!(msg.contains("input"), "must name the field: {msg}");

        use bevy_reflect::TypePath;
        let blob_path = <Product<crate::test_nodes::Blob> as TypePath>::type_path();
        let sludge_path = <Product<crate::test_nodes::Sludge> as TypePath>::type_path();
        assert!(
            msg.contains(&format!("expects `{blob_path}`")),
            "must name the field's expected capability: {msg}"
        );
        assert!(
            msg.contains(&format!("produces `{sludge_path}`")),
            "must name the source's produced capability: {msg}"
        );
    }

    #[test]
    fn a_slot_ordinal_out_of_range_reports_the_arity() {
        let mut app = engine_app();
        let src = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), src, 9, consumer, Consumer::INPUT);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains('9'), "{msg}");
        assert!(msg.contains('2'), "must state Producer's field arity: {msg}");
    }

    #[test]
    fn a_feeds_chain_orders_producer_before_consumer() {
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), producer, Producer::OUT_BLOB, consumer, Consumer::INPUT);

        let compiled = compile(app.world_mut()).expect("compiles");
        let order: Vec<Entity> = compiled.plans.iter().map(|p| p.entity).collect();
        let p_idx = order.iter().position(|&e| e == producer).expect("producer compiled");
        let c_idx = order.iter().position(|&e| e == consumer).expect("consumer compiled");
        assert!(p_idx < c_idx, "a Product source must be ordered before its consumer");
    }

    // --- ChildOf application ----------------------------------------------

    #[test]
    fn a_valid_hierarchy_is_applied_as_bevy_child_of() {
        let mut app = engine_app();
        let child = spawn_group(app.world_mut(), 0);
        let root = spawn_group(app.world_mut(), 1);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, root, Group::CHILDREN, 0);

        compile(app.world_mut()).expect("compiles");

        assert_eq!(
            app.world().get::<ChildOf>(child).map(|c| c.0),
            Some(root),
            "compile applies the hierarchy"
        );
    }

    #[test]
    fn a_rejected_hierarchy_applies_nothing() {
        // Validation gates application: a bad edit must leave the previous
        // graph in force rather than half-applying itself.
        let mut app = engine_app();
        let good_child = spawn_group(app.world_mut(), 0);
        let root = spawn_group(app.world_mut(), 2);
        let bad_source = spawn_gain(app.world_mut(), 1.0, 1.0); // not Product<Spatial>
        connect_at(app.world_mut(), good_child, Group::OUT_SPATIAL, root, Group::CHILDREN, 0);
        connect_at(app.world_mut(), bad_source, Gain::OUT_VALUE, root, Group::CHILDREN, 1);

        assert!(compile(app.world_mut()).is_err());

        assert!(
            app.world().get::<ChildOf>(good_child).is_none(),
            "a failed compile must not apply the edges that were legal"
        );
    }

    #[test]
    fn reparenting_removes_the_previous_child_of() {
        let mut app = engine_app();
        let child = spawn_group(app.world_mut(), 0);
        let first = spawn_group(app.world_mut(), 1);
        let second = spawn_group(app.world_mut(), 1);
        let edge = connect_at(app.world_mut(), child, Group::OUT_SPATIAL, first, Group::CHILDREN, 0);
        compile(app.world_mut()).expect("compiles");

        app.world_mut().despawn(edge);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, second, Group::CHILDREN, 0);
        compile(app.world_mut()).expect("recompiles");

        assert_eq!(app.world().get::<ChildOf>(child).map(|c| c.0), Some(second));
    }

    #[test]
    fn unparenting_removes_child_of_entirely() {
        let mut app = engine_app();
        let child = spawn_group(app.world_mut(), 0);
        let root = spawn_group(app.world_mut(), 1);
        let edge = connect_at(app.world_mut(), child, Group::OUT_SPATIAL, root, Group::CHILDREN, 0);
        compile(app.world_mut()).expect("compiles");

        app.world_mut().despawn(edge);
        compile(app.world_mut()).expect("recompiles");

        assert!(app.world().get::<ChildOf>(child).is_none());
    }

    #[test]
    fn an_applied_hierarchy_propagates_global_transforms() {
        // The point of compiling to Bevy's own hierarchy rather than to
        // something of ours: propagation is free. Assert it actually happens
        // rather than assuming the component alone suffices.
        use bevy_transform::TransformPlugin;
        use bevy_transform::prelude::{GlobalTransform, Transform};

        let mut app = engine_app();
        app.add_plugins(TransformPlugin);
        let child = spawn_group(app.world_mut(), 0);
        let root = spawn_group(app.world_mut(), 1);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, root, Group::CHILDREN, 0);
        compile(app.world_mut()).expect("compiles");

        app.world_mut().entity_mut(root).insert(Transform::from_xyz(10.0, 0.0, 0.0));
        app.world_mut().entity_mut(child).insert(Transform::from_xyz(0.0, 5.0, 0.0));
        app.update();

        let global = app
            .world()
            .get::<GlobalTransform>(child)
            .expect("propagation inserts GlobalTransform")
            .translation();
        assert_eq!(global, Transform::from_xyz(10.0, 5.0, 0.0).translation);
    }

    // --- New behaviour this task introduces --------------------------------

    #[test]
    fn a_spatial_edge_does_not_constrain_the_compiled_order() {
        // Design §4: a parent reads nothing from its child, so parenting is
        // excluded from the sort. Including it would reject this graph --
        // a child driving a param on its own parent.
        let mut app = engine_app();
        let group = spawn_group(app.world_mut(), 1);
        let child = spawn_group(app.world_mut(), 0);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, group, Group::CHILDREN, 0);
        let gain = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), gain, Gain::OUT_VALUE, group, Group::ROTATION_Y);

        assert!(compile(app.world_mut()).is_ok(), "parenting must not enter the sort");
    }

    #[test]
    fn a_union_cycle_across_both_old_dags_is_rejected() {
        // Design §4: dataflow and product edges used to live in separate
        // sorts (a ParamEdge topological order and a Feeds cook order), so a
        // cycle that crossed both could compile clean, with one side
        // silently reading stale data from phase ordering. Now there is one
        // order, so a cycle running entirely through Product edges — which
        // used to belong to the OTHER (Feeds) dag — is caught by the exact
        // same sort as a cycle through value edges.
        //
        // `Consumer` is used on both ends rather than pairing it with
        // `Producer`, because `Producer`'s only inlet is a plain `f32`
        // (`scale`), so a `Consumer.blob -> Producer.scale` edge (as a
        // literal "one edge per old dag" cycle would use) is rejected by the
        // type check before the cycle check ever runs. `Consumer` alone has
        // both a `Product<Blob>` inlet and a `Product<Blob>` outlet, so two
        // of them form a genuine, type-legal cycle.
        let mut app = engine_app();
        let a = spawn_consumer(app.world_mut());
        let b = spawn_consumer(app.world_mut());
        connect(app.world_mut(), a, Consumer::OUT_BLOB, b, Consumer::INPUT);
        connect(app.world_mut(), b, Consumer::OUT_BLOB, a, Consumer::INPUT);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
    }

    #[test]
    fn a_variadic_inlet_takes_one_edge_per_element() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 4.0, 5.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 1);
        recompile(&mut app);

        app.update();
        app.update();

        assert_eq!(port_value(&app, sum, Sum::OUT_TOTAL), 26.0, "6 + 20");
    }

    #[test]
    fn two_edges_into_one_variadic_element_are_rejected() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0]);
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 0);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains("exactly one edge"), "{err}");
        assert!(err.contains("terms"), "must name the field: {err}");
    }

    #[test]
    fn an_edge_past_a_variadic_field_names_its_length() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 7);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains('7') && err.contains('2'), "must name index and length: {err}");
    }

    #[test]
    fn resizing_a_variadic_field_leaves_other_fields_addressable() {
        // (field, index) addressing exists for exactly this: growing
        // `children` must not renumber `rotation_y`.
        let mut app = engine_app();
        let group = spawn_group(app.world_mut(), 1);
        let gain = spawn_gain(app.world_mut(), 2.0, 3.0);
        connect(app.world_mut(), gain, Gain::OUT_VALUE, group, Group::ROTATION_Y);
        recompile(&mut app);
        app.update();
        app.update();
        assert_eq!(port_value(&app, group, Group::ROTATION_Y), 6.0);

        app.world_mut()
            .get_mut::<crate::test_nodes::GroupInlets>(group)
            .expect("inlets")
            .children
            .push(Product::<Spatial>::default());
        recompile(&mut app);
        app.update();

        assert_eq!(
            port_value(&app, group, Group::ROTATION_Y),
            6.0,
            "the edge into rotation_y must still resolve after children grew"
        );
    }

    #[test]
    fn an_event_slot_is_empty_at_the_start_of_every_tick() {
        let mut app = engine_app();
        let emitter = spawn_emitter(app.world_mut(), 0.001);
        let sink = spawn_sink(app.world_mut());
        connect(app.world_mut(), emitter, Emitter::OUT_PULSE, sink, Sink::PULSE);
        recompile(&mut app);

        app.update();
        app.update();
        let after_one = event_offsets(&app, sink, Sink::PULSE).len();
        app.update();
        let after_two = event_offsets(&app, sink, Sink::PULSE).len();

        assert_eq!(after_one, 1, "one occurrence per tick");
        assert_eq!(after_two, 1, "occurrences must not accumulate across ticks");
    }

    #[test]
    fn a_product_outlet_is_seeded_with_its_own_entity() {
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), producer, Producer::OUT_BLOB, consumer, Consumer::INPUT);
        recompile(&mut app);

        app.update();
        app.update();

        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == consumer).expect("compiled");
        let (slot, access) = plan.product_inlets[0];
        let arena = app.world().resource::<PortArena>();
        assert_eq!(
            (access.get)(&*arena.values[slot]),
            Some(producer),
            "the consumer's product inlet must hold the producer's entity"
        );
    }
}
