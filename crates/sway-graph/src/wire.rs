//! `Wire` — a connection type. Spec §2.1.

use bevy_ecs::change_detection::Mut;
use bevy_ecs::component::{Component, Mutable};
use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::relationship::Relationship;
use bevy_ecs::world::World;
use bevy_transform::components::Transform;

/// A connection. The `Relationship` component lives on the CONSUMER and names
/// the producer; the `RelationshipTarget` on the producer collects consumers.
///
/// Bevy allows one component per type per entity, so "an inlet has at most one
/// source" holds by construction — there is no validation pass for it.
pub trait Wire: Relationship {
    /// The component read on the producer. Also the legality rule the editor
    /// uses: this wire may only originate at an entity that has one.
    type Source: Component;
    /// The component written on the consumer.
    type Target: Component<Mutability = Mutable>;

    /// Display name, for the editor.
    const NAME: &'static str;

    /// The entirety of this connection's behaviour.
    ///
    /// **Must not write an equal value.** `get_mut` marks `Changed<Target>`
    /// unconditionally, and `Changed<T>` is the whole dirty story downstream
    /// (spec §3.4). Use `Mut::map_unchanged(..).set_if_neq(..)`.
    fn propagate(src: &Self::Source, dst: Mut<Self::Target>);
}

/// A wire type's `propagate`, monomorphised and erased so a heterogeneous
/// step list can hold it. `Wire::propagate` takes associated types and so is
/// not object-safe; this is the only erasure the design needs.
pub type PropagateFn = fn(&mut World, Entity, Entity);

/// Hierarchy costs one impl, because `ChildOf` is already a `Relationship`.
///
/// `propagate` is empty because a structural connection carries no per-tick
/// value: its existence IS the state, and Bevy's own hooks maintain
/// `Children`. `Source`/`Target` still define wiring legality for tooling.
impl Wire for ChildOf {
    type Source = Transform;
    type Target = Transform;
    const NAME: &'static str = "parent";

    fn propagate(_: &Transform, _: Mut<Transform>) {}
}

pub fn propagate_of<W: Wire>(world: &mut World, src: Entity, dst: Entity) {
    // `src == dst` cannot happen: Bevy removes self-referential
    // relationships (see tests/relationship_semantics.rs), so this fetch
    // never aliases.
    let Ok([src_ref, mut dst_mut]) = world.get_entity_mut([src, dst]) else {
        return; // producer despawned or consumer missing (Bevy cleaned up the wire)
    };
    let Some(source) = src_ref.get::<W::Source>() else {
        return; // legal transient state during spawn
    };
    let Some(target) = dst_mut.get_mut::<W::Target>() else {
        return;
    };
    W::propagate(source, target);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_wires::{spawn_float, spawn_gain, Gain, GainFrom};
    use bevy_ecs::world::World;

    #[test]
    fn propagate_copies_the_source_into_the_target_field() {
        let mut world = World::new();
        let src = spawn_float(&mut world, 3.5);
        let dst = spawn_gain(&mut world, 0.0);

        propagate_of::<GainFrom>(&mut world, src, dst);

        assert_eq!(world.get::<Gain>(dst).map(|g| g.factor), Some(3.5));
    }

    #[test]
    fn a_despawned_producer_leaves_the_consumer_untouched() {
        let mut world = World::new();
        let src = spawn_float(&mut world, 3.5);
        let dst = spawn_gain(&mut world, 1.25);
        world.despawn(src);

        propagate_of::<GainFrom>(&mut world, src, dst);

        assert_eq!(
            world.get::<Gain>(dst).map(|g| g.factor),
            Some(1.25),
            "a dangling wire must be a no-op, not a panic"
        );
    }

    #[test]
    fn a_producer_without_the_source_component_is_a_no_op() {
        let mut world = World::new();
        let src = world.spawn_empty().id();
        let dst = spawn_gain(&mut world, 1.25);

        propagate_of::<GainFrom>(&mut world, src, dst);

        assert_eq!(world.get::<Gain>(dst).map(|g| g.factor), Some(1.25));
    }

    #[test]
    fn a_consumer_without_the_target_component_is_a_no_op() {
        let mut world = World::new();
        let src = spawn_float(&mut world, 3.5);
        let dst = world.spawn_empty().id();

        propagate_of::<GainFrom>(&mut world, src, dst);

        assert!(world.get::<Gain>(dst).is_none());
    }

    #[test]
    fn the_source_value_is_read_not_the_wire_component() {
        // Guards against reading the relationship's Entity by mistake.
        let mut world = World::new();
        let src = spawn_float(&mut world, -2.0);
        let dst = spawn_gain(&mut world, 0.0);
        world.entity_mut(dst).insert(GainFrom(src));

        propagate_of::<GainFrom>(&mut world, src, dst);

        assert_eq!(world.get::<Gain>(dst).map(|g| g.factor), Some(-2.0));
    }
}
