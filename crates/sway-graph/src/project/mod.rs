//! The project document: reading it, applying it, writing it.
//! Spec: docs/superpowers/specs/2026-08-06-project-format-design.md

pub mod apply;
pub mod diagnostics;
pub mod doc;
pub mod registry;

pub use apply::apply;
pub use diagnostics::{DocId, ItemError, ProjectDiagnostics};
pub use doc::{EntityDoc, FORMAT_VERSION, ParseError, ProjectDoc, parse};
pub use registry::{ComponentDocRegistry, ComponentEntry, register_authorable};
