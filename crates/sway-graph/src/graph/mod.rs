//! The graph model — nodes, edges, commands, order and the tick.
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
//!   [`register_node_kind`], which asserts the three-part shape. The reflected
//!   type of each part is read off the kind's own `TypeInfo` by [`part_type`].
//! - Everything outside the graph writes it through the graph's own
//!   operations: [`Graph::insert`], [`Graph::create`], [`Graph::remove`],
//!   [`Graph::set_field`], [`Graph::connect`], [`Graph::disconnect`] and
//!   [`Graph::set_slot`]. There is no second vocabulary restating them as
//!   data — a surface that cannot reach the graph when a gesture happens
//!   records it in a form of its own and applies it later.
//! - [`GraphPlugin`] inserts the resource and schedules the tick.

pub mod edge;
pub mod id;
pub mod legality;
pub mod model;
pub mod node;
// Private: rebuild and the tick are the only callers, and the plan they
// produce is not something a consumer reaches into.
mod order;
pub mod path;
pub mod registry;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod tick;

pub use edge::{Compat, Edge, Port};
pub use id::{EdgeId, NodeId};
pub use legality::{compatibility, is_valueless};
pub use model::{ConnectError, FieldWrite, Graph};
pub use node::{Node, Part};
pub use registry::{
    NodeKind, ReflectNodeKind, RegisterNodeKind, is_empty_part, node_kind_type_id, part_type,
    register_node_kind, registered_node_kinds,
};
pub use tick::{GraphPlugin, GraphTickSet, tick_graph};
