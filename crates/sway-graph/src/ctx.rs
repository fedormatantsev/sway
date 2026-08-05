//! Context shared by everything the graph runs this tick.

use bevy_ecs::component::Component;
use bevy_math::Vec2;
use bevy_reflect::Reflect;

/// Where the editor draws this entity, in graph-canvas space.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
pub struct EditorPos(pub Vec2);

/// Context shared by every behaviour run this tick.
pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds.
    pub tick_start: f64,
    /// Monotonically increasing tick counter, starting at 0.
    pub tick_index: u64,
}
