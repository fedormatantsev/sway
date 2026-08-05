//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod compile;
pub mod ctx;
pub mod edges;
pub mod order;
pub mod ports;
pub mod registry;
pub mod registry_wires;
pub mod run;
pub mod schema;
#[cfg(test)]
pub(crate) mod test_nodes;
pub mod tick;
pub mod transport;
pub mod view;
pub mod wire;
#[cfg(test)]
pub(crate) mod test_wires;

pub use compile::{ClearFn, CompileError, CompiledGraph, NodePlan, compile};
pub use edges::{Edge, EdgeFrom, EdgeTo, Endpoint, EditorPos, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges};
pub use order::{topological_order, Link, Sorted};
pub use ports::{clear_events_of, Events, Occurrence, PortArena, Product, Spatial};
pub use registry::{
    CookFn, InletLensFn, InsertDefaultsFn, NodeType, NodeTypeEntry, NodeTypeId, NodeTypeRegistry,
    PrefillFn, SeedOutletsFn, TickFn, TickOfFn, register_node_type,
};
pub use registry_wires::{
    register_behaviour, register_wire, BehaviourEntry, BehaviourFn, BehaviourRegistry, WireEntry,
    WireRegistry,
};
pub use run::{graph_tick as wire_tick, WireTickCount, WiresPlugin};
pub use schema::{
    derive_fields, register_events, register_product, FieldKind, FieldSpec, ProductAccess,
    ReflectEventList, ReflectProduct, SchemaError,
};
pub use tick::{GraphPlugin, GraphTickCount, graph_tick};
pub use transport::{MusicalTime, Transport, TransportState, TransportTime};
pub use view::{PortView, TickCtx};
pub use wire::{propagate_of, PropagateFn, Wire};
