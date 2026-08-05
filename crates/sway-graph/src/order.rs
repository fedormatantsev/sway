//! Evaluation order. Spec §3.

use std::cmp::{Ord, Ordering};
use std::collections::{BinaryHeap, HashMap, HashSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::wire::PropagateFn;

/// `Entity::Ord` is DESCENDING in raw spawn index for bevy_ecs 0.19.0: its
/// NonMaxU32 niche encoding stores the bitwise complement of the index.
/// A plain BinaryHeap (max-heap) over Entity's native Ord therefore pops
/// in ascending raw-index order -- do NOT wrap this in `Reverse`, and do
/// NOT "simplify" this to Reverse<Entity> without re-verifying against
/// whatever Bevy version is pinned at the time.
#[derive(Copy, Clone, Eq, PartialEq)]
struct AscendingEntity(Entity);

impl Ord for AscendingEntity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)  // Uses Entity's descending-index Ord; max-heap pops ascending
    }
}

impl PartialOrd for AscendingEntity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One wire instance, flattened for the sort. `run` is the wire type's
/// monomorphised propagate; the sort ignores it.
#[derive(Clone, Copy)]
pub struct Link {
    pub src: Entity,
    pub dst: Entity,
    pub run: PropagateFn,
}

pub struct Sorted {
    /// Evaluation order: the acyclic part first, then any cycle members in
    /// ascending entity order.
    pub order: Vec<Entity>,
    /// Entities participating in a cycle. Empty in a well-formed graph.
    pub cycles: Vec<Entity>,
}

/// Kahn's algorithm over entities.
///
/// Ties are broken by ascending `Entity` so the order is deterministic — the
/// editor shows it, and §6's tests assert on it.
///
/// A cycle never stops the render (spec §3.3): its members are appended after
/// the acyclic part and read the previous tick's value.
pub fn topological_order(vertices: &[Entity], links: &[Link]) -> Sorted {
    let mut indegree: HashMap<Entity, usize> = vertices.iter().map(|&e| (e, 0)).collect();
    let mut successors: HashMap<Entity, Vec<Entity>> = HashMap::new();

    for link in links {
        // A link naming an entity outside `vertices` is ignored: the sort
        // orders what exists.
        if !indegree.contains_key(&link.src) || !indegree.contains_key(&link.dst) {
            continue;
        }
        *indegree.get_mut(&link.dst).expect("dst is a vertex") += 1;
        successors.entry(link.src).or_default().push(link.dst);
    }

    let mut ready: BinaryHeap<AscendingEntity> = indegree
        .iter()
        .filter(|(_, degree)| *degree == &0)
        .map(|(entity, _)| AscendingEntity(*entity))
        .collect();

    let mut order: Vec<Entity> = Vec::with_capacity(vertices.len());
    while let Some(AscendingEntity(entity)) = ready.pop() {
        order.push(entity);
        for &next in successors.get(&entity).map_or(&[][..], |v| v.as_slice()) {
            let degree = indegree.get_mut(&next).expect("successor is a vertex");
            *degree -= 1;
            if *degree == 0 {
                ready.push(AscendingEntity(next));
            }
        }
    }

    let placed: HashSet<Entity> = order.iter().copied().collect();
    let mut cycles: Vec<Entity> = vertices.iter().copied().filter(|e| !placed.contains(e)).collect();
    // Compensate for Entity::Ord being descending-in-index: reverse the sort to get
    // ascending raw-index order, consistent with the main order's tie-breaking.
    cycles.sort_by(|a, b| b.cmp(a));
    order.extend(cycles.iter().copied());

    Sorted { order, cycles }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(index: u32) -> Entity {
        Entity::from_raw_u32(index).expect("valid entity index")
    }

    fn noop(_: &mut World, _: Entity, _: Entity) {}

    fn link(src: Entity, dst: Entity) -> Link {
        Link { src, dst, run: noop }
    }

    #[test]
    fn a_chain_is_ordered_source_first() {
        // The design's central claim: a chain resolves in one tick, which is
        // only true if the order puts every producer before its consumer.
        let (a, b, c) = (e(3), e(1), e(2));
        let sorted = topological_order(&[a, b, c], &[link(a, b), link(b, c)]);

        assert_eq!(sorted.order, vec![a, b, c]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn independent_vertices_are_ordered_by_entity_for_determinism() {
        let (a, b, c) = (e(2), e(1), e(3));
        let sorted = topological_order(&[a, b, c], &[]);

        assert_eq!(sorted.order, vec![e(1), e(2), e(3)]);
    }

    #[test]
    fn fan_out_puts_the_producer_before_every_consumer() {
        let (src, x, y) = (e(9), e(1), e(2));
        let sorted = topological_order(&[src, x, y], &[link(src, x), link(src, y)]);

        assert_eq!(sorted.order[0], src);
        assert_eq!(&sorted.order[1..], &[x, y]);
    }

    #[test]
    fn a_cycle_is_reported_and_its_members_appended() {
        // Spec §3.3: never stop the render. The acyclic part keeps its order
        // and the cycle's members follow, deterministically.
        let (free, a, b) = (e(1), e(2), e(3));
        let sorted = topological_order(&[free, a, b], &[link(a, b), link(b, a)]);

        assert_eq!(sorted.order, vec![free, a, b]);
        assert_eq!(sorted.cycles, vec![a, b]);
    }

    #[test]
    fn a_link_naming_an_unknown_entity_is_ignored() {
        // A wire whose producer was despawned since the last rebuild must not
        // strand its consumer at indegree 1 forever.
        let (a, gone) = (e(1), e(77));
        let sorted = topological_order(&[a], &[link(gone, a)]);

        assert_eq!(sorted.order, vec![a]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn two_wires_between_the_same_pair_still_order_correctly() {
        // Duplicate constraints must decrement indegree once each.
        let (a, b) = (e(1), e(2));
        let sorted = topological_order(&[a, b], &[link(a, b), link(a, b)]);

        assert_eq!(sorted.order, vec![a, b]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn entity_ord_is_descending_in_raw_index_for_same_generation() {
        // Pins the bevy_ecs quirk AscendingEntity and the cycle sort both
        // compensate for. If a Bevy upgrade changes this, this test fails
        // loudly instead of silently flipping topological_order's determinism.
        assert!(e(1) > e(2), "if this fails, Entity::Ord's encoding changed -- \
            re-verify AscendingEntity and the cycles.sort_by direction");
    }
}
