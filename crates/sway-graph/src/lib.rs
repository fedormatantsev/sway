//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod ports;

pub use ports::{ContinuousIdx, EventIdx, Occurrence, PortArena};
