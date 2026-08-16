//! `Wire` — a reflected relationship on the consumer.

use std::any::TypeId;

use bevy_ecs::change_detection::Mut;
use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::world::World;
use bevy_reflect::{PartialReflect, Reflect, ReflectMut, ReflectRef, reflect_trait};
use bevy_transform::components::Transform;

use crate::dispatch;

/// A connection. The relationship component lives on the consumer and names
/// the producer; the relationship-target on the producer collects consumers.
///
/// Object-safe so the type registry can hold `ReflectWire` and the tick can
/// call `propagate` on a `FromReflect` stack copy. Not a `Relationship`
/// supertrait: that trait is `Sized`.
#[reflect_trait]
pub trait Wire {
    fn producer(&self) -> Entity;
    fn source_type(&self) -> TypeId;
    fn target_type(&self) -> TypeId;
    fn source_path(&self) -> &'static str;
    fn target_path(&self) -> &'static str;

    /// Copy the producer's outlet into the consumer's inlet. Default is a
    /// reflected field copy along [`Self::source_path`] / [`Self::target_path`].
    /// An empty target path is a no-op (`ChildOf`).
    fn propagate(&self, outlet: &dyn PartialReflect, inlet: Mut<dyn Reflect>) {
        propagate_field_copy(outlet, inlet, self.source_path(), self.target_path());
    }
}

/// Copies `source_path` on `outlet` onto `target_path` on `inlet` when the
/// values are not reflect-equal. `None` from `reflect_partial_eq` is treated
/// as not equal. Empty `target_path` does nothing.
pub fn propagate_field_copy(
    outlet: &dyn PartialReflect,
    inlet: Mut<dyn Reflect>,
    source_path: &str,
    target_path: &str,
) {
    if target_path.is_empty() {
        return;
    }
    let Some(source_field) = reflect_field(outlet, source_path) else {
        return;
    };
    let equal = reflect_field((*inlet).as_partial_reflect(), target_path)
        .and_then(|target| target.reflect_partial_eq(source_field))
        == Some(true);
    if equal {
        return;
    }
    let Some(mut field) = map_field(inlet, target_path) else {
        return;
    };
    let _ = field.try_apply(source_field);
}

fn reflect_field<'a>(value: &'a dyn PartialReflect, path: &str) -> Option<&'a dyn PartialReflect> {
    match value.reflect_ref() {
        ReflectRef::Struct(target) => target.field(path).or_else(|| {
            path.parse::<usize>()
                .ok()
                .and_then(|index| target.field_at(index))
        }),
        ReflectRef::TupleStruct(target) => path
            .parse::<usize>()
            .ok()
            .and_then(|index| target.field(index)),
        ReflectRef::Tuple(target) => path
            .parse::<usize>()
            .ok()
            .and_then(|index| target.field(index)),
        _ => None,
    }
}

fn reflect_field_mut<'a>(
    value: &'a mut dyn PartialReflect,
    path: &str,
) -> Option<&'a mut dyn PartialReflect> {
    match value.reflect_mut() {
        ReflectMut::Struct(target) => {
            if target.field(path).is_some() {
                target.field_mut(path)
            } else {
                path.parse::<usize>()
                    .ok()
                    .and_then(|index| target.field_at_mut(index))
            }
        }
        ReflectMut::TupleStruct(target) => path
            .parse::<usize>()
            .ok()
            .and_then(|index| target.field_mut(index)),
        ReflectMut::Tuple(target) => path
            .parse::<usize>()
            .ok()
            .and_then(|index| target.field_mut(index)),
        _ => None,
    }
}

fn map_field<'a>(
    inlet: Mut<'a, dyn Reflect>,
    target_path: &str,
) -> Option<Mut<'a, dyn PartialReflect>> {
    // `map_unchanged` requires a field pointer; the immutable check above
    // already proved the path exists on this value.
    let path = target_path.to_owned();
    Some(inlet.map_unchanged(move |value| {
        reflect_field_mut(value.as_partial_reflect_mut(), &path)
            .expect("target field existed during the immutable equal-value check")
    }))
}

/// Hierarchy costs one impl, because `ChildOf` is already a `Relationship`.
/// Empty `target_path` makes the default propagate a no-op; Bevy's own hooks
/// maintain `Children`. Source/target types still define wiring legality.
impl Wire for ChildOf {
    fn producer(&self) -> Entity {
        self.0
    }

    fn source_type(&self) -> TypeId {
        TypeId::of::<Transform>()
    }

    fn target_type(&self) -> TypeId {
        TypeId::of::<Transform>()
    }

    fn source_path(&self) -> &'static str {
        "0"
    }

    fn target_path(&self) -> &'static str {
        ""
    }
}

/// `FromReflect`s the wire on `dst`, fetches outlet/inlet from its type
/// methods, and calls [`Wire::propagate`]. Missing source, target, or
/// entities returns [`Err`] without mutating the other entity.
pub fn propagate_reflected(
    world: &mut World,
    src: Entity,
    dst: Entity,
    type_id: TypeId,
) -> dispatch::Result<()> {
    dispatch::propagate_reflected(world, src, dst, type_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::register_wire_type;
    use crate::test_wires::{Gain, GainFrom, spawn_float, spawn_gain};
    use bevy_app::App;
    use bevy_ecs::change_detection::DetectChanges;
    use bevy_ecs::reflect::AppTypeRegistry;

    fn wire_world() -> (World, Entity, Entity) {
        let mut app = App::new();
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<Gain>();
        register_wire_type::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 3.5);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));
        let world = std::mem::take(app.world_mut());
        (world, src, dst)
    }

    fn propagate(world: &mut World, src: Entity, dst: Entity) -> dispatch::Result<()> {
        propagate_reflected(world, src, dst, TypeId::of::<GainFrom>())
    }

    #[test]
    fn propagate_copies_the_source_into_the_target_field() {
        let (mut world, src, dst) = wire_world();
        world
            .entity_mut(src)
            .insert(crate::test_wires::FloatOut(3.5));

        propagate(&mut world, src, dst).expect("propagate");

        assert_eq!(world.get::<Gain>(dst).map(|g| g.factor), Some(3.5));
    }

    #[test]
    fn a_despawned_producer_leaves_the_consumer_untouched() {
        let (mut world, src, dst) = wire_world();
        world.entity_mut(dst).insert(Gain {
            factor: 1.25,
            value: 0.0,
        });
        world.despawn(src);

        assert!(propagate(&mut world, src, dst).is_err());

        assert_eq!(
            world.get::<Gain>(dst).map(|g| g.factor),
            Some(1.25),
            "a dangling wire must be a no-op, not a panic"
        );
    }

    #[test]
    fn a_producer_without_the_source_component_is_a_no_op() {
        let mut app = App::new();
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<Gain>();
        register_wire_type::<GainFrom>(&mut app);
        let src = app.world_mut().spawn_empty().id();
        let dst = spawn_gain(app.world_mut(), 1.25);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        assert!(propagate(app.world_mut(), src, dst).is_err());

        assert_eq!(app.world().get::<Gain>(dst).map(|g| g.factor), Some(1.25));
    }

    #[test]
    fn a_consumer_without_the_target_component_is_a_no_op() {
        let mut app = App::new();
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<Gain>();
        register_wire_type::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 3.5);
        let dst = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        assert!(propagate(app.world_mut(), src, dst).is_err());

        assert!(app.world().get::<Gain>(dst).is_none());
    }

    #[test]
    fn the_source_value_is_read_not_the_wire_component() {
        let (mut world, src, dst) = wire_world();
        world
            .entity_mut(src)
            .insert(crate::test_wires::FloatOut(-2.0));
        world.entity_mut(dst).insert(Gain {
            factor: 0.0,
            value: 0.0,
        });

        propagate(&mut world, src, dst).expect("propagate");

        assert_eq!(world.get::<Gain>(dst).map(|g| g.factor), Some(-2.0));
    }

    #[test]
    fn equal_value_does_not_dirty_the_target() {
        let (mut world, src, dst) = wire_world();
        world
            .entity_mut(src)
            .insert(crate::test_wires::FloatOut(3.5));
        world.entity_mut(dst).insert(Gain {
            factor: 0.0,
            value: 0.0,
        });

        propagate(&mut world, src, dst).expect("propagate");
        world.clear_trackers();
        propagate(&mut world, src, dst).expect("propagate");

        assert!(
            !world.entity(dst).get_ref::<Gain>().unwrap().is_changed(),
            "a second copy of the same value must not re-mark Changed"
        );
    }

    #[test]
    fn child_of_is_a_reflected_no_op_wire() {
        let mut app = App::new();
        register_wire_type::<ChildOf>(&mut app);
        let registry = app.world().resource::<AppTypeRegistry>().read();
        assert!(
            registry
                .get(TypeId::of::<ChildOf>())
                .and_then(|r| r.data::<ReflectWire>())
                .is_some()
        );
        let wire = ChildOf(Entity::from_raw_u32(1).unwrap());
        assert!(wire.target_path().is_empty());
    }
}
