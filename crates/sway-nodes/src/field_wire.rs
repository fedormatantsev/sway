//! One macro for the twenty-odd near-identical value wires.
//!
//! A wire is a `Relationship` on the consumer, a `RelationshipTarget` on the
//! producer, and a `propagate` that writes one field. All three are mechanical;
//! the only per-wire facts are the two types, the document name, which field of
//! the target to write, and how to get the value out of the source.
//!
//! `set_if_neq` is not optional. `get_mut` marks `Changed` unconditionally, and
//! `Changed<T>` is the whole dirty story downstream (architecture §7), so a wire
//! that writes an equal value silently recooks everything it feeds.

/// ```ignore
/// field_wire!(
///     /// Doc comment for the wire type.
///     TranslationFrom / DrivesTranslation,
///     Vec3Out => Transform,
///     "translation",
///     |t| &mut t.translation,
///     |s| s.0
/// );
/// ```
macro_rules! field_wire {
    (
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $name:literal,
        |$t:ident| $field:expr,
        |$s:ident| $value:expr
    ) => {
        $(#[$attr])*
        #[derive(bevy::prelude::Component)]
        #[relationship(relationship_target = $drives)]
        pub struct $wire(#[entities] pub bevy::prelude::Entity);

        #[derive(bevy::prelude::Component)]
        #[relationship_target(relationship = $wire)]
        pub struct $drives(Vec<bevy::prelude::Entity>);

        impl sway_graph::Wire for $wire {
            type Source = $src;
            type Target = $dst;
            const NAME: &'static str = $name;

            fn propagate(src: &$src, dst: bevy_ecs::change_detection::Mut<$dst>) {
                let $s = src;
                let value = $value;
                let mut field = dst.map_unchanged(|$t| $field);
                bevy_ecs::change_detection::DetectChangesMut::set_if_neq(&mut field, value);
            }
        }
    };
}

pub(crate) use field_wire;
