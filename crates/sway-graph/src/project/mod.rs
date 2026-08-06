//! The project document: reading it, applying it, writing it.
//! Spec: docs/superpowers/specs/2026-08-06-project-format-design.md

pub mod doc;

pub use doc::{EntityDoc, FORMAT_VERSION, ParseError, ProjectDoc, parse};
