//! Shared node types for `sway-graph`'s own test suite.
//!
//! Shared across Tasks 3, 4 and 5 — whichever lands first creates this file,
//! the others extend it. Task 3 used a private `Probe` defined inline in
//! `registry.rs`'s own test module; Task 4 is the first task that needs the
//! *same* node type visible from another module's tests, so it moves here.
//!
//! Task 4 adds `Probe`, `IntProbe`, `Emitter` and the spawners/app builder
//! its compiler tests need. `Gain`, `Sink`, wiring helpers (`connect`,
//! `connect_event`), assertion helpers (`recompile`, `port_value`,
//! `event_count`, `sink_offsets`) and the `GraphPlugin`-backed app builders
//! are Task 5's — left out here rather than stubbed, so Task 5 extends this
//! file instead of rewriting it.
//!
//! `Emitter`'s and `IntProbe`'s `tick` bodies are empty: Task 4's tests only
//! exercise `compile`, never `graph_tick`, and `PortView`/`TickCtx` are still
//! Task 5's stub types with no read/write/emit methods to call. Task 5 fills
//! in `Emitter::tick` (one occurrence per tick at offset `at`) when it gives
//! `PortView` an `emit` method.

use core::sync::atomic::{AtomicU32, Ordering};

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;

use crate::edges::{GraphNode, NodeId};
use crate::ports::Event;
use crate::registry::{register_node_type, NodeType, NodeTypeId, NodeTypeRegistry};
use crate::schema::register_event_port;
use crate::view::{PortView, TickCtx};

/// The event payload every test node's event port carries. Its exact shape
/// doesn't matter to Task 4's tests — only that it is a distinct, registered
/// `Reflect` type edges can compare for a match.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
pub(crate) struct NoteMsg {
    pub note: u8,
    pub velocity: u8,
}

// --- Probe -------------------------------------------------------------
//
// params `gain: f32`, `trigger: Event<NoteMsg>`, `bias: f32`; outputs
// `value: f32`. The general-purpose node most compiler tests wire up.

#[derive(Reflect, Component, Default)]
pub(crate) struct ProbeParams {
    pub gain: f32,
    pub trigger: Event<NoteMsg>,
    pub bias: f32,
}

#[derive(Reflect, Default)]
pub(crate) struct ProbeOut {
    pub value: f32,
}

#[derive(Component, Default)]
pub(crate) struct ProbeState;

pub(crate) struct Probe;

impl Probe {
    pub const GAIN: u16 = 0;
    pub const BIAS: u16 = 1;
    pub const OUT_VALUE: u16 = 2; // inputs then outputs, within the kind
    pub const TRIGGER: u16 = 0; // event space is separate
}

impl NodeType for Probe {
    type Params = ProbeParams;
    type Outputs = ProbeOut;
    type State = ProbeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("gain", Probe::GAIN),
        ("bias", Probe::BIAS),
        ("value", Probe::OUT_VALUE),
        ("trigger", Probe::TRIGGER),
    ];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- IntProbe ------------------------------------------------------------
//
// params `count: u32`; outputs `count_out: u32` — exists only to make a
// type mismatch against `Probe.value: f32`.

#[derive(Reflect, Component, Default)]
pub(crate) struct IntProbeParams {
    pub count: u32,
}

#[derive(Reflect, Default)]
pub(crate) struct IntProbeOut {
    pub count_out: u32,
}

#[derive(Component, Default)]
pub(crate) struct IntProbeState;

pub(crate) struct IntProbe;

impl IntProbe {
    pub const COUNT: u16 = 0;
    pub const OUT_COUNT: u16 = 1; // one continuous input, so outputs start at 1
}

impl NodeType for IntProbe {
    type Params = IntProbeParams;
    type Outputs = IntProbeOut;
    type State = IntProbeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] =
        &[("count", IntProbe::COUNT), ("count_out", IntProbe::OUT_COUNT)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Emitter -------------------------------------------------------------
//
// params `at: f32`; outputs `pulse: Event<NoteMsg>`. Task 5 gives it a tick
// body that emits one occurrence at offset `at`; Task 4 only needs it as an
// event *source* for fan-in tests, so tick is a no-op for now.

#[derive(Reflect, Component, Default)]
pub(crate) struct EmitterParams {
    pub at: f32,
}

#[derive(Reflect, Default)]
pub(crate) struct EmitterOut {
    pub pulse: Event<NoteMsg>,
}

#[derive(Component, Default)]
pub(crate) struct EmitterState;

pub(crate) struct Emitter;

impl Emitter {
    pub const AT: u16 = 0;
    pub const OUT_PULSE: u16 = 0; // no continuous ports at all, so event space starts at 0
}

impl NodeType for Emitter {
    type Params = EmitterParams;
    type Outputs = EmitterOut;
    type State = EmitterState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] =
        &[("at", Emitter::AT), ("pulse", Emitter::OUT_PULSE)];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Spawning --------------------------------------------------------------

/// Monotonically increasing across the whole test binary. `NodeId` only
/// needs to be distinct and consistently ordered *within* one compiled
/// graph, so a process-wide counter is simplest and safe under parallel
/// tests (each test builds its own `World`).
static NEXT_NODE_ID: AtomicU32 = AtomicU32::new(0);

fn next_node_id() -> NodeId {
    NodeId(NEXT_NODE_ID.fetch_add(1, Ordering::Relaxed))
}

fn node_type_id<N: NodeType>(world: &World) -> NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered by probe_app")
}

pub(crate) fn spawn_probe(world: &mut World) -> Entity {
    let node_type = node_type_id::<Probe>(world);
    world
        .spawn((GraphNode { id: next_node_id(), node_type }, ProbeParams::default(), ProbeState))
        .id()
}

pub(crate) fn spawn_int_probe(world: &mut World) -> Entity {
    let node_type = node_type_id::<IntProbe>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            IntProbeParams::default(),
            IntProbeState,
        ))
        .id()
}

pub(crate) fn spawn_emitter(world: &mut World) -> Entity {
    let node_type = node_type_id::<Emitter>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            EmitterParams::default(),
            EmitterState,
        ))
        .id()
}

// --- App builder -------------------------------------------------------------

/// A bare `App` with `Probe`, `IntProbe` and `Emitter` registered — everything
/// Task 4's compiler tests need. `compile` only reads the `World`, so this
/// does not need `MinimalPlugins`, a `GraphPlugin` or a fixed timestep; Task 5
/// extends this (or adds sibling builders) once its tests need to tick.
pub(crate) fn probe_app() -> App {
    let mut app = App::new();
    register_node_type::<Probe>(&mut app);
    register_node_type::<IntProbe>(&mut app);
    register_node_type::<Emitter>(&mut app);
    app
}
