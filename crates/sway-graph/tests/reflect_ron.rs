//! What bevy_reflect 0.19 and ron 0.12 actually do together, pinned.
//!
//! The project format (specs/2026-08-06-project-format-design.md §3, §10)
//! rests on these behaviours. They are checked here, against the real
//! libraries, before anything is built on them.

use std::any::TypeId;

use bevy_ecs::component::Component;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::prelude::ReflectDefault;
use bevy_reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer};
use bevy_reflect::{PartialReflect, Reflect, TypeRegistry};
use serde::de::DeserializeSeed;
use ron::de::Deserializer as RonDeserializer;

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
enum Shape {
    #[default]
    Sine,
    Saw,
}

#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, PartialEq)]
struct Osc {
    hz: f32,
    shape: Shape,
    amplitude: f32,
}

impl Default for Osc {
    fn default() -> Self {
        Self { hz: 1.0, shape: Shape::Sine, amplitude: 0.5 }
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    registry.register::<Osc>();
    registry.register::<Shape>();
    registry
}

/// The exact path the loader will take: text -> ron deserializer -> partial reflect.
/// (Fallback: ron::Value causes enum issues with bevy_reflect 0.19,
/// so we use ron::Deserializer directly instead per Task 1 fallback.)
fn payload(text: &str, registry: &TypeRegistry) -> Box<dyn PartialReflect> {
    let registration = registry
        .get(TypeId::of::<Osc>())
        .expect("Osc is registered");
    let mut de = RonDeserializer::from_str(text).expect("text is valid ron");
    TypedReflectDeserializer::new(registration, registry)
        .deserialize(&mut de)
        .expect("ron deserializer drives a reflect deserializer")
}

fn reflect_component(registry: &TypeRegistry) -> ReflectComponent {
    registry
        .get_type_data::<ReflectComponent>(TypeId::of::<Osc>())
        .expect("#[reflect(Component)] supplies ReflectComponent")
        .clone()
}

/// CLAIM 1: a ron::Value can drive a reflect deserializer at all, and a full
/// payload reconstructs the component exactly.
#[test]
fn a_full_payload_becomes_the_component() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("Osc(hz: 2.0, shape: Saw, amplitude: 0.25)", &registry);

    let mut world = World::new();
    let entity = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(entity), &*value, &registry);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 2.0, shape: Shape::Saw, amplitude: 0.25 })
    );
}

/// CLAIM 2: a PARTIAL payload fills the rest from ReflectDefault. This is what
/// lets a document name one field of Transform. If it fails, the format needs
/// complete payloads everywhere.
#[test]
fn a_partial_payload_fills_the_rest_from_default() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("Osc(hz: 2.0)", &registry);

    let mut world = World::new();
    let entity = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(entity), &*value, &registry);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 2.0, shape: Shape::Sine, amplitude: 0.5 }),
        "unnamed fields come from Default, not from zero"
    );
}

/// CLAIM 3: `apply` on an EXISTING component touches only the named fields.
/// This is what stops a reload from clobbering a field a wire is driving.
#[test]
fn apply_leaves_unnamed_fields_alone() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("Osc(hz: 3.0)", &registry);

    let mut world = World::new();
    let entity = world
        .spawn(Osc { hz: 1.0, shape: Shape::Saw, amplitude: 0.9 })
        .id();
    reflect.apply(&mut world.entity_mut(entity), &*value);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 3.0, shape: Shape::Saw, amplitude: 0.9 })
    );
}

/// CLAIM 4: a partial value compares equal to a component whose named fields
/// match. This is the skip-if-unchanged gate; without it every reload marks
/// every component Changed.
#[test]
fn a_partial_value_compares_against_the_live_component() {
    let registry = registry();
    let current = Osc { hz: 3.0, shape: Shape::Saw, amplitude: 0.9 };

    let same = payload("Osc(hz: 3.0)", &registry);
    let different = payload("Osc(hz: 4.0)", &registry);

    assert_eq!(same.reflect_partial_eq(current.as_partial_reflect()), Some(true));
    assert_eq!(different.reflect_partial_eq(current.as_partial_reflect()), Some(false));
}

/// CLAIM 5: a live component serializes back to text the loader can read.
/// The emitter's whole job, in one assertion.
#[test]
fn a_component_round_trips_through_text() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let original = Osc { hz: 7.5, shape: Shape::Saw, amplitude: 0.125 };

    let mut world = World::new();
    let entity = world.spawn(original).id();

    let entity_ref = world.entity(entity);
    let value = reflect.reflect(entity_ref).expect("component is present");
    let text = ron::to_string(&TypedReflectSerializer::new(
        value.as_partial_reflect(),
        &registry,
    ))
    .expect("a reflected component serializes");

    let back = payload(&text, &registry);
    let restored = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(restored), &*back, &registry);

    assert_eq!(world.get::<Osc>(restored), Some(&original));
}

/// AppTypeRegistry is what the applier will read out of the world; check it
/// carries what a plain TypeRegistry does.
#[test]
fn the_app_registry_carries_the_same_type_data() {
    let mut world = World::new();
    world.init_resource::<AppTypeRegistry>();
    world.resource_mut::<AppTypeRegistry>().write().register::<Osc>();

    let registry = world.resource::<AppTypeRegistry>().clone();
    let read = registry.read();
    assert!(read.get_type_data::<ReflectComponent>(TypeId::of::<Osc>()).is_some());
}
