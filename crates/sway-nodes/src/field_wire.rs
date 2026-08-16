//! One macro for the near-identical value wires.
//!
//! A wire is a `Relationship` on the consumer, a `RelationshipTarget` on the
//! producer, and reflected field copy from outlet tuple field `0` onto a
//! named inlet field. Identity is the type path, not a short name.

/// ```ignore
/// field_wire!(
///     /// Doc comment for the wire type.
///     TranslationFrom / DrivesTranslation,
///     Vec3Out => Transform,
///     "translation"
/// );
/// ```
macro_rules! field_wire {
    (
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $target_path:literal
    ) => {
        $(#[$attr])*
        #[derive(bevy::prelude::Component, bevy::prelude::Reflect, Clone, Copy)]
        #[relationship(relationship_target = $drives)]
        #[reflect(Component, Wire)]
        pub struct $wire(#[entities] pub bevy::prelude::Entity);

        #[derive(bevy::prelude::Component)]
        #[relationship_target(relationship = $wire)]
        pub struct $drives(Vec<bevy::prelude::Entity>);

        impl From<bevy::prelude::Entity> for $wire {
            fn from(entity: bevy::prelude::Entity) -> Self {
                Self(entity)
            }
        }

        impl sway_graph::Wire for $wire {
            fn producer(&self) -> bevy::prelude::Entity {
                self.0
            }

            fn source_type(&self) -> std::any::TypeId {
                std::any::TypeId::of::<$src>()
            }

            fn target_type(&self) -> std::any::TypeId {
                std::any::TypeId::of::<$dst>()
            }

            fn source_path(&self) -> &'static str {
                "0"
            }

            fn target_path(&self) -> &'static str {
                $target_path
            }
        }
    };
}

pub(crate) use field_wire;
