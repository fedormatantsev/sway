//! Type-erased wire/behaviour dispatch through `AppTypeRegistry`.

use std::any::TypeId;

use bevy_ecs::change_detection::Mut;
use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::{EntityMut, World};
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::tuple_struct::DynamicTupleStruct;
use bevy_reflect::{Reflect, ReflectFromReflect, TypeRegistry};

use crate::behaviour::ReflectBehaviour;
use crate::ctx::TickCtx;
use crate::wire::{ReflectWire, Wire};

pub fn propagate_reflected(world: &mut World, src: Entity, dst: Entity, type_id: TypeId) {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let Some(wire) = stack_wire(&registry, world, dst, type_id) else {
        return;
    };
    let source_type = wire.source_type();
    let target_type = wire.target_type();
    let Some(outlet_reflect) = reflect_component(&registry, source_type) else {
        return;
    };
    let Some(inlet_reflect) = reflect_component(&registry, target_type) else {
        return;
    };
    drop(registry);

    let Ok([src_ref, dst_mut]) = world.get_entity_mut([src, dst]) else {
        return;
    };
    let Some(outlet) = outlet_reflect.reflect(src_ref) else {
        return;
    };
    let Some(inlet) = inlet_reflect.reflect_mut(dst_mut) else {
        return;
    };
    wire.propagate(outlet.as_partial_reflect(), inlet);
}

pub fn evaluate_reflected(world: &mut World, entity: Entity, type_id: TypeId, ctx: &TickCtx) {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let Some(registration) = registry.get(type_id) else {
        return;
    };
    let Some(inlets_reflect) = registration.data::<ReflectComponent>().cloned() else {
        return;
    };
    let Some(reflect_behaviour) = registration.data::<ReflectBehaviour>().cloned() else {
        return;
    };
    let Some(from_reflect) = registration.data::<ReflectFromReflect>().cloned() else {
        return;
    };
    let Ok(entity_ref) = world.get_entity(entity) else {
        return;
    };
    let Some(reflected) = inlets_reflect.reflect(entity_ref) else {
        return;
    };
    let Some(owned) = from_reflect.from_reflect(reflected.as_partial_reflect()) else {
        return;
    };
    let Ok(behaviour) = reflect_behaviour.get_boxed(owned) else {
        return;
    };

    let state_type = behaviour.state_type();
    let outlet_type = behaviour.outlet_type();
    let state_reflect = state_type.and_then(|id| reflect_component(&registry, id));
    let outlet_reflect = outlet_type.and_then(|id| reflect_component(&registry, id));
    let state_default = state_type.and_then(|id| reflect_default(&registry, id));
    let outlet_default = outlet_type.and_then(|id| reflect_default(&registry, id));
    drop(registry);

    if let (Some(reflect_component), Some(reflect_default)) =
        (state_reflect.as_ref(), state_default.as_ref())
    {
        ensure_component(world, entity, reflect_component, reflect_default, &type_registry);
    }
    if let (Some(reflect_component), Some(reflect_default)) =
        (outlet_reflect.as_ref(), outlet_default.as_ref())
    {
        ensure_component(world, entity, reflect_component, reflect_default, &type_registry);
    }

    let Ok(entity_world_mut) = world.get_entity_mut(entity) else {
        return;
    };
    let mut entity_mut = EntityMut::from(entity_world_mut);
    let (state, outlets) =
        reflect_state_and_outlets(&mut entity_mut, state_reflect.as_ref(), outlet_reflect.as_ref());
    behaviour.evaluate(state, outlets, ctx);
}

pub fn stack_wire(
    registry: &TypeRegistry,
    world: &World,
    dst: Entity,
    type_id: TypeId,
) -> Option<Box<dyn Wire>> {
    let registration = registry.get(type_id)?;
    let reflect_component = registration.data::<ReflectComponent>()?;
    let reflect_wire = registration.data::<ReflectWire>()?;
    let from_reflect = registration.data::<ReflectFromReflect>()?;
    let entity_ref = world.get_entity(dst).ok()?;
    let reflected = reflect_component.reflect(entity_ref)?;
    let owned = from_reflect.from_reflect(reflected.as_partial_reflect())?;
    reflect_wire.get_boxed(owned).ok()
}

pub fn make_wire_component(
    registry: &TypeRegistry,
    type_id: TypeId,
    producer: Entity,
) -> Option<Box<dyn Reflect>> {
    let registration = registry.get(type_id)?;
    let from_reflect = registration.data::<ReflectFromReflect>()?;
    let mut dynamic = DynamicTupleStruct::default();
    dynamic.set_represented_type(Some(registration.type_info()));
    dynamic.insert(producer);
    from_reflect.from_reflect(&dynamic)
}

pub fn reflect_component(registry: &TypeRegistry, type_id: TypeId) -> Option<ReflectComponent> {
    registry
        .get(type_id)
        .and_then(|registration| registration.data::<ReflectComponent>().cloned())
}

pub fn reflect_default(registry: &TypeRegistry, type_id: TypeId) -> Option<ReflectDefault> {
    registry
        .get(type_id)
        .and_then(|registration| registration.data::<ReflectDefault>().cloned())
}

pub fn entity_has_type(world: &World, entity: Entity, type_id: TypeId) -> bool {
    world
        .get_entity(entity)
        .is_ok_and(|entity| entity.contains_type_id(type_id))
}

pub fn entities_with_type(world: &World, type_id: TypeId) -> Vec<Entity> {
    let Some(component_id) = world.components().get_id(type_id) else {
        return Vec::new();
    };
    entities_with_component(world, component_id)
}

pub fn entities_with_component(world: &World, component_id: ComponentId) -> Vec<Entity> {
    world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.contains(component_id))
        .flat_map(|archetype| archetype.entities().iter().map(|entity| entity.id()))
        .collect()
}

pub fn insert_wire(world: &mut World, type_path: &str, dst: Entity, src: Entity) {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let Some(registration) = registry.get_with_type_path(type_path) else {
        return;
    };
    if registration.data::<ReflectWire>().is_none() {
        return;
    }
    let type_id = registration.type_id();
    let Some(reflect_component) = registration.data::<ReflectComponent>().cloned() else {
        return;
    };
    let Some(owned) = make_wire_component(&registry, type_id, src) else {
        return;
    };
    drop(registry);

    let Ok(mut entity_mut) = world.get_entity_mut(dst) else {
        return;
    };
    let registry = type_registry.read();
    reflect_component.insert(&mut entity_mut, owned.as_partial_reflect(), &registry);
}

pub fn remove_wire(world: &mut World, type_path: &str, dst: Entity) {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let Some(registration) = registry.get_with_type_path(type_path) else {
        return;
    };
    if registration.data::<ReflectWire>().is_none() {
        return;
    }
    let Some(reflect_component) = registration.data::<ReflectComponent>().cloned() else {
        return;
    };
    drop(registry);

    let Ok(mut entity_mut) = world.get_entity_mut(dst) else {
        return;
    };
    reflect_component.remove(&mut entity_mut);
}

pub fn connect_is_legal(world: &World, type_path: &str, src: Entity, dst: Entity) -> bool {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>() else {
        return false;
    };
    let registry = type_registry.read();
    let Some(registration) = registry.get_with_type_path(type_path) else {
        return false;
    };
    let Some(reflect_wire) = registration.data::<ReflectWire>() else {
        return false;
    };
    let type_id = registration.type_id();
    let Some(owned) = make_wire_component(&registry, type_id, src) else {
        return false;
    };
    let Some(wire) = reflect_wire.get(owned.as_ref()) else {
        return false;
    };
    entity_has_type(world, src, wire.source_type()) && entity_has_type(world, dst, wire.target_type())
}

fn ensure_component(
    world: &mut World,
    entity: Entity,
    reflect_component: &ReflectComponent,
    reflect_default: &ReflectDefault,
    type_registry: &AppTypeRegistry,
) {
    let already = world
        .get_entity(entity)
        .ok()
        .is_some_and(|entity| reflect_component.contains(entity));
    if already {
        return;
    }
    let value = reflect_default.default();
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    let registry = type_registry.read();
    reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &registry);
}

fn reflect_state_and_outlets<'a>(
    entity_mut: &'a mut EntityMut,
    state_reflect: Option<&ReflectComponent>,
    outlet_reflect: Option<&ReflectComponent>,
) -> (Option<Mut<'a, dyn Reflect>>, Option<Mut<'a, dyn Reflect>>) {
    match (state_reflect, outlet_reflect) {
        (None, None) => (None, None),
        (None, Some(outlet_reflect)) => {
            let outlets = outlet_reflect.reflect_mut(entity_mut.reborrow());
            (None, outlets)
        }
        (Some(state_reflect), None) => {
            let state = state_reflect.reflect_mut(entity_mut.reborrow());
            (state, None)
        }
        (Some(state_reflect), Some(outlet_reflect)) => {
            // SAFETY: state and outlets are different components. `UnsafeEntityCell`
            // is not `Copy`, but every field is, so duplicating it is the same
            // as the documented `reflect_unchecked_mut` dual-borrow pattern.
            let cell = entity_mut.as_unsafe_entity_cell();
            let cell_outlets = unsafe { core::ptr::read(&cell) };
            let state = unsafe { state_reflect.reflect_unchecked_mut(cell) };
            let outlets = unsafe { outlet_reflect.reflect_unchecked_mut(cell_outlets) };
            (state, outlets)
        }
    }
}
