//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod compile;
pub mod edges;
pub mod ports;
pub mod registry;
pub mod schema;
pub mod slots;
mod structure;
#[cfg(test)]
pub(crate) mod test_nodes;
pub mod tick;
pub mod view;

pub use compile::{CompileError, CompiledGraph, NodePlan, compile};
pub use edges::{
    EdgeFrom, EdgeTo, EditorPos, FeedsEdge, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges,
    ParamEdge, ParentEdge, PortKind,
};
pub use ports::{
    clear_events_of, BoxedOccurrence, ContinuousIdx, Event, EventIdx, Events, Occurrence,
    PortArena, Product, SlotIdx, Spatial,
};
pub use registry::{
    InsertDefaultsFn, NodeSchema, NodeType, NodeTypeEntry, NodeTypeId, NodeTypeRegistry, PrefillFn,
    SeedOutputsFn, TickFn, TickOfFn, register_node_type,
};
pub use schema::{
    PortField, ReflectEventPort, SchemaError, SchemaHalf, derive_schema, register_event_port,
};
pub use slots::{
    NoOutputs, NoSlots, ReflectSlot, Slot, SlotField, SlotSource, derive_slots, register_slot,
};
pub use tick::{GraphPlugin, GraphTickCount, graph_tick};
pub use view::{EventRef, PortView, SlotView, TickCtx};
