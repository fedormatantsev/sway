//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod ports;
pub mod schema;

pub use ports::{ContinuousIdx, Event, EventIdx, Occurrence, PortArena};
pub use schema::{derive_schema, register_event_port, PortField, ReflectEventPort, SchemaError, SchemaHalf};
