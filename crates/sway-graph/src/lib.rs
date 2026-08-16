//! The sway wire engine. Spec: docs/superpowers/specs/2026-08-05-wires-design.md

pub mod command;
pub mod ctx;
pub mod diagnostics;
pub mod order;
pub mod registry_components;
pub mod registry_wires;
pub mod run;
#[cfg(any(test, feature = "test-wires"))]
pub mod test_wires;
pub mod viewport_input;
pub mod watch;
pub mod wire;

pub use command::{
    EditorCommand, EditorRx, FieldValue, apply_editor_command, apply_editor_commands,
};
pub use ctx::{EditorPos, HiddenFromEditor, Selection, TickCtx};
pub use diagnostics::GraphDiagnostics;
pub use order::{GraphOrder, Link, Sorted, Step, TopologyDirty, rebuild_order, topological_order};
pub use registry_components::{ComponentDocRegistry, ComponentEntry, register_authorable};
pub use registry_wires::{
    BehaviourEntry, BehaviourFn, BehaviourRegistry, WireEntry, WireRegistry, register_behaviour,
    register_wire,
};
pub use run::{WireTickCount, WiresPlugin, graph_tick};
pub use viewport_input::{
    ViewportButton, ViewportInput, ViewportInputRx, ViewportKey, ViewportModifiers,
    normalize_viewport_pos,
};
pub use watch::{Authoring, WatchSet};
pub use wire::{PropagateFn, Wire, propagate_of};
