//! The sway graph engine.
//!
//! Two models live here during the `redesign-graph-model` migration:
//!
//! - [`graph`] — the node/edge model: a `Graph` resource holding `Vec<Node>`
//!   plus an edge list, addressed by generational `NodeId`. **New work goes
//!   here.**
//! - everything else — the entity/wire engine
//!   (`docs/superpowers/specs/2026-08-05-wires-design.md`), kept compiling
//!   until that change's group 9 deletes it.
//!
//! The two do not share types. Where a name would collide, the new one keeps
//! its `graph::` path: `graph::FieldValue` is the graph command set's, and
//! `FieldValue` at the crate root is still `EditorCommand`'s.

pub mod behaviour;
pub mod command;
pub mod ctx;
pub mod diagnostics;
pub mod dispatch;
pub mod graph;
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

// --- the graph model -------------------------------------------------------
//
// `graph::FieldValue` is deliberately not re-exported: the crate root's
// `FieldValue` is still `EditorCommand`'s, until group 9 removes it.
pub use graph::{
    CommandOutcome, Compat, ConnectError, Edge, EdgeId, EvalOrder, Graph, GraphCommand,
    GraphPlugin, GraphRx, GraphStep, Node, NodeId, NodeKind, NodeParts, Part, PartType, Port,
    PropagateStep, ReflectNodeKind, RegisterNodeKind, Target, apply_graph_command,
    apply_graph_commands, node_kind_type_id, register_node_kind, registered_node_kinds, tick_graph,
};
