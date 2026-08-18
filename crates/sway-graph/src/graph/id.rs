//! Identity for nodes and edges.
//!
//! `NodeId` is a **generational index** into [`Graph`](crate::graph::Graph)'s
//! node `Vec`. The index is reused when a slot is freed; the generation is not,
//! so a stale id never resolves to the node that took its slot
//! (`graph`: A graph is nodes and edges — "a stale identity does not resolve").
//!
//! `NodeId`'s `Ord` is `(index, generation)` ascending, which is what the
//! evaluation sort and the variadic-slot sort use to break ties. Unlike
//! `bevy_ecs::entity::Entity` — whose `Ord` runs *descending* in raw index
//! because of its `NonMaxU32` niche encoding — there is no inversion to
//! compensate for here.

use core::fmt;

use bevy_reflect::Reflect;

/// A node's identity, stable for as long as that node exists.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[reflect(Clone, PartialEq, Hash, Debug)]
pub struct NodeId {
    index: u32,
    generation: u32,
}

impl NodeId {
    /// An id that never resolves. Useful as a placeholder in tests and as the
    /// "nothing selected" sentinel when an `Option` is inconvenient.
    pub const PLACEHOLDER: Self = Self {
        index: u32::MAX,
        generation: u32::MAX,
    };

    /// Builds an id directly. Only the graph should mint ids; this exists for
    /// tests and for deserializers that already hold both halves.
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The slot this id addresses.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// How many times that slot had been reused when this id was minted.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}v{}", self.index, self.generation)
    }
}

/// An edge's identity. Monotonic and never reused, so an editor may hold one
/// across edits and a disconnect can name exactly one edge on a variadic
/// inlet.
#[derive(Reflect, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[reflect(Clone, PartialEq, Hash, Debug)]
pub struct EdgeId(u64);

impl EdgeId {
    /// Builds an id directly. Only the graph should mint ids.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying counter value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_order_by_index_then_generation_ascending() {
        // The sort's tie-breaker depends on this, and `Entity` gets it
        // backwards — so pin it.
        assert!(NodeId::new(0, 9) < NodeId::new(1, 0));
        assert!(NodeId::new(1, 0) < NodeId::new(1, 1));
    }

    #[test]
    fn a_reused_slot_is_a_different_id() {
        assert_ne!(NodeId::new(3, 0), NodeId::new(3, 1));
    }
}
