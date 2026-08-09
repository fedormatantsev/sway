//! `sway-app`'s library surface.
//!
//! The binary (`src/main.rs`) is the real entry point; this file exists so
//! an integration test in `tests/` can reach `demo_assets` without
//! reimplementing it. Nothing here is meant to be depended on from outside
//! this crate's own tests.

pub mod demo_assets;
