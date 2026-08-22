//! Fixture node kinds for this crate's behaviour tests.
//!
//! They stand in for the real producers and consumers a domain will declare:
//! `sway-graph`'s `test-support` harness runs them over a real `Graph`, so
//! everything asserted here is asserted about the actual tick rather than a
//! model of it (design D10).

use bevy_ecs::world::World;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{Reflect, TypeRegistry};
use sway_graph::graph::{NodeKind, ReflectNodeKind, register_node_kind};

use crate::arena::EventArena;
use crate::handle::EventHandle;
use crate::plugin::register_event_handle;

/// The payload every fixture publishes.
#[derive(Reflect, Clone, Copy, Debug, PartialEq)]
pub struct Ping(pub u32);

/// A second payload, so "two handles of different payloads cannot connect" has
/// something to be refused against.
#[derive(Reflect, Clone, Copy, Debug, PartialEq)]
pub struct Pong(pub u32);

// --- Emitter: publishes and keeps nothing ------------------------------

/// [`Emitter`]'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct EmitterIn {
    /// How many occurrences to publish this tick. An ordinary `f32` so the
    /// rate is drivable from the graph.
    pub count: f32,
}

/// [`Emitter`]'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct EmitterOut {
    /// The handle naming this tick's batch.
    pub pings: EventHandle<Ping>,
}

/// Publishes `count` occurrences per tick. Its `state` is `()`, which is the
/// point: everything it published is reachable from the handle on its outlet.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Emitter {
    /// Inlets.
    pub inlets: EmitterIn,
    /// State — nothing is kept between ticks.
    pub state: (),
    /// Outlets.
    pub outlets: EmitterOut,
}

impl NodeKind for Emitter {
    fn evaluate(&mut self, world: &World) {
        let count = self.inlets.count.max(0.0) as u32;
        // No arena is no occurrences, not a failed evaluation.
        self.outlets.pings = match world.get_non_send::<EventArena>() {
            // Published unconditionally: an empty batch folds to the empty
            // handle, so a silent tick still reports no change (design D7).
            Some(arena) => arena.publish((0..count).map(Ping)),
            None => EventHandle::EMPTY,
        };
    }
}

// --- Tally: reads a handle ---------------------------------------------

/// [`Tally`]'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct TallyIn {
    /// The handle to read.
    pub pings: EventHandle<Ping>,
}

/// [`Tally`]'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct TallyOut {
    /// How many occurrences arrived this tick.
    pub count: f32,
    /// The sum of their payloads, so "the same batch" can be told from "a
    /// batch of the same length".
    pub sum: f32,
}

/// Counts the occurrences on its inlet. Reads twice on purpose, because
/// reading must not consume.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Tally {
    /// Inlets.
    pub inlets: TallyIn,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: TallyOut,
}

impl NodeKind for Tally {
    fn evaluate(&mut self, world: &World) {
        let batch = world
            .get_non_send::<EventArena>()
            .and_then(|arena| arena.read(self.inlets.pings));
        let Some(batch) = batch else {
            self.outlets.count = 0.0;
            self.outlets.sum = 0.0;
            return;
        };
        self.outlets.count = batch.len() as f32;
        self.outlets.sum = batch.into_iter().map(|ping| ping.0 as f32).sum();
    }
}

// --- Relay: reads a batch and publishes one of its own -----------------

/// [`Relay`]'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct RelayIn {
    /// The handle to forward.
    pub pings: EventHandle<Ping>,
}

/// [`Relay`]'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct RelayOut {
    /// A handle naming the relay's *own* batch — never the one it received.
    pub pings: EventHandle<Ping>,
}

/// Forwards occurrences by publishing a batch of its own, which is what
/// forwarding means here: there is no operation that would let it hand on the
/// handle it was given and still be a producer.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Relay {
    /// Inlets.
    pub inlets: RelayIn,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: RelayOut,
}

impl NodeKind for Relay {
    fn evaluate(&mut self, world: &World) {
        let Some(arena) = world.get_non_send::<EventArena>() else {
            self.outlets.pings = EventHandle::EMPTY;
            return;
        };
        // Reading and then publishing in one scope is the case design D2 is
        // about: the batch is an owned share, so nothing is borrowed here.
        let forwarded: Vec<Ping> = match arena.read(self.inlets.pings) {
            Some(batch) => batch.into_iter().map(|ping| Ping(ping.0 + 100)).collect(),
            None => Vec::new(),
        };
        self.outlets.pings = arena.publish(forwarded);
    }
}

// --- Wrappers: the optional and variadic inlet shapes -------------------

/// [`Maybe`]'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct MaybeIn {
    /// An optional handle inlet — absent, not defaulted, when unconnected.
    pub pings: Option<EventHandle<Ping>>,
    /// A variadic handle inlet: several trigger sources merged by the ordinary
    /// rule, in ordering-key order.
    pub many: Vec<EventHandle<Ping>>,
    /// A handle of another payload, for the connect-legality tests.
    pub pongs: EventHandle<Pong>,
}

/// [`Maybe`]'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct MaybeOut {
    /// Whether the optional inlet was present this tick.
    pub had_pings: bool,
    /// The payloads read off `many`, in the order the handles arrived.
    pub merged: Vec<f32>,
}

/// Carries the wrapper-shaped handle inlets.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Maybe {
    /// Inlets.
    pub inlets: MaybeIn,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: MaybeOut,
}

impl NodeKind for Maybe {
    fn evaluate(&mut self, world: &World) {
        self.outlets.had_pings = self.inlets.pings.is_some();
        self.outlets.merged.clear();
        let Some(arena) = world.get_non_send::<EventArena>() else {
            return;
        };
        for handle in &self.inlets.many {
            let Some(batch) = arena.read(*handle) else {
                continue;
            };
            self.outlets
                .merged
                .extend(batch.into_iter().map(|ping| ping.0 as f32));
        }
    }
}

// --- Held: a handle inlet beside an authored one -----------------------

/// [`Held`]'s inlets — a handle standing next to an ordinary authored field,
/// which is the document round-trip's whole subject.
#[derive(Reflect, Default, Debug, PartialEq)]
pub struct HeldIn {
    /// Session state: never authored, never stored.
    pub pings: EventHandle<Ping>,
    /// An ordinary authored value, which must survive the round-trip.
    pub gain: f32,
}

/// A node kind whose inlets mix session state and authored data.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Held {
    /// Inlets.
    pub inlets: HeldIn,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: (),
}

impl NodeKind for Held {
    fn evaluate(&mut self, _world: &World) {}
}

/// A registry with every fixture kind and the fixture payload handles.
pub fn test_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_node_kind::<Emitter>(&mut registry);
    register_node_kind::<Tally>(&mut registry);
    register_node_kind::<Relay>(&mut registry);
    register_node_kind::<Maybe>(&mut registry);
    register_node_kind::<Held>(&mut registry);
    register_part_types(&mut registry);
    registry
}

/// The part types and handles, registered separately so the `App`-side tests
/// can add them to a registry `register_node_kind` already populated.
pub fn register_part_types(registry: &mut TypeRegistry) {
    register_event_handle::<Ping>(registry);
    register_event_handle::<Pong>(registry);
    registry.register::<Ping>();
    registry.register::<Pong>();
    registry.register::<EmitterIn>();
    registry.register::<EmitterOut>();
    registry.register::<TallyIn>();
    registry.register::<TallyOut>();
    registry.register::<RelayIn>();
    registry.register::<RelayOut>();
    registry.register::<MaybeIn>();
    registry.register::<MaybeOut>();
    registry.register::<HeldIn>();
}

/// Registers every fixture kind on an `App`, for the plugin-level tests.
pub fn register_fixtures(app: &mut bevy_app::App) {
    use bevy_ecs::reflect::AppTypeRegistry;
    use sway_graph::graph::RegisterNodeKind;

    app.register_node_kind::<Emitter>()
        .register_node_kind::<Tally>()
        .register_node_kind::<Relay>()
        .register_node_kind::<Maybe>()
        .register_node_kind::<Held>();
    let registry = app
        .world_mut()
        .get_resource_or_init::<AppTypeRegistry>()
        .clone();
    register_part_types(&mut registry.write());
}
