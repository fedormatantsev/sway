//! The project document: reading it, applying it, writing it.
//! Spec: docs/superpowers/specs/2026-08-06-project-format-design.md
//!
//! Extracted from `sway-graph` in M6 (spec M6-2). The component registry
//! deliberately did *not* come with it: which component types are authorable
//! is a property of the ECS authoring surface, and the palette and inspector
//! both read it without any document existing.

pub mod apply;
pub mod asset;
pub mod claim;
pub mod diagnostics;
pub mod doc;
pub mod emit;
pub mod file;
pub mod v3;

pub use apply::apply;
pub use asset::{ProjectAsset, ProjectHandle, ProjectLoader, ProjectPlugin};
pub use claim::claim_editor_entities;
pub use diagnostics::{DocId, ItemError, ProjectDiagnostics};
pub use doc::{EntityDoc, FORMAT_VERSION, ParseError, ProjectDoc, parse};
pub use emit::{to_document, to_ron};
pub use file::{CurrentDocument, LastApplied, open_from_path, save_to_path};
