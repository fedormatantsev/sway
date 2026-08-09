//! Outlets. An outlet is a component (spec §2.1): an entity has a `f32`
//! outlet because it has `FloatOut`.

use bevy::prelude::*;

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FloatOut(pub f32);

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct Vec3Out(pub Vec3);
