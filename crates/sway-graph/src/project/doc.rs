//! The document, and the only code that reads text. Spec §2.
//!
//! Deliberately free of `World`, registries and Bevy: a document is data, and
//! every syntax-level failure is decided here so the applier can assume a
//! coherent one.
//!
//! Component payloads are captured as raw, unparsed RON text
//! (`ron::value::RawValue`) rather than `ron::Value`: Task 1 found that
//! `ron::Value` cannot drive `bevy_reflect`'s `TypedReflectDeserializer`
//! through an enum field. Task 6 re-parses each payload's raw text directly
//! via `ron::Deserializer::from_str` instead.

use std::collections::BTreeMap;

use ron::value::RawValue;
use serde::{Deserialize, Serialize};

/// Bumped when the document shape changes incompatibly. An unknown version is
/// rejected rather than guessed at.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Project")]
pub struct ProjectDoc {
    pub version: u32,
    pub entities: Vec<EntityDoc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Entity")]
pub struct EntityDoc {
    /// Stable identity across reloads, and the entity's `Name` in the world
    /// (spec §2.4). Renaming is a delete plus an add.
    pub id: String,
    /// Short registered component name -> its payload, left unparsed here.
    /// A `BTreeMap` rather than the file's own order: the reader never
    /// rewrites the file, so only the emitter sees this order, and
    /// alphabetical is deterministic.
    #[serde(default)]
    pub components: BTreeMap<String, Box<RawValue>>,
    /// Wire `NAME` -> the id of the producer.
    #[serde(default)]
    pub wires: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The text is not valid RON, or does not match the document shape.
    Ron(String),
    UnsupportedVersion(u32),
    /// Two entities share an id, so nothing in the document can be resolved
    /// unambiguously (spec §4.3: this rejects the whole reload).
    DuplicateId(String),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ron(message) => write!(f, "{message}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "project version {version} is not supported (this build reads {FORMAT_VERSION})"
            ),
            Self::DuplicateId(id) => write!(f, "two entities share the id \"{id}\""),
        }
    }
}

impl core::error::Error for ParseError {}

pub fn parse(text: &str) -> Result<ProjectDoc, ParseError> {
    let doc: ProjectDoc = ron::from_str(text).map_err(|e| ParseError::Ron(e.to_string()))?;
    if doc.version != FORMAT_VERSION {
        return Err(ParseError::UnsupportedVersion(doc.version));
    }
    let mut seen = std::collections::HashSet::new();
    for entity in &doc.entities {
        if !seen.insert(entity.id.as_str()) {
            return Err(ParseError::DuplicateId(entity.id.clone()));
        }
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
Project(
    version: 1,
    entities: [
        Entity(
            id: "lfoA",
            components: {
                // a comment, which RON keeps and the parser ignores
                "Lfo": (beats: 8.0, amplitude: 0.5),
                "FloatOut": (0.0),
            },
            wires: {},
        ),
        Entity(
            id: "cube",
            components: { "Transform": (translation: (0.8, 0.0, 0.0)) },
            wires: { "translation.y": "lfoA", "parent": "group" },
        ),
    ],
)
"#;

    #[test]
    fn a_document_parses_into_entities_components_and_wires() {
        let doc = parse(MINIMAL).expect("parses");

        assert_eq!(doc.version, 1);
        assert_eq!(doc.entities.len(), 2);
        assert_eq!(doc.entities[0].id, "lfoA");
        assert_eq!(doc.entities[0].components.len(), 2);
        assert!(doc.entities[0].components.contains_key("Lfo"));
        assert_eq!(
            doc.entities[1].wires.get("translation.y").map(String::as_str),
            Some("lfoA")
        );
        assert_eq!(
            doc.entities[1].wires.get("parent").map(String::as_str),
            Some("group")
        );
    }

    #[test]
    fn a_payload_is_kept_unparsed() {
        // The parser must not know what a Lfo is -- that is the registry's
        // job, one layer up.
        let doc = parse(MINIMAL).expect("parses");
        let payload = doc.entities[0].components.get("Lfo").expect("present");
        let text = ron::to_string(payload).expect("a payload re-serializes");
        assert!(text.contains("8"), "the payload survived as data: {text}");
    }

    #[test]
    fn missing_maps_default_to_empty() {
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "bare")])"#)
            .expect("parses");
        assert!(doc.entities[0].components.is_empty());
        assert!(doc.entities[0].wires.is_empty());
    }

    #[test]
    fn a_syntax_error_is_reported_not_panicked() {
        let error = parse("Project(version: 1, entities: [").expect_err("must fail");
        assert!(matches!(error, ParseError::Ron(_)), "got {error:?}");
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let error = parse(r#"Project(version: 99, entities: [])"#).expect_err("must fail");
        assert_eq!(error, ParseError::UnsupportedVersion(99));
    }

    #[test]
    fn a_duplicate_id_rejects_the_document() {
        // Spec §4.3: nothing in the document can be resolved unambiguously,
        // so this is a whole-reload failure rather than a per-item one.
        let error = parse(
            r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "a")])"#,
        )
        .expect_err("must fail");
        assert_eq!(error, ParseError::DuplicateId("a".to_string()));
    }

    #[test]
    fn an_empty_document_is_valid() {
        let doc = parse("Project(version: 1, entities: [])").expect("parses");
        assert!(doc.entities.is_empty());
    }
}
