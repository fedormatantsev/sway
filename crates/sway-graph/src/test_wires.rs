//! Engine-only wire fixtures. Deliberately not real nodes: these exist to
//! exercise the contract, not to do anything musical.

use bevy_ecs::change_detection::{DetectChangesMut, Mut};
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::wire::Wire;

/// A producer's output. An outlet is a component (spec §2.1).
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
pub struct FloatOut(pub f32);

/// A consumer with one driveable field and one derived one.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
pub struct Gain {
    pub factor: f32,
    pub value: f32,
}

#[derive(Component)]
#[relationship(relationship_target = DrivesGain)]
pub struct GainFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = GainFrom)]
pub struct DrivesGain(Vec<Entity>);

impl Wire for GainFrom {
    type Source = FloatOut;
    type Target = Gain;
    const NAME: &'static str = "factor";

    fn propagate(src: &FloatOut, dst: Mut<Gain>) {
        dst.map_unchanged(|g| &mut g.factor).set_if_neq(src.0);
    }
}

pub fn spawn_float(world: &mut World, value: f32) -> Entity {
    world.spawn(FloatOut(value)).id()
}

pub fn spawn_gain(world: &mut World, factor: f32) -> Entity {
    world.spawn(Gain { factor, value: 0.0 }).id()
}
