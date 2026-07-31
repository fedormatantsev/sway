//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod compile;
pub mod edges;
pub mod ports;
pub mod registry;
pub mod schema;
#[cfg(test)]
pub(crate) mod test_nodes;
pub mod tick;
pub mod view;

pub use compile::{CompileError, CompiledGraph, NodePlan, compile};
pub use edges::{
    EdgeFrom, EdgeTo, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges, ParamEdge, PortKind,
};
pub use ports::{ContinuousIdx, Event, EventIdx, Occurrence, PortArena};
pub use registry::{
    InsertDefaultsFn, NodeSchema, NodeType, NodeTypeEntry, NodeTypeId, NodeTypeRegistry, PrefillFn,
    SeedOutputsFn, TickFn, TickOfFn, register_node_type,
};
pub use schema::{
    PortField, ReflectEventPort, SchemaError, SchemaHalf, derive_schema, register_event_port,
};
pub use tick::{GraphPlugin, GraphTickCount, graph_tick};
pub use view::{EventRef, PortView, TickCtx};
