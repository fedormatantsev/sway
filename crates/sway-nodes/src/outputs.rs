//! Outlets. An outlet is a component (spec §2.1): an entity has a `f32`
//! outlet because it has `FloatOut`.

use bevy::prelude::*;
use bevy_ecs::change_detection::Mut;
use bevy_reflect::{PartialReflect, Reflect};

/// Writes `next` through a reflected outlet slot, leaving `Changed` clear
/// when the value is already equal.
pub(crate) fn write_outlet(outlets: Option<Mut<dyn Reflect>>, next: impl PartialReflect) {
    let Some(mut outlets) = outlets else {
        return;
    };
    if (*outlets).as_partial_reflect().reflect_partial_eq(&next) == Some(true) {
        return;
    }
    let _ = outlets.try_apply(&next);
}

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FloatOut(pub f32);

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct Vec3Out(pub Vec3);
