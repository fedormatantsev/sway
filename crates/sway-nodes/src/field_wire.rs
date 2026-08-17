//! One macro for the near-identical value wires.
//!
//! A wire is a `Relationship` on the consumer, a `RelationshipTarget` on the
//! producer, and reflected field copy from outlet tuple field `0` onto a
//! named inlet field. Identity is the type path, not a short name.
//!
//! **Exported, not crate-private.** Design D6 puts `SpriteMaterial` and
//! `FrameSequence` in `sway-runtime` while `sway-nodes` keeps the value
//! outlets, so their wires have to be declared from the other side of a crate
//! boundary. Everything the macro expands to therefore names `$crate` or a
//! `bevy::`-rooted path — a caller only needs `bevy` and `sway-graph`, which is
//! exactly what a crate declaring a wire already depends on. The two hook
//! helpers are `pub` for the same reason: the `supplies_target` arm expands to
//! calls into them.

/// The connect half of design D5: a material wire owns the
/// `MeshMaterial3d<M>` it writes into, because that component is typed per
/// material kind and a mesh node therefore cannot supply one up front without
/// deciding the kind for every wire that will ever reach it.
///
/// `if_new` rather than a plain insert so that re-pointing an existing wire at
/// another producer does not throw away the handle already delivered — the
/// relationship component is immutable, so a rewire is an insert on top of an
/// insert, and clobbering here would dirty the mesh for a tick with the
/// engine's fallback material.
pub fn supply_target<T: bevy::prelude::Component + Default>(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    world
        .commands()
        .entity(context.entity)
        .try_insert_if_new(T::default());
}

/// The disconnect half of D5.
///
/// `on_discard` rather than `on_remove` because `sway_graph::register_wire_type`
/// already owns every wire's `on_add` and `on_remove` slots for its topology
/// bookkeeping, and `ComponentHooks::on_remove` panics on a second registration;
/// `on_insert` and `on_discard` are the two slots Bevy 0.19's derive *composes*
/// — it appends the relationship's own hook after ours rather than replacing it.
///
/// `on_discard` also fires when a wire is overwritten in place, which a rewire
/// is: the relationship component is immutable, so re-pointing a wire at another
/// producer is an insert over an insert. Hence the deferred re-check — by the
/// time the command runs, a rewire has already put the new wire back, so `W`
/// present means "not actually disconnected" and the material stays put. A
/// despawn leaves no entity at all, which the `Ok` guard covers.
pub fn withdraw_target<W: bevy::prelude::Component, T: bevy::prelude::Component>(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let entity = context.entity;
    world
        .commands()
        .queue(move |world: &mut bevy::prelude::World| {
            let Ok(mut entity) = world.get_entity_mut(entity) else {
                return;
            };
            if entity.contains::<W>() {
                return;
            }
            entity.remove::<T>();
        });
}

/// ```ignore
/// field_wire!(
///     /// Doc comment for the wire type.
///     TranslationFrom / DrivesTranslation,
///     Vec3Out => Transform,
///     "translation"
/// );
/// ```
///
/// A trailing `supplies_target` asks for the D5 behaviour: the wire inserts
/// its own target component on connect and removes it on disconnect, instead
/// of expecting the consumer to have been born with one.
///
/// ```ignore
/// field_wire!(
///     /// Doc comment for the wire type.
///     MaterialFrom / DrivesMaterial,
///     MaterialOut => MeshMaterial3d<StandardMaterial>,
///     "0",
///     supplies_target
/// );
/// ```
///
/// Extending the macro rather than hand-writing `MaterialFrom` outside it:
/// every material kind needs exactly this, differing only in `M`, so the hooks
/// are generic functions and the two public arms differ by one attribute.
/// Hooks rather than observers because a hook travels with the type — a wire
/// that arrives from a loaded document works whether or not anyone remembered
/// to register anything, which is what the "hand-authored document connects the
/// same way" scenario asks for.
///
/// A `""` target path declares a wire that copies no field, exactly as
/// `sway_graph`'s own `impl Wire for ChildOf` does: `propagate_field_copy`
/// returns immediately on an empty target path, so the wire is pure topology.
/// The source and target *types* still define connect legality, which is the
/// point — see `ColorRunFrom` in `sway-runtime` for why a whole-struct outlet
/// wants that rather than a copy.
///
/// The invoking crate must have `ReflectWire` in scope, since
/// `#[reflect(Component, Wire)]` expands to it by name.
#[macro_export]
macro_rules! field_wire {
    (
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $target_path:literal
    ) => {
        $crate::field_wire!(
            @define []
            $(#[$attr])*
            $wire / $drives,
            $src => $dst,
            $target_path
        );
    };

    (
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $target_path:literal,
        supplies_target
    ) => {
        $crate::field_wire!(
            @define [#[component(
                on_insert = Self::supply_target,
                on_discard = Self::withdraw_target,
            )]]
            $(#[$attr])*
            $wire / $drives,
            $src => $dst,
            $target_path
        );

        // Monomorphic wrappers so the attribute above can name a plain path.
        impl $wire {
            fn supply_target(
                world: bevy::ecs::world::DeferredWorld,
                context: bevy::ecs::lifecycle::HookContext,
            ) {
                $crate::field_wire::supply_target::<$dst>(world, context);
            }

            fn withdraw_target(
                world: bevy::ecs::world::DeferredWorld,
                context: bevy::ecs::lifecycle::HookContext,
            ) {
                $crate::field_wire::withdraw_target::<Self, $dst>(world, context);
            }
        }
    };

    (
        @define [$(#[$hook:meta])*]
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $target_path:literal
    ) => {
        $(#[$attr])*
        #[derive(bevy::prelude::Component, bevy::prelude::Reflect, Clone, Copy)]
        #[relationship(relationship_target = $drives)]
        #[reflect(Component, Wire)]
        $(#[$hook])*
        pub struct $wire(#[entities] pub bevy::prelude::Entity);

        #[derive(bevy::prelude::Component)]
        #[relationship_target(relationship = $wire)]
        pub struct $drives(Vec<bevy::prelude::Entity>);

        impl From<bevy::prelude::Entity> for $wire {
            fn from(entity: bevy::prelude::Entity) -> Self {
                Self(entity)
            }
        }

        impl ::sway_graph::Wire for $wire {
            fn producer(&self) -> bevy::prelude::Entity {
                self.0
            }

            fn source_type(&self) -> ::std::any::TypeId {
                ::std::any::TypeId::of::<$src>()
            }

            fn target_type(&self) -> ::std::any::TypeId {
                ::std::any::TypeId::of::<$dst>()
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
