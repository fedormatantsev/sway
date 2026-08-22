//! [`Trigger`]: the generic occurrence payload other domains fire and consume.
//!
//! A unit struct: it means something happened, and carries nothing else
//! (design D1). Handles of this type travel ordinary graph wires; the arena
//! holds the batches.

use bevy_reflect::Reflect;

/// A unit occurrence: something happened this tick.
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger;
