//! The legality rule (design D5).
//!
//! ```text
//! outlet type S -> inlet type D is legal iff
//!   D == S            direct
//!   D == Option<S>    optional inlet
//!   D == Vec<S>       variadic inlet, and multiple edges may land here
//! ```
//!
//! Decided once, at connect time, from reflected type information — never at
//! evaluation (`graph`: An edge addresses fields by path).
//!
//! Nothing here needs a type registry: a node holds a concrete value, so the
//! type at a path is `get_represented_type_info()` on the resolved field.

use core::any::TypeId;

use bevy_reflect::{PartialReflect, TypeInfo};

use crate::graph::edge::Compat;

/// The `TypeId` of `S` when `info` describes `Option<S>`.
fn option_inner(info: &TypeInfo) -> Option<TypeId> {
    let TypeInfo::Enum(enum_info) = info else {
        return None;
    };
    // Match `Option` structurally rather than by name where possible, but the
    // path check keeps a user enum that happens to have a one-field `Some`
    // variant from being treated as optional.
    if !enum_info.type_path().starts_with("core::option::Option<") {
        return None;
    }
    let variant = enum_info.variant("Some")?;
    let bevy_reflect::enums::VariantInfo::Tuple(tuple) = variant else {
        return None;
    };
    Some(tuple.field_at(0)?.type_id())
}

/// The `TypeId` of `S` when `info` describes a list of `S` (`Vec<S>` and
/// anything else `bevy_reflect` reports as a list).
fn list_item(info: &TypeInfo) -> Option<TypeId> {
    match info {
        TypeInfo::List(list) => Some(list.item_ty().id()),
        _ => None,
    }
}

/// Applies the rule to two reflected types.
///
/// `None` means the connection is illegal.
pub fn compatibility(src: &TypeInfo, dst: &TypeInfo) -> Option<Compat> {
    let source = src.type_id();
    // `D == S` is checked first, so `Vec<f32> -> Vec<f32>` is a direct copy
    // rather than a variadic slot.
    if dst.type_id() == source {
        return Some(Compat::Direct);
    }
    if option_inner(dst) == Some(source) {
        return Some(Compat::Optional);
    }
    if list_item(dst) == Some(source) {
        return Some(Compat::Variadic);
    }
    None
}

/// Applies the rule to two resolved values, which is what `connect` has in
/// hand. `None` when either value has no static type, or the pair is illegal.
pub fn compatibility_of_values(
    src: &dyn PartialReflect,
    dst: &dyn PartialReflect,
) -> Option<Compat> {
    let src = src.get_represented_type_info()?;
    let dst = dst.get_represented_type_info()?;
    compatibility(src, dst)
}

/// Whether a value carries nothing — a marker/protocol source (design D6).
///
/// Measured with `size_of_val` through the vtable rather than by naming marker
/// types, so `sway-graph` never learns what a `SceneMaterial` is.
pub fn is_valueless(value: &dyn PartialReflect) -> bool {
    core::mem::size_of_val(value) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::Vec2;
    use bevy_reflect::Typed;

    fn compat<S: Typed, D: Typed>() -> Option<Compat> {
        compatibility(S::type_info(), D::type_info())
    }

    #[test]
    fn the_same_type_is_a_direct_connection() {
        assert_eq!(compat::<f32, f32>(), Some(Compat::Direct));
        assert_eq!(compat::<Vec2, Vec2>(), Some(Compat::Direct));
    }

    #[test]
    fn an_option_of_the_source_is_an_optional_inlet() {
        assert_eq!(compat::<f32, Option<f32>>(), Some(Compat::Optional));
    }

    #[test]
    fn a_vec_of_the_source_is_a_variadic_inlet() {
        assert_eq!(compat::<f32, Vec<f32>>(), Some(Compat::Variadic));
    }

    #[test]
    fn a_matching_vec_on_both_sides_stays_direct() {
        assert_eq!(compat::<Vec<f32>, Vec<f32>>(), Some(Compat::Direct));
    }

    #[test]
    fn mismatched_types_are_illegal() {
        assert_eq!(compat::<f32, u32>(), None);
        assert_eq!(compat::<f32, Option<u32>>(), None);
        assert_eq!(compat::<f32, Vec<u32>>(), None);
        assert_eq!(compat::<Vec2, f32>(), None);
    }

    #[test]
    fn a_zero_sized_value_is_valueless() {
        assert!(is_valueless(&()));
        assert!(!is_valueless(&0.0f32));
    }
}
