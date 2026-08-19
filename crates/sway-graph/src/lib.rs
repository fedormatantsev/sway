//! The sway graph engine: a `Graph` resource of nodes and edges.
//!
//! Nodes are addressed by generational [`NodeId`]. An edge names two nodes
//! and two field paths. Evaluation is an exclusive walk over that resource
//! via `World::resource_scope`.

pub mod graph;
pub mod viewport_input;

pub use graph::{
    CommandOutcome, Compat, ConnectError, Edge, EdgeId, EvalOrder, FieldValue, Graph, GraphCommand,
    GraphPlugin, GraphRx, GraphStep, GraphTickSet, Node, NodeId, NodeKind, NodeParts, Part,
    PartType, Port, PropagateStep, ReflectNodeKind, RegisterNodeKind, Target, apply_graph_command,
    apply_graph_commands, node_kind_type_id, register_node_kind, registered_node_kinds, tick_graph,
};
pub use viewport_input::{
    ViewportButton, ViewportInput, ViewportInputRx, ViewportKey, ViewportModifiers,
    normalize_viewport_pos,
};
