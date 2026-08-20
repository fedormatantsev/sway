//! Path resolution over a node's three parts.
//!
//! Stored paths are relative to the part (design D5): an edge's source path is
//! `"out"`, not `"outlets.out"`. This module is the only place the prefix is
//! added, so the document, the editor and the tick all speak the short form.
//!
//! Resolution goes through `bevy_reflect::GetPath`, so arbitrary depth
//! (`"transform.translation.x"`) and list indices (`"layers[2]"`) work with no
//! extra machinery.

use bevy_reflect::{GetPath, ParsedPath, PartialReflect};

use crate::graph::node::{Node, Part};

/// Builds the absolute reflect path for `relative` within `part`.
///
/// An empty `relative` addresses the whole part. A `relative` that starts with
/// an index access (`"[0]"`) is appended without a separating dot, which is
/// what `bevy_reflect`'s parser expects.
pub fn absolute_path(part: Part, relative: &str) -> String {
    let part = part.as_str();
    if relative.is_empty() {
        return part.to_owned();
    }
    if relative.starts_with('[') || relative.starts_with('.') || relative.starts_with('#') {
        return format!("{part}{relative}");
    }
    format!("{part}.{relative}")
}

/// Parses `relative` within `part` into a reusable [`ParsedPath`].
///
/// Rebuild does this once per edge; the tick then walks the parsed form every
/// tick with no string parsing and no allocation.
pub fn parse(part: Part, relative: &str) -> Option<ParsedPath> {
    ParsedPath::parse(&absolute_path(part, relative)).ok()
}

/// Resolves `relative` within `part` on `node`.
pub fn resolve<'a>(node: &'a Node, part: Part, relative: &str) -> Option<&'a dyn PartialReflect> {
    node.value()
        .reflect_path(absolute_path(part, relative).as_str())
        .ok()
}

/// Resolves `relative` within `part` on `node`, mutably.
pub fn resolve_mut<'a>(
    node: &'a mut Node,
    part: Part,
    relative: &str,
) -> Option<&'a mut dyn PartialReflect> {
    node.value_mut()
        .reflect_path_mut(absolute_path(part, relative).as_str())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Nested};
    use bevy_math::Vec2;

    #[test]
    fn the_resolver_prepends_the_part() {
        assert_eq!(absolute_path(Part::Inlets, "frequency"), "inlets.frequency");
        assert_eq!(absolute_path(Part::Outlets, "out"), "outlets.out");
        assert_eq!(absolute_path(Part::State, ""), "state");
        assert_eq!(absolute_path(Part::Inlets, "[2]"), "inlets[2]");
    }

    #[test]
    fn a_stored_path_stays_short() {
        let node = Node::of(Counter::default());
        // "step", not "inlets.step".
        let value = resolve(&node, Part::Inlets, "step").expect("resolves");
        assert_eq!(value.reflect_type_path(), "f32");
    }

    #[test]
    fn a_nested_field_is_addressable() {
        let mut node = Node::of(Nested::default());
        let field = resolve_mut(&mut node, Part::Inlets, "point.x").expect("resolves");
        field.try_apply(&2.5f32).expect("apply");

        let after = resolve(&node, Part::Inlets, "point").expect("resolves");
        assert_eq!(
            after.reflect_partial_eq(&Vec2::new(2.5, 0.0)),
            Some(true),
            "only the named component is written; the rest is unchanged"
        );
    }

    #[test]
    fn an_unknown_path_resolves_to_nothing() {
        let node = Node::of(Counter::default());
        assert!(resolve(&node, Part::Inlets, "nope").is_none());
    }

    #[test]
    fn a_parsed_path_addresses_the_same_field() {
        let node = Node::of(Counter::default());
        let parsed = parse(Part::Inlets, "step").expect("parses");
        let value = node.value().reflect_path(&parsed).expect("resolves");
        assert_eq!(value.reflect_type_path(), "f32");
    }
}
