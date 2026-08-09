//! Engine-only wire fixtures. Deliberately not real nodes: these exist to
//! exercise the contract, not to do anything musical.

use bevy_ecs::change_detection::{DetectChangesMut, Mut};
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

use crate::wire::Wire;

/// A producer's output. An outlet is a component (spec §2.1).
///
/// `Reflect`/`#[reflect(Component, Default, PartialEq)]` so Task 7's project
/// tests can register it via `register_authorable` — that's the only
/// consumer of this trait on this type, the tick path never reflects it.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FloatOut(pub f32);

/// A consumer with one driveable field and one derived one.
///
/// `Reflect`/`#[reflect(Component, Default, PartialEq)]` for the same reason
/// as `FloatOut`.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
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
