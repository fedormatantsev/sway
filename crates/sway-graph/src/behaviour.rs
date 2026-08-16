//! `Behaviour` — reflected in-tick node logic over inlets, state, and outlets.

use std::any::TypeId;

use bevy_ecs::change_detection::Mut;
use bevy_reflect::{Reflect, reflect_trait};

use crate::ctx::TickCtx;

/// In-tick computation attached to a node's inlet (or marker) component.
///
/// `&self` is the inlets after inbound wires this tick. State is read/write
/// in place; outlets are write-only in place. The trait never sees `World`
/// or `Entity`.
#[reflect_trait]
pub trait Behaviour {
    fn state_type(&self) -> Option<TypeId>;
    fn outlet_type(&self) -> Option<TypeId>;
    fn evaluate(
        &self,
        state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        ctx: &TickCtx,
    );
}
