//! The version 4 document, and the only code that reads its text.
//!
//! Design D9's sketch:
//!
//! ```ron
//! Graph(
//!     version: 4,
//!     nodes: {
//!         "lfoA":  Node(type: "CurveSampler", metadata: {"pos": {"glam::Vec2": (x: -460.0, y: 40.0)}}, inlets: (time: 0.0, keys: [])),
//!         "vec3A": Node(type: "MakeVec3", metadata: {}, inlets: (x: -0.8)),
//!     },
//!     edges: [ Edge(from: ("lfoA", "out"), to: ("vec3A", "y"), slot: 0) ],
//! )
//! ```
//!
//! `nodes` is keyed by the document's own stable id, never `NodeId` (that is
//! runtime-only — see `crate::v4::ids`). A node's `type` is the registered
//! kind's **short name**, the last `::`-segment of its `TypePath`, resolved to
//! a registered kind by `crate::v4::load`; this crate does not depend on
//! `sway-nodes` and does not resolve it here. Edge paths are relative to the
//! part they address (`"out"`, not `"outlets.out"`) — `sway-graph`'s resolver
//! prepends `inlets.` / `outlets.`, and `Graph::connect` is what actually
//! interprets them.
//!
//! Inlets payloads are captured as raw, unparsed RON text
//! (`ron::value::RawValue`), exactly as the version 2 document captures
//! component payloads (see `crate::doc`): `ron::Value` cannot drive
//! `bevy_reflect`'s `TypedReflectDeserializer` through an enum field, so the
//! loader re-parses each payload's raw text directly. An annotation's payload
//! is captured the same way, and re-parsed with the *untyped*
//! `ReflectDeserializer` — which is what the type-path key in the sketch above
//! is for. The document therefore declares nothing about what any annotation
//! key holds, and interprets none of them.

use std::collections::BTreeMap;

use ron::value::RawValue;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Bumped when the document shape changes incompatibly. An unknown version is
/// rejected rather than guessed at (spec: "Format version 4").
pub const FORMAT_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Graph")]
pub struct GraphDoc {
    pub version: u32,
    /// Stable id -> node entry. A `BTreeMap` so a reload with no shape change
    /// re-emits byte-identical text (spec: "An edge round-trips" depends on
    /// nothing here reordering between a load and the following save).
    #[serde(default, deserialize_with = "deserialize_nodes")]
    pub nodes: BTreeMap<String, NodeDoc>,
    #[serde(default)]
    pub edges: Vec<EdgeDoc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Node")]
pub struct NodeDoc {
    /// The node kind's short name — the last segment of its registered
    /// `TypePath`. Never a full module path (`crate::v4::load` resolves it
    /// against the registry), so a file move in the node-defining crate
    /// cannot break a saved document.
    #[serde(rename = "type")]
    pub kind: String,
    /// The node's annotations, keyed by name, each payload left unparsed here.
    /// No key has a field of its own and none is interpreted: a payload
    /// carries its own type path (`crate::v4::load` re-parses it with the
    /// untyped `ReflectDeserializer`), so the document declares nothing about
    /// what any key holds. A `BTreeMap` because the order they are written in
    /// has to be stable — saving an unchanged document twice must produce the
    /// same bytes.
    #[serde(default)]
    pub metadata: BTreeMap<String, Box<RawValue>>,
    /// The node's `inlets` part, and nothing else — never `state` or
    /// `outlets` (spec: "A document stores inlets only"). Left unparsed here;
    /// `crate::v4::load` re-parses it against the node kind's registered
    /// `inlets` type.
    pub inlets: Box<RawValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Edge")]
pub struct EdgeDoc {
    /// `(source node id, path within its outlets)`.
    pub from: (String, String),
    /// `(destination node id, path within its inlets)`.
    pub to: (String, String),
    /// A sort key, not an index (design D5) — sparse values are legal.
    pub slot: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The text is not valid RON, or does not match the document shape.
    Ron(String),
    UnsupportedVersion(u32),
    /// Two node entries share an id, so an edge naming it could not resolve
    /// unambiguously (spec: "Duplicate ids are refused" — parse fails whole).
    DuplicateId(String),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ron(message) => write!(f, "{message}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "graph document version {version} is not supported (this build reads {FORMAT_VERSION})"
            ),
            Self::DuplicateId(id) => write!(f, "two nodes share the id \"{id}\""),
        }
    }
}

impl core::error::Error for ParseError {}

/// Sentinel wrapped around a duplicate-id message by [`deserialize_nodes`] so
/// [`parse`] can recover [`ParseError::DuplicateId`] from serde's stringly
/// typed error channel rather than reporting it as a generic syntax error.
const DUPLICATE_MARKER: &str = "\u{0}sway-document-v4-duplicate-node-id\u{0}";

/// A `nodes` map's `Deserialize`, hand-written instead of derived so a
/// duplicate key can be caught here: the derived `BTreeMap<String, NodeDoc>`
/// impl silently keeps the last of two identical keys, which would make
/// `parse` unable to tell "two nodes share an id" from "one node, once".
fn deserialize_nodes<'de, D>(deserializer: D) -> Result<BTreeMap<String, NodeDoc>, D::Error>
where
    D: Deserializer<'de>,
{
    struct NodesVisitor;

    impl<'de> Visitor<'de> for NodesVisitor {
        type Value = BTreeMap<String, NodeDoc>;

        fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("a map of node id to node")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, NodeDoc>()? {
                if result.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "{DUPLICATE_MARKER}{key}{DUPLICATE_MARKER}"
                    )));
                }
            }
            Ok(result)
        }
    }

    deserializer.deserialize_map(NodesVisitor)
}

pub fn parse(text: &str) -> Result<GraphDoc, ParseError> {
    let mut doc: GraphDoc = ron::from_str(text).map_err(|error| classify(&error.to_string()))?;
    if doc.version != FORMAT_VERSION {
        return Err(ParseError::UnsupportedVersion(doc.version));
    }
    // Same `ron` 0.12.2 `RawValue`-reparse quirk `crate::doc::parse` works
    // around: a payload read back from text carries a leading space a
    // freshly-built one never had. Trim it so `Box<RawValue>` equality
    // reflects content, not incidental whitespace, which the round-trip
    // tests depend on.
    doc.nodes = std::mem::take(&mut doc.nodes)
        .into_iter()
        .map(|(id, mut node)| {
            node.inlets = node.inlets.trim_boxed();
            node.metadata = std::mem::take(&mut node.metadata)
                .into_iter()
                .map(|(key, value)| (key, value.trim_boxed()))
                .collect();
            (id, node)
        })
        .collect();
    Ok(doc)
}

fn classify(message: &str) -> ParseError {
    // `ron` wraps a custom deserialize error in position info ("{pos}:
    // {message}"), so the marker cannot be assumed to start the string —
    // only that it brackets the id, wherever it lands.
    if let Some(start) = message.find(DUPLICATE_MARKER) {
        let after = &message[start + DUPLICATE_MARKER.len()..];
        if let Some(end) = after.find(DUPLICATE_MARKER) {
            return ParseError::DuplicateId(after[..end].to_string());
        }
    }
    ParseError::Ron(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
Graph(
    version: 4,
    nodes: {
        "lfoA": Node(type: "CurveSampler", metadata: {"pos": {"glam::Vec2": (x: -460.0, y: 40.0)}}, inlets: (time: 0.0, keys: [(0.0, 0.0), (8.0, 0.5)])),
        "vec3A": Node(type: "MakeVec3", metadata: {}, inlets: (x: -0.8, y: 0.0, z: 0.0)),
    },
    edges: [
        Edge(from: ("lfoA", "out"), to: ("vec3A", "y"), slot: 0),
    ],
)
"#;

    #[test]
    fn a_document_parses_into_nodes_and_edges() {
        let doc = parse(MINIMAL).expect("parses");

        assert_eq!(doc.version, 4);
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.nodes["lfoA"].kind, "CurveSampler");
        assert_eq!(doc.nodes["lfoA"].metadata.len(), 1);
        assert!(doc.nodes["vec3A"].metadata.is_empty());
        assert_eq!(doc.edges.len(), 1);
        assert_eq!(doc.edges[0].from, ("lfoA".to_string(), "out".to_string()));
        assert_eq!(doc.edges[0].to, ("vec3A".to_string(), "y".to_string()));
        assert_eq!(doc.edges[0].slot, 0);
    }

    #[test]
    fn an_inlets_payload_is_kept_unparsed() {
        let doc = parse(MINIMAL).expect("parses");
        let text = ron::to_string(&doc.nodes["lfoA"].inlets).expect("a payload re-serializes");
        assert!(text.contains('8'), "the payload survived as data: {text}");
    }

    #[test]
    fn missing_edges_defaults_to_empty() {
        let doc = parse(
            r#"Graph(version: 4, nodes: { "a": Node(type: "K", metadata: {}, inlets: ()) })"#,
        )
        .expect("parses");
        assert!(doc.edges.is_empty());
    }

    #[test]
    fn an_entry_with_no_metadata_field_parses() {
        // Annotations are not required to be present: a node nothing has
        // annotated is written and read by the same rule as one that is.
        let doc = parse(r#"Graph(version: 4, nodes: { "a": Node(type: "K", inlets: ()) })"#)
            .expect("parses");
        assert!(doc.nodes["a"].metadata.is_empty());
    }

    #[test]
    fn an_unrecognised_annotation_key_is_carried_not_rejected() {
        // The document interprets no key, so one it has never seen is data
        // like any other.
        let doc = parse(
            r#"Graph(version: 4, nodes: {
                "a": Node(type: "K", metadata: {"who knows": {"f32": 1.5}}, inlets: ()),
            })"#,
        )
        .expect("parses");
        assert_eq!(doc.nodes["a"].metadata.len(), 1);
        assert!(doc.nodes["a"].metadata.contains_key("who knows"));
    }

    #[test]
    fn a_syntax_error_is_reported_not_panicked() {
        let error = parse("Graph(version: 4, nodes: {").expect_err("must fail");
        assert!(matches!(error, ParseError::Ron(_)), "got {error:?}");
    }

    #[test]
    fn a_version_three_document_is_refused_by_version() {
        // Version 3 carried a dedicated `pos` field. The whole-file version
        // check has to be what refuses it, naming both versions — failing on
        // whichever field happens to be missing would be a far worse message.
        let text = r#"Graph(
    version: 3,
    nodes: {
        "a": Node(type: "K", pos: (1.0, 2.0), inlets: ()),
    },
    edges: [],
)"#;
        let error = parse(text).expect_err("must fail");
        assert_eq!(error, ParseError::UnsupportedVersion(3));
        assert_eq!(
            error.to_string(),
            "graph document version 3 is not supported (this build reads 4)"
        );
    }

    #[test]
    fn an_earlier_version_is_rejected() {
        let error = parse(r#"Graph(version: 2, nodes: {}, edges: [])"#).expect_err("must fail");
        assert_eq!(error, ParseError::UnsupportedVersion(2));
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let error = parse(r#"Graph(version: 99, nodes: {}, edges: [])"#).expect_err("must fail");
        assert_eq!(error, ParseError::UnsupportedVersion(99));
    }

    #[test]
    fn a_duplicate_id_rejects_the_whole_document() {
        let text = r#"Graph(version: 4, nodes: {
            "a": Node(type: "K", metadata: {}, inlets: ()),
            "a": Node(type: "K", metadata: {}, inlets: ()),
        })"#;
        let error = parse(text).expect_err("must fail");
        assert_eq!(error, ParseError::DuplicateId("a".to_string()));
    }

    #[test]
    fn an_empty_document_is_valid() {
        let doc = parse("Graph(version: 4, nodes: {}, edges: [])").expect("parses");
        assert!(doc.nodes.is_empty());
        assert!(doc.edges.is_empty());
    }
}
