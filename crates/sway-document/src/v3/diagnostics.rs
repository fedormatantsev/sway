//! What a load could not do, and to which node or edge.
//!
//! Unlike version 2's `ProjectDiagnostics` (`crate::diagnostics`), a version 3
//! load never edits a live world — it builds a fresh `Graph` from nothing
//! (design D1's "load builds a `Graph`", not "reconciles one"), so there is no
//! `parse`/`items` split to make: a whole-document failure is a `ParseError`
//! returned from `crate::v3::parse`, and everything past that point is a
//! per-item skip recorded here (spec: "Unresolved ids, kinds and paths are
//! reported and skipped").

#[derive(Debug, Clone, PartialEq)]
pub enum LoadItemError {
    /// A node's `type` does not resolve to any registered node kind.
    UnknownNodeKind { id: String, kind: String },
    /// A node's `type` short name resolves to more than one registered kind.
    /// Load cannot guess which was meant, so the node is skipped exactly as
    /// an unknown kind would be.
    AmbiguousNodeKind {
        id: String,
        kind: String,
        candidates: Vec<String>,
    },
    /// A node's `inlets` payload did not parse, did not deserialize against
    /// the kind's registered `inlets` type, or did not apply. The node is
    /// skipped whole — nothing about it is inserted into the graph.
    BadInlets { id: String, message: String },
    /// An edge names a source id no node entry declares (including one that
    /// failed to load for a reason recorded separately).
    MissingEdgeSource { from: String, to: String },
    /// An edge names a destination id no node entry declares.
    MissingEdgeDestination { from: String, to: String },
    /// Both ids resolved, but `Graph::connect` refused the connection — a
    /// path that does not exist on the node it addresses, an incompatible
    /// pair of types, or a self-connection.
    EdgeRefused {
        from: String,
        to: String,
        reason: String,
    },
}

impl core::fmt::Display for LoadItemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownNodeKind { id, kind } => {
                write!(f, "{id}: no node kind is registered as \"{kind}\"")
            }
            Self::AmbiguousNodeKind {
                id,
                kind,
                candidates,
            } => write!(
                f,
                "{id}: \"{kind}\" names more than one registered kind: {}",
                candidates.join(", ")
            ),
            Self::BadInlets { id, message } => write!(f, "{id}.inlets: {message}"),
            Self::MissingEdgeSource { from, to } => {
                write!(f, "{from} -> {to}: no node has the id \"{from}\"")
            }
            Self::MissingEdgeDestination { from, to } => {
                write!(f, "{from} -> {to}: no node has the id \"{to}\"")
            }
            Self::EdgeRefused { from, to, reason } => {
                write!(f, "{from} -> {to}: {reason}")
            }
        }
    }
}

/// The result of one `crate::v3::load` call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadDiagnostics {
    pub items: Vec<LoadItemError>,
}

impl LoadDiagnostics {
    pub fn is_clean(&self) -> bool {
        self.items.is_empty()
    }
}
