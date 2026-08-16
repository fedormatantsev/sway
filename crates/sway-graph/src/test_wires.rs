//! Engine-only wire fixtures. Deliberately not real nodes: these exist to
//! exercise the contract, not to do anything musical.

use std::any::TypeId;

use bevy_ecs::change_detection::Mut;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::world::World;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::Reflect;

use crate::behaviour::Behaviour;
use crate::ctx::TickCtx;
use crate::wire::{ReflectWire, Wire};

/// A producer's output. An outlet is a component.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FloatOut(pub f32);

/// A consumer with one driveable field and one derived one.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct Gain {
    pub factor: f32,
    pub value: f32,
}

#[derive(Component, Reflect, Clone, Copy)]
#[relationship(relationship_target = DrivesGain)]
#[reflect(Component, Wire)]
pub struct GainFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = GainFrom)]
pub struct DrivesGain(Vec<Entity>);

impl From<Entity> for GainFrom {
    fn from(entity: Entity) -> Self {
        Self(entity)
    }
}

impl Wire for GainFrom {
    fn producer(&self) -> Entity {
        self.0
    }

    fn source_type(&self) -> TypeId {
        TypeId::of::<FloatOut>()
    }

    fn target_type(&self) -> TypeId {
        TypeId::of::<Gain>()
    }

    fn source_path(&self) -> &'static str {
        "0"
    }

    fn target_path(&self) -> &'static str {
        "factor"
    }
}

impl Behaviour for Gain {
    fn state_type(&self) -> Option<TypeId> {
        None
    }

    fn outlet_type(&self) -> Option<TypeId> {
        Some(TypeId::of::<FloatOut>())
    }

    fn evaluate(
        &self,
        _state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        _ctx: &TickCtx,
    ) {
        let Some(mut outlets) = outlets else {
            return;
        };
        let doubled = self.factor * 2.0;
        let next = FloatOut(doubled);
        if (*outlets).as_partial_reflect().reflect_partial_eq(&next) == Some(true) {
            return;
        }
        let _ = outlets.try_apply(&next);
    }
}

pub fn spawn_float(world: &mut World, value: f32) -> Entity {
    world.spawn(FloatOut(value)).id()
}

pub fn spawn_gain(world: &mut World, factor: f32) -> Entity {
    world.spawn(Gain { factor, value: 0.0 }).id()
}
