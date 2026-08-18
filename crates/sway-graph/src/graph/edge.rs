//! Edges: `(src node, outlet path) -> (dst node, inlet path)` plus a slot.
//!
//! Design D5. An edge's stored paths are **relative to the part** —
//! `"frequency"`, not `"inlets.frequency"`. The resolver prepends `outlets.` /
//! `inlets.`, because an edge's source is always an outlet and its destination
//! is always an inlet, so the prefix carries no information.

use crate::graph::id::{EdgeId, NodeId};

/// One end of an edge: a node and a path relative to that node's part.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Port {
    /// The node this end names.
    pub node: NodeId,
    /// A `bevy_reflect` path relative to the part — `"frequency"`,
    /// `"transform.translation.x"`. Empty addresses the whole part.
    pub path: String,
}

impl Port {
    /// Builds a port.
    pub fn new(node: NodeId, path: impl Into<String>) -> Self {
        Self {
            node,
            path: path.into(),
        }
    }
}

/// How the destination accepts the source type — the legality rule's verdict,
/// decided once at connect time (`graph`: An edge addresses fields by path —
/// "legality MUST be decided when the connection is made").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Compat {
    /// `D == S`. The value is copied straight across.
    Direct,
    /// `D == Option<S>`. The value is wrapped in `Some`.
    Optional,
    /// `D == Vec<S>`. Many edges may land here; the `Vec` is derived from the
    /// edge set in slot order.
    Variadic,
}

/// A connection. Not a node, not a type — data in the graph.
#[derive(Clone, Debug)]
pub struct Edge {
    /// Stable identity, minted at connect and never reused.
    pub id: EdgeId,
    /// The producing node and a path within its `outlets`.
    pub src: Port,
    /// The consuming node and a path within its `inlets`.
    pub dst: Port,
    /// A **sort key, not an index**. Sparse keys are legal; the `Vec` behind a
    /// variadic inlet is sized from the edge count, so inserting between two
    /// layers changes one key and nothing else.
    pub slot: i32,
    /// How the destination accepts the source type.
    pub compat: Compat,
    /// `true` when the source value is zero-sized — a marker/protocol edge.
    ///
    /// Design D6 asks for either "zero-sized source type" or "an explicit flag
    /// set at connect time"; this is both, and deliberately so. The *decision*
    /// is made at connect from `size_of_val` on the resolved source value, so
    /// `sway-graph` never needs to know a marker type by name; the *result* is
    /// stored on the edge, so rebuild can skip the propagate step without a
    /// type-registry lookup per edge. A ZST copy would be a no-op anyway —
    /// storing the flag is what lets rebuild emit no step at all while still
    /// keeping the edge as a sort constraint.
    pub valueless: bool,
}

impl Edge {
    /// Whether this edge produces a propagate step. The inverse of
    /// [`Edge::valueless`], spelled out for readers of the rebuild.
    pub fn carries_value(&self) -> bool {
        !self.valueless
    }

    /// The key the variadic sort uses: slot first, then the source node so the
    /// order is deterministic, then the edge id so two edges from one node into
    /// one inlet at one slot still order stably.
    pub fn sort_key(&self) -> (i32, NodeId, EdgeId) {
        (self.slot, self.src.node, self.id)
    }
}
