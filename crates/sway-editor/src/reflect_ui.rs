//! Reflection is the editor's schema.
//!
//! Design D11: the editor keeps no second description of nodes, sockets, edges
//! or field kinds. Every question the widget layer used to ask a hand-written
//! view type -- what sockets does this node have, what control does this field
//! want, how does a typed edit parse back -- is answered here directly from
//! `bevy_reflect`'s own `TypeInfo`, with the graph's `part_type` lookup as
//! the entry point.
//!
//! Nothing in this module allocates a description that outlives the call: the
//! widgets cache what they *paint* (masonry's retained model), and the
//! `&'static TypeInfo` they hold onto is reflection's, not a copy of it.

use std::any::TypeId;

use bevy_reflect::enums::{DynamicEnum, DynamicVariant};
use bevy_reflect::{PartialReflect, ReflectRef, TypeInfo, TypeRegistry};
use sway_graph::graph::{Part, node_kind_type_id, part_type};

/// One field a node kind declares in one of its parts.
///
/// `path` is relative to the part -- `"frequency"`, never `"inlets.frequency"`
/// -- which is exactly what an [`Edge`](sway_graph::graph::Edge) stores and
/// what [`EditorEdit::SetField`](crate::edit::EditorEdit) takes.
#[derive(Clone, Debug)]
pub struct PartField {
    /// Field path relative to the part.
    pub path: String,
    /// The field's declared reflected type, when it has static type info.
    pub info: Option<&'static TypeInfo>,
}

/// Every field a node kind declares in `part`, in declaration order.
///
/// This is what makes an *unconnected* inlet a socket: the answer comes from
/// the kind's declared schema, never from the edge list.
pub fn part_fields(registry: &TypeRegistry, kind: &str, part: Part) -> Vec<PartField> {
    let Some(type_id) = node_kind_type_id(registry, kind) else {
        return Vec::new();
    };
    let Some(info) = part_type(registry, type_id, part) else {
        return Vec::new();
    };
    fields_of(info)
}

/// The fields of a reflected struct or tuple struct. An empty `()` part, or
/// anything else, has none.
pub fn fields_of(info: &'static TypeInfo) -> Vec<PartField> {
    match info {
        TypeInfo::Struct(s) => s
            .iter()
            .map(|field| PartField {
                path: field.name().to_string(),
                info: field.type_info(),
            })
            .collect(),
        TypeInfo::TupleStruct(t) => t
            .iter()
            .enumerate()
            .map(|(index, field)| PartField {
                path: index.to_string(),
                info: field.type_info(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether this inlet accepts several connections at once.
///
/// `sway-graph` decides that by matching the destination type as
/// `TypeInfo::List` (`D == Vec<S>`), so this asks the same question of the same
/// type information rather than keeping a second list of variadic field names.
pub fn is_variadic(info: &TypeInfo) -> bool {
    matches!(info, TypeInfo::List(_))
}

/// Whether this field wants a checkbox.
pub fn is_bool(info: &TypeInfo) -> bool {
    info.type_id() == TypeId::of::<bool>()
}

/// Every variant name of an enum field, or `None` when this type is not an
/// enum the editor offers a control for.
///
/// `Option<T>` is deliberately excluded even though it *is* an enum: an
/// optional inlet's `Some`/`None` is decided by whether something is connected
/// to it, and offering a variant picker would let the user author a shape the
/// next propagate immediately overwrites. It falls through to the read-only
/// control instead, which is what the spec asks for a type with no control.
pub fn enum_variants(info: &TypeInfo) -> Option<Vec<String>> {
    let TypeInfo::Enum(e) = info else {
        return None;
    };
    if e.type_path().starts_with("core::option::Option<") {
        return None;
    }
    Some(e.iter().map(|variant| variant.name().to_string()).collect())
}

/// Whether this field commits through a text box (as opposed to a checkbox, a
/// variant picker, or no control at all).
pub fn is_text_field(info: &TypeInfo) -> bool {
    let id = info.type_id();
    id == TypeId::of::<f32>()
        || id == TypeId::of::<f64>()
        || is_integer(id)
        || id == TypeId::of::<String>()
        || id == TypeId::of::<bevy_math::Vec2>()
        || id == TypeId::of::<bevy_math::Vec3>()
}

/// Whether the editor has *any* control for this field. A field that has none
/// is shown read-only rather than omitted (`editor`: "A field with no control
/// is shown read-only").
pub fn has_control(info: &TypeInfo) -> bool {
    is_bool(info) || enum_variants(info).is_some() || is_text_field(info)
}

fn is_integer(id: TypeId) -> bool {
    id == TypeId::of::<i8>()
        || id == TypeId::of::<i16>()
        || id == TypeId::of::<i32>()
        || id == TypeId::of::<i64>()
        || id == TypeId::of::<isize>()
        || id == TypeId::of::<u8>()
        || id == TypeId::of::<u16>()
        || id == TypeId::of::<u32>()
        || id == TypeId::of::<u64>()
        || id == TypeId::of::<usize>()
}

/// Converts a committed string into a value of the field's **declared type**.
///
/// The control is the only thing that knows what it produced, so this is the
/// editor's job, not the graph's: `Graph::set_field` takes a reflected value
/// and enumerates no set of types a write may carry.
///
/// `None` means "do not send anything": either the type has no control, or the
/// text does not parse, in which case the field simply snaps back on the next
/// read. A silently-dropped write and a write of the wrong value are both
/// worse than no write.
///
/// An out-of-range integer **saturates** rather than being dropped. A dropped
/// write looks identical to a UI that ignored the keystroke, because the
/// inspector re-reads the unchanged field and snaps back.
pub fn coerce_field(info: &TypeInfo, text: &str) -> Option<Box<dyn PartialReflect>> {
    let id = info.type_id();
    if id == TypeId::of::<f32>() {
        return text
            .trim()
            .parse::<f64>()
            .ok()
            .map(|number| Box::new(number as f32) as Box<dyn PartialReflect>);
    }
    if id == TypeId::of::<f64>() {
        return text
            .trim()
            .parse::<f64>()
            .ok()
            .map(|number| Box::new(number) as Box<dyn PartialReflect>);
    }
    if is_integer(id) {
        let number = text.trim().parse::<i64>().ok()?;
        return narrow_integer(id, number);
    }
    if id == TypeId::of::<bool>() {
        return Some(Box::new(text.trim() == "true"));
    }
    if id == TypeId::of::<String>() {
        return Some(Box::new(text.to_string()));
    }
    if id == TypeId::of::<bevy_math::Vec2>() {
        let [x, y] = parse_components::<2>(text)?;
        return Some(Box::new(bevy_math::Vec2::new(x, y)));
    }
    if id == TypeId::of::<bevy_math::Vec3>() {
        let [x, y, z] = parse_components::<3>(text)?;
        return Some(Box::new(bevy_math::Vec3::new(x, y, z)));
    }
    if enum_variants(info).is_some() {
        // A unit variant, by name. `try_apply` on an enum switches variant.
        return Some(Box::new(DynamicEnum::new(
            text.trim().to_string(),
            DynamicVariant::Unit,
        )));
    }
    None
}

/// Boxes `value` as the concrete integer type `id` names, saturating rather
/// than wrapping.
///
/// The cast must be explicit: `-1i64 as u32` wraps to `u32::MAX`, which would
/// be a far worse answer than `0`.
fn narrow_integer(id: TypeId, value: i64) -> Option<Box<dyn PartialReflect>> {
    macro_rules! narrow {
        ($($t:ty),+ $(,)?) => {$(
            if id == TypeId::of::<$t>() {
                let saturated = <$t>::try_from(value)
                    .unwrap_or(if value < 0 { <$t>::MIN } else { <$t>::MAX });
                return Some(Box::new(saturated));
            }
        )+};
    }
    narrow!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
    None
}

/// Parses exactly `N` comma-separated floats, or nothing. Every component must
/// parse and the count must match, so a typo becomes no write rather than a
/// different vector.
fn parse_components<const N: usize>(text: &str) -> Option<[f32; N]> {
    let mut parts = text.split(',');
    let mut out = [0.0; N];
    for slot in out.iter_mut() {
        *slot = parts.next()?.trim().parse::<f32>().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// How a field's current value is displayed. Anything unrecognised falls back
/// to its debug form, which is the signal that the type wants a control.
pub fn format_value(value: &dyn PartialReflect) -> String {
    if let Some(v) = value.try_downcast_ref::<f32>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<f64>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<bool>() {
        return v.to_string();
    }
    if let Some(v) = value.try_downcast_ref::<String>() {
        return v.clone();
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec2>() {
        return format!("{:.2}, {:.2}", v.x, v.y);
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec3>() {
        return format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z);
    }
    for parse in [
        integer_text::<i8>,
        integer_text::<i16>,
        integer_text::<i32>,
        integer_text::<i64>,
        integer_text::<isize>,
        integer_text::<u8>,
        integer_text::<u16>,
        integer_text::<u32>,
        integer_text::<u64>,
        integer_text::<usize>,
    ] {
        if let Some(text) = parse(value) {
            return text;
        }
    }
    if let ReflectRef::Enum(e) = value.reflect_ref() {
        return e.variant_name().to_string();
    }
    format!("{value:?}")
}

fn integer_text<T: core::fmt::Display + Clone + 'static>(
    value: &dyn PartialReflect,
) -> Option<String> {
    value.try_downcast_ref::<T>().map(ToString::to_string)
}

/// The last segment of a reflected type path, with generic arguments shortened
/// the same way. What the palette, the canvas and the tree display instead of
/// `my_crate::nodes::Oscillator`.
pub fn short_type_name(path: &str) -> String {
    fn last_segment(s: &str) -> &str {
        match s.rfind("::") {
            Some(i) => &s[i + 2..],
            None => s,
        }
    }

    let mut out = String::with_capacity(path.len());
    let mut segment_start = 0;
    for (i, ch) in path.char_indices() {
        if matches!(ch, '<' | '>' | ',' | ' ') {
            out.push_str(last_segment(&path[segment_start..i]));
            out.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
    out.push_str(last_segment(&path[segment_start..]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_kinds::{Gate, Mixer, Source, registry};

    #[test]
    fn a_kinds_inlets_come_from_its_declared_schema_not_its_edges() {
        let registry = registry();
        let fields = part_fields(&registry, Source::path(), Part::Inlets);
        let paths: Vec<&str> = fields.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["level", "label", "enabled", "shape"]);
    }

    #[test]
    fn outlets_are_read_the_same_way() {
        let registry = registry();
        let fields = part_fields(&registry, Source::path(), Part::Outlets);
        let paths: Vec<&str> = fields.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["out", "pair"]);
    }

    #[test]
    fn an_empty_part_declares_no_fields() {
        let registry = registry();
        assert!(part_fields(&registry, Gate::path(), Part::State).is_empty());
    }

    #[test]
    fn an_unregistered_kind_declares_no_fields() {
        let registry = registry();
        assert!(part_fields(&registry, "nothing::Like::This", Part::Inlets).is_empty());
    }

    #[test]
    fn a_control_is_chosen_from_the_reflected_type() {
        let registry = registry();
        let fields = part_fields(&registry, Source::path(), Part::Inlets);
        let field = |name: &str| {
            fields
                .iter()
                .find(|f| f.path == name)
                .and_then(|f| f.info)
                .expect("declared field with type info")
        };

        assert!(is_text_field(field("level")));
        assert!(is_text_field(field("label")));
        assert!(is_bool(field("enabled")));
        assert_eq!(
            enum_variants(field("shape")),
            Some(vec!["Sine".to_string(), "Saw".to_string()])
        );
    }

    #[test]
    fn a_type_with_no_control_is_reported_as_such_rather_than_dropped() {
        let registry = registry();
        let fields = part_fields(&registry, Mixer::path(), Part::Inlets);
        let terms = fields
            .iter()
            .find(|f| f.path == "terms")
            .expect("the variadic inlet is still declared");
        assert!(
            !has_control(terms.info.expect("Vec<f32> has type info")),
            "a Vec inlet has no editing control -- it is shown read-only",
        );
    }

    #[test]
    fn an_option_inlet_is_not_offered_as_a_variant_picker() {
        let registry = registry();
        let fields = part_fields(&registry, Gate::path(), Part::Inlets);
        let gate = fields
            .iter()
            .find(|f| f.path == "gate")
            .and_then(|f| f.info)
            .expect("Option<f32> has type info");
        assert_eq!(enum_variants(gate), None);
        assert!(!has_control(gate));
    }

    #[test]
    fn parsing_is_decided_by_the_field_type() {
        let registry = registry();
        let fields = part_fields(&registry, Source::path(), Part::Inlets);
        let info = |name: &str| {
            fields
                .iter()
                .find(|f| f.path == name)
                .and_then(|f| f.info)
                .unwrap()
        };

        // Each value arrives as the field's *declared* type, not as some
        // intermediate the graph would then have to narrow.
        let level = coerce_field(info("level"), " 0.75 ").expect("parses");
        assert_eq!(level.try_downcast_ref::<f32>(), Some(&0.75));
        assert!(coerce_field(info("level"), "nope").is_none());

        let enabled = coerce_field(info("enabled"), "true").expect("parses");
        assert_eq!(enabled.try_downcast_ref::<bool>(), Some(&true));

        let label = coerce_field(info("label"), "hi").expect("parses");
        assert_eq!(label.try_downcast_ref::<String>(), Some(&"hi".to_string()));

        // A unit enum variant arrives as a `DynamicEnum` naming it: applying
        // one is what switches the variant.
        let shape = coerce_field(info("shape"), "Saw").expect("parses");
        assert_eq!(shape.reflect_type_path(), "bevy_reflect::DynamicEnum");
    }

    #[test]
    fn an_out_of_range_integer_saturates_rather_than_being_dropped() {
        // A dropped write looks identical to a UI that ignored the keystroke,
        // because the inspector re-reads the unchanged field and snaps back.
        use bevy_reflect::Typed;

        let info = <u8 as Typed>::type_info();
        let high = coerce_field(info, "9999").expect("parses");
        assert_eq!(high.try_downcast_ref::<u8>(), Some(&u8::MAX));

        let low = coerce_field(info, "-5").expect("parses");
        assert_eq!(low.try_downcast_ref::<u8>(), Some(&0));

        let ok = coerce_field(info, "7").expect("parses");
        assert_eq!(ok.try_downcast_ref::<u8>(), Some(&7));
    }

    #[test]
    fn a_value_that_is_already_the_fields_type_passes_straight_through() {
        use bevy_reflect::Typed;

        let vec2 = coerce_field(<bevy_math::Vec2 as Typed>::type_info(), "1.5, -2.0")
            .expect("parses");
        assert_eq!(
            vec2.try_downcast_ref::<bevy_math::Vec2>(),
            Some(&bevy_math::Vec2::new(1.5, -2.0)),
        );

        let float = coerce_field(<f64 as Typed>::type_info(), "0.25").expect("parses");
        assert_eq!(float.try_downcast_ref::<f64>(), Some(&0.25));
    }

    #[test]
    fn short_type_name_strips_module_paths() {
        assert_eq!(short_type_name("sway_nodes::lfo::Lfo"), "Lfo");
        assert_eq!(short_type_name("bevy::asset::Handle<Mesh>"), "Handle<Mesh>");
    }

    #[test]
    fn values_render_through_reflection() {
        assert_eq!(format_value(&0.5f32), "0.500");
        assert_eq!(format_value(&true), "true");
        assert_eq!(format_value(&7u32), "7");
        assert_eq!(format_value(&bevy_math::Vec2::new(1.0, 2.0)), "1.00, 2.00");
    }
}
