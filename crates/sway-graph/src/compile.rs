//! Graph compilation: turning a graph of node instances into a flat,
//! contiguous [`NodePlan`] per node plus the shared [`PortArena`] layout.
//!
//! **Stub for Task 3.** Task 4 owns this module and fills in the actual
//! compilation pass (topological ordering, `continuous_copies`,
//! `event_merges`, arena sizing, etc). Task 3 only needs `NodePlan` to exist
//! with the fields `check_ordinals`/`prefill_of` read, so that
//! `registry.rs` compiles and its tests can construct one.

use bevy_ecs::entity::Entity;

use crate::registry::{NodeSchema, NodeTypeId};

/// The compiled, per-node-instance plan the runner and prefill step read.
///
/// Task 4 will extend this with `continuous_copies` (edges into this node's
/// continuous inputs) and `event_merges` (edges into this node's event
/// inputs), and will own constructing it during compilation.
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
}
