//! The sway graph engine: a `Graph` resource of nodes and edges.
//!
//! Nodes are addressed by generational [`NodeId`]. An edge names two nodes
//! and two field paths. Evaluation is an exclusive walk over that resource
//! via `World::resource_scope`.

pub mod graph;

pub use graph::{
    Compat, ConnectError, Edge, EdgeId, FieldWrite, Graph, GraphPlugin, GraphTickSet, Node, NodeId,
    NodeKind, Part, Port, ReflectNodeKind, RegisterNodeKind, is_empty_part, node_kind_type_id,
    part_type, register_node_kind, registered_node_kinds, tick_graph,
};
