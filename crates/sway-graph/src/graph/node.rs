//! The node container: one reflected value with three nested parts.
//!
//! Design D3 — a node kind is ONE `#[derive(Reflect)]` struct with exactly the
//! fields `inlets`, `state` and `outlets`. An absent part is `()`, which
//! `bevy_reflect` implements `Reflect` for, so nothing in the tick, the
//! serializer or the editor ever unwraps an `Option` for a missing part.

use core::fmt;

use bevy_math::Vec2;
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

/// One node: its kind, where the editor draws it, and the reflected value
/// holding its three parts.
///
/// `value` is the node kind's own struct — `Oscillator`, not a wrapper — so a
/// node's logic can be written and tested as a plain Rust type.
pub struct Node {
    kind: &'static str,
    pos: Vec2,
    value: Box<dyn Reflect>,
}

impl Node {
    /// Builds a node around an already-constructed node-kind value.
    ///
    /// `kind` is the value's reflected type path
    /// (`bevy_reflect::TypePath::type_path`). It is stored rather than looked
    /// up so a node can be described without a type registry in hand.
    pub fn new(kind: &'static str, pos: Vec2, value: Box<dyn Reflect>) -> Self {
        Self { kind, pos, value }
    }

    /// Builds a node from a concrete node-kind value.
    pub fn of<T: Reflect + bevy_reflect::TypePath>(pos: Vec2, value: T) -> Self {
        Self::new(T::type_path(), pos, Box::new(value))
    }

    /// The reflected type path of this node's kind.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Where the editor draws this node, in graph-canvas space.
    pub fn pos(&self) -> Vec2 {
        self.pos
    }

    /// Moves the node. Returns `true` when the position actually changed —
    /// callers use that to decide whether to dirty (architecture §7: never
    /// write an equal value).
    pub fn set_pos(&mut self, pos: Vec2) -> bool {
        if self.pos == pos {
            return false;
        }
        self.pos = pos;
        true
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
        f.debug_struct("Node")
            .field("kind", &self.kind)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Sink};

    #[test]
    fn every_part_is_addressable_including_an_empty_one() {
        // `Sink` has no state: its `state` field is `()`. The point of D3 is
        // that it is addressed exactly like a populated part.
        let node = Node::of(Vec2::ZERO, Sink::default());
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
    fn a_node_reports_its_kind_and_position() {
        let node = Node::of(Vec2::new(3.0, 4.0), Counter::default());
        assert_eq!(node.kind(), "sway_graph::graph::testing::Counter");
        assert_eq!(node.pos(), Vec2::new(3.0, 4.0));
    }

    #[test]
    fn moving_to_the_same_position_reports_no_change() {
        let mut node = Node::of(Vec2::new(1.0, 1.0), Counter::default());
        assert!(!node.set_pos(Vec2::new(1.0, 1.0)));
        assert!(node.set_pos(Vec2::new(1.0, 2.0)));
    }
}
