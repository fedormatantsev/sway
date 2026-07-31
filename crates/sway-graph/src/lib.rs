//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod compile;
pub mod ports;
pub mod registry;
pub mod schema;
pub mod view;

pub use compile::NodePlan;
pub use ports::{ContinuousIdx, Event, EventIdx, Occurrence, PortArena};
pub use registry::{
    register_node_type, NodeSchema, NodeType, NodeTypeEntry, NodeTypeId, NodeTypeRegistry,
    PrefillFn, TickFn, TickOfFn,
};
pub use schema::{derive_schema, register_event_port, PortField, ReflectEventPort, SchemaError, SchemaHalf};
pub use view::{PortView, TickCtx};
