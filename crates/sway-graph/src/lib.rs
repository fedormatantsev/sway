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

pub use compile::{ClearFn, CompileError, CompiledGraph, NodePlan, compile};
pub use edges::{Edge, EdgeFrom, EdgeTo, Endpoint, EditorPos, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges};
pub use ports::{clear_events_of, Events, Occurrence, PortArena, Product, SlotIdx, Spatial};
pub use registry::{
    CookFn, InletLensFn, InsertDefaultsFn, NodeType, NodeTypeEntry, NodeTypeId, NodeTypeRegistry,
    PrefillFn, SeedOutletsFn, TickFn, TickOfFn, register_node_type,
};
pub use schema::{
    derive_fields, register_events, register_product, FieldKind, FieldSpec, ProductAccess,
    ReflectEventList, ReflectProduct, SchemaError,
};
pub use tick::{GraphPlugin, GraphTickCount, graph_tick};
pub use view::{PortView, TickCtx};
