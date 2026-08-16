//! Registration of reflected wire and behaviour types.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::world::DeferredWorld;
use bevy_reflect::{GetTypeRegistration, Reflect, TypePath};

use crate::behaviour::{Behaviour, ReflectBehaviour};
use crate::order::TopologyDirty;
use crate::watch::Authoring;
use crate::wire::{ReflectWire, Wire};

/// `register_type` plus `ReflectWire` type data and `Authoring`-gated
/// topology hooks. Safe to call for Bevy's `ChildOf`, which cannot carry
/// `#[reflect(Wire)]` on the upstream type.
pub fn register_wire_type<W>(app: &mut App)
where
    W: Wire + Component + Reflect + GetTypeRegistration + TypePath,
{
    app.register_type::<W>();
    app.register_type_data::<W, ReflectWire>();
    app.world_mut()
        .register_component_hooks::<W>()
        .on_add(mark_topology_dirty_if_authoring)
        .on_remove(mark_topology_dirty_if_authoring);
}

/// `register_type` plus `ReflectBehaviour` type data and `Authoring`-gated
/// topology hooks. Pair with `#[reflect(Behaviour)]` on the inlet type.
pub fn register_behaviour_type<B>(app: &mut App)
where
    B: Behaviour + Component + Reflect + GetTypeRegistration + TypePath,
{
    app.register_type::<B>();
    app.register_type_data::<B, ReflectBehaviour>();
    app.world_mut()
        .register_component_hooks::<B>()
        .on_add(mark_topology_dirty_if_authoring)
        .on_remove(mark_topology_dirty_if_authoring);
}

fn mark_topology_dirty_if_authoring(mut world: DeferredWorld, _ctx: HookContext) {
    if world.get_resource::<Authoring>().is_none() {
        return;
    }
    if let Some(mut dirty) = world.get_resource_mut::<TopologyDirty>() {
        dirty.0 = true;
    }
}
