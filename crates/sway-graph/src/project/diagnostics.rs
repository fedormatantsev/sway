//! What a reload could not do, and to which item. Spec §4.3.
//!
//! Mirrors `GraphDiagnostics`: a resource the editor renders, never an error
//! that stops the app.

use bevy_ecs::component::Component;
use bevy_ecs::resource::Resource;

/// An entity's identity in the document, and its identity across reloads.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocId(pub String);

#[derive(Debug, Clone, PartialEq)]
pub enum ItemError {
    UnknownComponent { entity: String, name: String },
    BadPayload { entity: String, name: String, message: String },
    UnknownWire { entity: String, wire: String },
    UnresolvedTarget { entity: String, wire: String, target: String },
}

impl core::fmt::Display for ItemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownComponent { entity, name } => {
                write!(f, "{entity}: no component is registered as \"{name}\"")
            }
            Self::BadPayload { entity, name, message } => {
                write!(f, "{entity}.{name}: {message}")
            }
            Self::UnknownWire { entity, wire } => {
                write!(f, "{entity}: no wire is registered as \"{wire}\"")
            }
            Self::UnresolvedTarget { entity, wire, target } => {
                write!(f, "{entity}.{wire}: no entity has the id \"{target}\"")
            }
        }
    }
}

/// The result of the most recent load attempt.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct ProjectDiagnostics {
    /// Set when a reload was rejected whole. The running world is untouched.
    pub parse: Option<String>,
    /// Per-item failures; everything else applied.
    pub items: Vec<ItemError>,
}

impl ProjectDiagnostics {
    pub fn is_clean(&self) -> bool {
        self.parse.is_none() && self.items.is_empty()
    }
}
