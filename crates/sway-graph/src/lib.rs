//! The sway wire engine. Spec: docs/superpowers/specs/2026-08-05-wires-design.md

pub mod ctx;
pub mod diagnostics;
pub mod order;
pub mod registry_wires;
pub mod run;
#[cfg(test)]
pub(crate) mod test_wires;
pub mod transport;
pub mod watch;
pub mod wire;

pub use ctx::{EditorPos, TickCtx};
pub use diagnostics::GraphDiagnostics;
pub use order::{GraphOrder, Link, Sorted, Step, TopologyDirty, rebuild_order, topological_order};
pub use registry_wires::{
    BehaviourEntry, BehaviourFn, BehaviourRegistry, WireEntry, WireRegistry, register_behaviour,
    register_wire,
};
pub use run::{WireTickCount, WiresPlugin, graph_tick};
pub use transport::{MusicalTime, Transport, TransportState, TransportTime};
pub use watch::{Authoring, WatchSet};
pub use wire::{PropagateFn, Wire, propagate_of};
