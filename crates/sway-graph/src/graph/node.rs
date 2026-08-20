//! The node container: one reflected value with three nested parts.
//!
//! Design D3 — a node kind is ONE `#[derive(Reflect)]` struct with exactly the
//! fields `inlets`, `state` and `outlets`. An absent part is `()`, which
//! `bevy_reflect` implements `Reflect` for, so nothing in the tick, the
//! serializer or the editor ever unwraps an `Option` for a missing part.

use core::fmt;
use std::collections::HashMap;

use bevy_reflect::{GetPath, PartialReflect, Reflect};

/// Which of a node kind's three parts a path is relative to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Part {
    /// Values the node consumes. Authorable, serialized, written by edges.
    Inlets,
    /// Memory that persists between evaluations. Never serialized.
    State,
    /// Values other nodes may consume. Never serialized.
    Outlets,
}

impl Part {
    /// The field name this part occupies on a node kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inlets => "inlets",
            Self::State => "state",
            Self::Outlets => "outlets",
        }
    }

    /// Every part, in declaration order.
    pub const ALL: [Part; 3] = [Part::Inlets, Part::State, Part::Outlets];
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One node: its kind, the annotations a surface hung on it, and the reflected
/// value holding its three parts.
///
/// `value` is the node kind's own struct — `Oscillator`, not a wrapper — so a
/// node's logic can be written and tested as a plain Rust type.
pub struct Node {
    kind: &'static str,
    /// Annotations, keyed by name and holding a value of any registered type.
    /// The graph never reads a key and never acts on a write: an annotation is
    /// where a surface parks its own state (the editor's canvas position under
    /// `"pos"`), not a node value.
    metadata: HashMap<String, Box<dyn PartialReflect>>,
    value: Box<dyn Reflect>,
}

impl Node {
    /// Builds a node around an already-constructed node-kind value, with no
    /// annotations.
    ///
    /// `kind` is the value's reflected type path
    /// (`bevy_reflect::TypePath::type_path`). It is stored rather than looked
    /// up so a node can be described without a type registry in hand.
    pub fn new(kind: &'static str, value: Box<dyn Reflect>) -> Self {
        Self {
            kind,
            metadata: HashMap::new(),
            value,
        }
    }

    /// Builds a node from a concrete node-kind value.
    pub fn of<T: Reflect + bevy_reflect::TypePath>(value: T) -> Self {
        Self::new(T::type_path(), Box::new(value))
    }

    /// The reflected type path of this node's kind.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The node's annotations. The graph does not interpret any of them.
    pub fn metadata(&self) -> &HashMap<String, Box<dyn PartialReflect>> {
        &self.metadata
    }

    /// The node's annotations, mutably. Writing one is not a node change, so
    /// this deliberately does not dirty anything.
    pub fn metadata_mut(&mut self) -> &mut HashMap<String, Box<dyn PartialReflect>> {
        &mut self.metadata
    }

    /// The whole node-kind value.
    pub fn value(&self) -> &dyn Reflect {
        self.value.as_ref()
    }

    /// The whole node-kind value, mutably.
    pub fn value_mut(&mut self) -> &mut dyn Reflect {
        self.value.as_mut()
    }

    /// One of the three parts, whole. `None` only when the node-kind type does
    /// not have that field — which `register_node_kind` refuses to register.
    pub fn part(&self, part: Part) -> Option<&dyn PartialReflect> {
        self.value.as_ref().reflect_path(part.as_str()).ok()
    }

    /// One of the three parts, whole and mutable.
    pub fn part_mut(&mut self, part: Part) -> Option<&mut dyn PartialReflect> {
        self.value.as_mut().reflect_path_mut(part.as_str()).ok()
    }

    /// Consumes the node, returning its value.
    pub fn into_value(self) -> Box<dyn Reflect> {
        self.value
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut keys: Vec<&str> = self.metadata.keys().map(String::as_str).collect();
        keys.sort_unstable();
        f.debug_struct("Node")
            .field("kind", &self.kind)
            .field("metadata", &keys)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Sink};
    use bevy_math::Vec2;

    #[test]
    fn every_part_is_addressable_including_an_empty_one() {
        // `Sink` has no state: its `state` field is `()`. The point of D3 is
        // that it is addressed exactly like a populated part.
        let node = Node::of(Sink::default());
        for part in Part::ALL {
            assert!(node.part(part).is_some(), "{part} must resolve");
        }
        assert_eq!(
            node.part(Part::State).unwrap().reflect_type_path(),
            "()",
            "an absent part is the unit type, not a missing field"
        );
    }

    #[test]
    fn a_node_reports_its_kind() {
        let node = Node::of(Counter::default());
        assert_eq!(node.kind(), "sway_graph::graph::testing::Counter");
    }

    #[test]
    fn a_new_node_carries_no_annotations() {
        // No key is required to be present, and none is privileged.
        let node = Node::of(Counter::default());
        assert!(node.metadata().is_empty());
    }

    #[test]
    fn an_annotation_reads_back_as_the_type_it_was_written_with() {
        // The point of holding annotations reflectively rather than as strings:
        // the reader downcasts, it does not parse.
        let mut node = Node::of(Counter::default());
        node.metadata_mut()
            .insert("pos".into(), Box::new(Vec2::new(3.0, 4.0)));
        node.metadata_mut().insert("flag".into(), Box::new(true));

        let pos = node.metadata()["pos"]
            .try_downcast_ref::<Vec2>()
            .expect("still a Vec2");
        assert_eq!(*pos, Vec2::new(3.0, 4.0));
        assert_eq!(
            node.metadata()["flag"].try_downcast_ref::<bool>(),
            Some(&true)
        );
    }
}
