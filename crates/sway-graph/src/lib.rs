//! The sway graph engine: a `Graph` resource of nodes and edges.
//!
//! Nodes are addressed by generational [`NodeId`]. An edge names two nodes
//! and two field paths. Evaluation is an exclusive walk over that resource
//! via `World::resource_scope`.

pub mod graph;

pub use graph::{
    Compat, ConnectError, Edge, EdgeId, EvalOrder, FieldWrite, Graph, GraphPlugin, GraphStep,
    GraphTickSet, Node, NodeId, NodeKind, NodeParts, Part, PartType, Port, PropagateStep,
    ReflectNodeKind, RegisterNodeKind, Target, node_kind_type_id, register_node_kind,
    registered_node_kinds, tick_graph,
};
