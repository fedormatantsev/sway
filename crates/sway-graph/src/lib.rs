//! The sway wire engine. Spec: docs/superpowers/specs/2026-08-05-wires-design.md

pub mod behaviour;
pub mod command;
pub mod ctx;
pub mod diagnostics;
pub mod dispatch;
pub mod order;
pub mod register;
pub mod registry_components;
pub mod run;
#[cfg(any(test, feature = "test-wires"))]
pub mod test_wires;
pub mod viewport_input;
pub mod watch;
pub mod wire;

pub use behaviour::{Behaviour, ReflectBehaviour};
pub use command::{
    EditorCommand, EditorRx, FieldValue, apply_editor_command, apply_editor_commands,
};
pub use ctx::{EditorPos, HiddenFromEditor, Selection, TickCtx};
pub use diagnostics::GraphDiagnostics;
pub use order::{GraphOrder, Link, Sorted, Step, TopologyDirty, rebuild_order, topological_order};
pub use register::{register_behaviour_type, register_wire_type};
pub use registry_components::{ComponentDocRegistry, ComponentEntry, register_authorable};
pub use run::{WireTickCount, WiresPlugin, graph_tick};
pub use viewport_input::{
    ViewportButton, ViewportInput, ViewportInputRx, ViewportKey, ViewportModifiers,
    normalize_viewport_pos,
};
pub use watch::{Authoring, WatchSet};
pub use wire::{ReflectWire, Wire, propagate_field_copy, propagate_reflected};
