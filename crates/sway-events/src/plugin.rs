//! Registration and the crate's single plugin.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy_ecs::system::NonSendMut;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};
use bevy_reflect::{TypePath, TypeRegistry};

use crate::arena::EventArena;
use crate::handle::EventHandle;

/// Makes `EventHandle<P>` known to reflection.
///
/// Three pieces of type data, and each one is load-bearing: `ReflectDefault`
/// because a node is built from it and only its inlets are then applied, so a
/// handle field has to start as [`EventHandle::EMPTY`]; `ReflectSerialize` and
/// `ReflectDeserialize` because the document serializes the whole `inlets`
/// struct in one go, and a handle inlet with no serializer fails the *save of
/// its node* (design D8/D9).
///
/// The arena needs no registration of any kind: it never asks what a payload
/// is, only that `Vec<P>` downcasts back out of the box it stored.
pub fn register_event_handle<P: TypePath + Send + Sync + 'static>(registry: &mut TypeRegistry) {
    registry.register::<EventHandle<P>>();
    registry.register_type_data::<EventHandle<P>, ReflectDefault>();
    registry.register_type_data::<EventHandle<P>, ReflectSerialize>();
    registry.register_type_data::<EventHandle<P>, ReflectDeserialize>();
}

/// `App`-side sugar for [`register_event_handle`], shaped like `sway-graph`'s
/// `RegisterNodeKind`.
pub trait RegisterEventHandle {
    /// Registers `EventHandle<P>`.
    fn register_event_handle<P: TypePath + Send + Sync + 'static>(&mut self) -> &mut Self;
}

impl RegisterEventHandle for App {
    fn register_event_handle<P: TypePath + Send + Sync + 'static>(&mut self) -> &mut Self {
        let type_registry = self
            .world_mut()
            .get_resource_or_init::<AppTypeRegistry>()
            .clone();
        register_event_handle::<P>(&mut type_registry.write());
        self
    }
}

/// The set the arena's clear runs in, ordered before `GraphTickSet`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventClearSet;

/// Empties the arena. The whole lifecycle of an occurrence, on the engine's
/// side, is this one system.
pub fn clear_event_arena(mut arena: NonSendMut<EventArena>) {
    arena.clear();
}

/// The crate's one plugin: adding it is all a host does for the arena to exist
/// and be emptied before every tick.
///
/// The clear is deliberately **not** gated on asset loading, even though the
/// tick is. If a producer outside the graph ever publishes while a project is
/// still loading, an ungated clear is what keeps the arena bounded; and with
/// no tick running there is nothing to starve.
pub struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(EventArena::default()).add_systems(
            FixedUpdate,
            clear_event_arena
                .in_set(EventClearSet)
                .before(sway_graph::GraphTickSet),
        );
    }
}
