//! The graph model — nodes, edges, commands, order and the tick.
//!
//! This is the model described by `openspec/changes/redesign-graph-model`. It
//! lands **beside** the entity/wire engine in the crate root; nothing there is
//! deleted until that change's group 9.
//!
//! ## Shape
//!
//! - A [`Graph`] is [`Node`]s and [`Edge`]s and nothing else. It is a
//!   `Resource`, not a sub-world.
//! - A [`Node`] is one reflected value with exactly three parts — `inlets`,
//!   `state`, `outlets` — where an absent part is `()`.
//! - An [`Edge`] is `(src [`NodeId`], outlet path) -> (dst [`NodeId`], inlet
//!   path)` plus a `slot`, which is a sort key rather than an index.
//! - A node kind implements [`NodeKind`] and is registered with
//!   [`register_node_kind`], which also records the reflected type of each
//!   part as [`NodeParts`] type data.
//! - Everything outside the graph writes it through [`GraphCommand`].
//! - [`GraphPlugin`] schedules the command drain and the tick.

pub mod command;
pub mod edge;
pub mod id;
pub mod legality;
pub mod model;
pub mod node;
pub mod order;
pub mod path;
pub mod registry;
#[cfg(test)]
pub mod testing;
pub mod tick;

pub use command::{
    CommandOutcome, FieldValue, GraphCommand, GraphRx, apply_graph_command, apply_graph_commands,
};
pub use edge::{Compat, Edge, Port};
pub use id::{EdgeId, NodeId};
pub use legality::{compatibility, compatibility_of_values, is_valueless};
pub use model::{ConnectError, Graph};
pub use node::{Node, Part};
pub use order::{EvalOrder, GraphStep, Link, PropagateStep, Sorted, Target, topological_order};
pub use registry::{
    NodeKind, NodeParts, PartType, ReflectNodeKind, RegisterNodeKind, node_kind_type_id,
    register_node_kind, registered_node_kinds,
};
pub use tick::{GraphPlugin, run, tick_graph};
