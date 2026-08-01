//! The structure pass: `ParentEdge` and `FeedsEdge`. Design §4.
//!
//! Separate from the dataflow pass because structure edges are not param
//! dependencies and their failures need their own vocabulary — "cycle
//! detected" is unhelpful when the author filled one slot twice (parent
//! §2.5). `ParentEdge` enters no ordering at all; `FeedsEdge` produces
//! `cook_order`, which is a second topological sort over a different DAG.

use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::query::With;
use bevy_ecs::world::World;

use crate::compile::CompileError;
use crate::edges::{EdgeFrom, EdgeTo, FeedsEdge, ParentEdge};
use crate::slots::SlotField;

/// What the structure pass needs to know about one node — its registry entry
/// flattened, so this module reads no resources.
pub(crate) struct StructureNode {
    pub entity: Entity,
    pub type_name: &'static str,
    pub slots: Vec<SlotField>,
    pub produces: core::any::TypeId,
    pub produces_path: &'static str,
    pub spatial: bool,
}

pub(crate) struct Structure {
    /// Node indices in `Feeds`-topological order.
    pub cook_order: Vec<usize>,
    /// Per node index, per slot ordinal: the source node's index.
    pub slots: Vec<Vec<Option<usize>>>,
    /// Per node index: the parent node's index.
    pub parents: Vec<Option<usize>>,
}

pub(crate) fn validate(
    world: &mut World,
    nodes: &[StructureNode],
    index_of: &HashMap<Entity, usize>,
) -> Result<Structure, CompileError> {
    let n = nodes.len();

    // --- Parent edges -------------------------------------------------
    struct RawParent {
        edge: Entity,
        child: Entity,
        parent: Entity,
    }
    let mut parent_query = world.query_filtered::<(Entity, &EdgeFrom, &EdgeTo), With<ParentEdge>>();
    let raw_parents: Vec<RawParent> = parent_query
        .iter(world)
        .map(|(edge, from, to)| RawParent {
            edge,
            child: from.0,
            parent: to.0,
        })
        .collect();

    let mut parents: Vec<Option<usize>> = vec![None; n];
    let mut parent_entity: Vec<Option<Entity>> = vec![None; n];
    for raw in raw_parents {
        let &child_idx = index_of
            .get(&raw.child)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.child })?;
        let &parent_idx = index_of
            .get(&raw.parent)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.parent })?;

        if !nodes[child_idx].spatial {
            return Err(CompileError::NotSpatial {
                node: raw.child,
                type_name: nodes[child_idx].type_name,
                role: "parented",
            });
        }
        if !nodes[parent_idx].spatial {
            return Err(CompileError::NotSpatial {
                node: raw.parent,
                type_name: nodes[parent_idx].type_name,
                role: "used as a parent",
            });
        }
        if let Some(first) = parent_entity[child_idx] {
            return Err(CompileError::DuplicateParent {
                child: raw.child,
                first,
                second: raw.parent,
            });
        }
        parent_entity[child_idx] = Some(raw.parent);
        parents[child_idx] = Some(parent_idx);
    }

    // Parenting acyclicity: walk each chain, bounded by n steps.
    for start in 0..n {
        let mut seen = 0usize;
        let mut cursor = parents[start];
        let mut chain = vec![nodes[start].entity];
        while let Some(idx) = cursor {
            if idx == start {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            chain.push(nodes[idx].entity);
            seen += 1;
            if seen > n {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            cursor = parents[idx];
        }
    }

    // --- Feeds edges ---------------------------------------------------
    struct RawFeeds {
        edge: Entity,
        slot: u16,
        source: Entity,
        target: Entity,
    }
    let mut feeds_query = world.query::<(Entity, &FeedsEdge, &EdgeFrom, &EdgeTo)>();
    let raw_feeds: Vec<RawFeeds> = feeds_query
        .iter(world)
        .map(|(edge, feeds, from, to)| RawFeeds {
            edge,
            slot: feeds.slot,
            source: from.0,
            target: to.0,
        })
        .collect();

    let mut slots: Vec<Vec<Option<usize>>> =
        nodes.iter().map(|node| vec![None; node.slots.len()]).collect();
    let mut slot_source_entity: Vec<Vec<Option<Entity>>> =
        nodes.iter().map(|node| vec![None; node.slots.len()]).collect();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree = vec![0u32; n];

    for raw in raw_feeds {
        let &source_idx = index_of
            .get(&raw.source)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.source })?;
        let &target_idx = index_of
            .get(&raw.target)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.target })?;

        let target = &nodes[target_idx];
        let slot = target.slots.get(raw.slot as usize).ok_or(CompileError::SlotOutOfRange {
            node: raw.target,
            slot: raw.slot,
            arity: target.slots.len(),
        })?;

        let source = &nodes[source_idx];
        if source.produces == core::any::TypeId::of::<()>() {
            return Err(CompileError::SourceProducesNothing {
                source: raw.source,
                type_name: source.type_name,
                target: raw.target,
                slot: slot.name,
            });
        }
        if source.produces != slot.capability {
            return Err(CompileError::SlotTypeMismatch {
                target: raw.target,
                slot: slot.name,
                expected: slot.capability_path,
                source: raw.source,
                produces: source.produces_path,
            });
        }
        if let Some(first) = slot_source_entity[target_idx][raw.slot as usize] {
            return Err(CompileError::DuplicateSlot {
                target: raw.target,
                slot: slot.name,
                first,
                second: raw.source,
            });
        }

        slot_source_entity[target_idx][raw.slot as usize] = Some(raw.source);
        slots[target_idx][raw.slot as usize] = Some(source_idx);
        adjacency[source_idx].push(target_idx);
        in_degree[target_idx] += 1;
    }

    for adj in &mut adjacency {
        adj.sort_unstable();
    }

    // Kahn's again, over the Feeds DAG. Seeded in node order so a tie the
    // edges leave unresolved still breaks deterministically.
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut cook_order: Vec<usize> = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while let Some(idx) = queue.pop_front() {
        cook_order.push(idx);
        placed[idx] = true;
        for &next in &adjacency[idx] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    if cook_order.len() != n {
        let remaining: Vec<Entity> =
            (0..n).filter(|&i| !placed[i]).map(|i| nodes[i].entity).collect();
        return Err(CompileError::FeedsCycle { nodes: remaining });
    }

    Ok(Structure {
        cook_order,
        slots,
        parents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::test_nodes::{
        spawn_group, spawn_probe, spawn_sinkgeo, spawn_sludge_source, spawn_source, structure_app,
    };

    fn feeds(world: &mut World, from: Entity, to: Entity, slot: u16) -> Entity {
        world
            .spawn((FeedsEdge { slot }, EdgeFrom(from), EdgeTo(to)))
            .id()
    }

    fn parent(world: &mut World, child: Entity, parent: Entity) -> Entity {
        world
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(parent)))
            .id()
    }

    #[test]
    fn a_feeds_chain_orders_producer_before_consumer() {
        let mut app = structure_app();
        let src = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), src, sink, 0);

        let compiled = compile(app.world_mut()).expect("compiles");
        let cooked: Vec<Entity> = compiled
            .cook_order
            .iter()
            .map(|&i| compiled.plans[i].entity)
            .collect();
        let src_at = cooked.iter().position(|&e| e == src).expect("source cooks");
        let sink_at = cooked.iter().position(|&e| e == sink).expect("sink cooks");
        assert!(src_at < sink_at, "a Feeds source must cook first");
    }

    #[test]
    fn two_parent_edges_from_one_child_are_rejected() {
        let mut app = structure_app();
        let child = spawn_group(app.world_mut());
        let a = spawn_group(app.world_mut());
        let b = spawn_group(app.world_mut());
        parent(app.world_mut(), child, a);
        parent(app.world_mut(), child, b);

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
        let mut app = structure_app();
        let lfo_like = spawn_probe(app.world_mut()); // SPATIAL = false
        let group = spawn_group(app.world_mut());
        parent(app.world_mut(), lfo_like, group);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("scene node"), "{msg}");
        assert!(msg.contains(&format!("{lfo_like}")), "must name the node: {msg}");
    }

    #[test]
    fn parenting_under_a_non_spatial_node_is_rejected() {
        // The child-spatial check runs first in `validate`, so a spatial
        // child (`Group`) is required here to actually reach the
        // parent-spatial branch and its distinct "used as a parent" wording.
        let mut app = structure_app();
        let child = spawn_group(app.world_mut()); // SPATIAL = true
        let lfo_like = spawn_probe(app.world_mut()); // SPATIAL = false
        parent(app.world_mut(), child, lfo_like);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("used as a parent"), "{msg}");
        assert!(msg.contains(&format!("{lfo_like}")), "must name the non-spatial node: {msg}");
    }

    #[test]
    fn a_parenting_cycle_is_rejected() {
        let mut app = structure_app();
        let a = spawn_group(app.world_mut());
        let b = spawn_group(app.world_mut());
        parent(app.world_mut(), a, b);
        parent(app.world_mut(), b, a);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("parent"), "must speak of parenting, not dataflow: {msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_slot_filled_twice_is_rejected() {
        let mut app = structure_app();
        let a = spawn_source(app.world_mut());
        let b = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), a, sink, 0);
        feeds(app.world_mut(), b, sink, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("already filled"), "{msg}");
        assert!(msg.contains("input"), "must name the slot: {msg}");
        assert!(
            msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")),
            "must name both sources: {msg}"
        );
    }

    #[test]
    fn a_source_that_produces_nothing_is_rejected() {
        let mut app = structure_app();
        // `Group` produces nothing; feeding it into a Blob slot must not be
        // reported as a generic "cycle" or "out of range".
        let group = spawn_group(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), group, sink, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{group}")), "must name the source: {msg}");
        assert!(msg.contains("input"), "must name the slot: {msg}");
    }

    #[test]
    fn a_slot_type_mismatch_names_the_capability_on_both_sides() {
        let mut app = structure_app();
        // `SludgeSource` produces `Sludge`, a real (non-unit) capability
        // distinct from the `Blob` `SinkGeo.input` expects — unlike `Group`,
        // which produces nothing and would instead trip
        // `SourceProducesNothing` before this check is ever reached.
        let sludge = spawn_sludge_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), sludge, sink, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{sludge}")), "must name the source: {msg}");
        assert!(msg.contains("input"), "must name the slot: {msg}");

        // Tied to the real `TypePath::type_path()` values (rather than a
        // hardcoded guess at their format) and checked in their distinct
        // positions — "expects `Blob`" vs. "produces `Sludge`" — so a bug
        // that swapped the `expected`/`produces` fields in `SlotTypeMismatch`
        // would fail this test instead of slipping through on a bare
        // "message contains both names somewhere" check.
        use bevy_reflect::TypePath;
        let blob_path = <crate::test_nodes::Blob as TypePath>::type_path();
        let sludge_path = <crate::test_nodes::Sludge as TypePath>::type_path();
        assert!(
            msg.contains(&format!("expects `{blob_path}`")),
            "must name the slot's expected capability: {msg}"
        );
        assert!(
            msg.contains(&format!("produces `{sludge_path}`")),
            "must name the source's produced capability: {msg}"
        );
    }

    #[test]
    fn a_slot_ordinal_out_of_range_reports_the_arity() {
        let mut app = structure_app();
        let src = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), src, sink, 9);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains('9'), "{msg}");
        assert!(msg.contains('1'), "must state the Slots arity: {msg}");
    }

    #[test]
    fn a_feeds_cycle_is_rejected_in_feeds_vocabulary() {
        let mut app = structure_app();
        let a = spawn_sinkgeo(app.world_mut()); // has a slot AND produces
        let b = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), a, b, 0);
        feeds(app.world_mut(), b, a, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("Feeds"), "must name the edge kind: {msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }
}
