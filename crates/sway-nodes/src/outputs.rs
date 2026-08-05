//! Outlets. An outlet is a component (spec §2.1): an entity has a `f32`
//! outlet because it has `FloatOut`.

use bevy::prelude::*;

#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
pub struct FloatOut(pub f32);

#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
pub struct Vec3Out(pub Vec3);
