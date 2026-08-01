//! Shared node types for `sway-graph`'s own test suite.
//!
//! Shared across Tasks 3, 4 and 5 — whichever lands first creates this file,
//! the others extend it. Task 3 used a private `Probe` defined inline in
//! `registry.rs`'s own test module; Task 4 is the first task that needs the
//! *same* node type visible from another module's tests, so it moves here.
//!
//! Task 4 added `Probe`, `IntProbe`, `Emitter` and the spawners/app builder
//! its compiler tests need. Task 5 adds `Gain`, `Sink`, the wiring helpers
//! (`connect`, `connect_event`), the assertion helpers (`recompile`,
//! `port_value`, `event_count`, `sink_offsets`) and the `GraphPlugin`-backed
//! app builders (`gain_app`, `emitter_app`), and replaces `Emitter::tick`'s
//! no-op body now that `PortView` has real accessors.

use core::sync::atomic::{AtomicU32, Ordering};

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::{Reflect, TypePath};
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};

use crate::compile::compile;
use crate::edges::{EdgeFrom, EdgeTo, GraphNode, NodeId, NodeRuntime, ParamEdge, PortKind};
use crate::ports::{ContinuousIdx, Event, EventIdx, PortArena};
use crate::registry::{NodeType, NodeTypeId, NodeTypeRegistry, register_node_type};
use crate::schema::register_event_port;
use crate::slots::{NoOutputs, NoSlots, Slot, register_slot};
use crate::tick::GraphPlugin;
use crate::view::{PortView, SlotView, TickCtx};

/// Graph tick rate used by every headless test app. Matches
/// `crates/sway-app/src/graph.rs`'s M0 provisional value (spec §11).
pub(crate) const TICK_HZ: f64 = 120.0;

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
    type Slots = NoSlots;
    type Produces = ();
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
    type Slots = NoSlots;
    type Produces = ();
    type State = IntProbeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("count", IntProbe::COUNT),
        ("count_out", IntProbe::OUT_COUNT),
    ];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Emitter -------------------------------------------------------------
//
// params `at: f32`; outputs `pulse: Event<NoteMsg>`. Emits one occurrence
// per tick, at offset `at`.

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
    type Slots = NoSlots;
    type Produces = ();
    type State = EmitterState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] =
        &[("at", Emitter::AT), ("pulse", Emitter::OUT_PULSE)];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
    }

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let at: f32 = ports.read(ContinuousIdx(Emitter::AT as u32));
        ports.emit(EventIdx(Emitter::OUT_PULSE as u32), at, NoteMsg::default());
    }
}

// --- Gain ------------------------------------------------------------------
//
// params `gain: f32`, `bias: f32`; outputs `value: f32`; tick writes
// `gain * bias`. The general-purpose node Task 5's tick tests wire up.

#[derive(Reflect, Component, Default)]
pub(crate) struct GainParams {
    pub gain: f32,
    pub bias: f32,
}

#[derive(Reflect, Default)]
pub(crate) struct GainOut {
    pub value: f32,
}

#[derive(Component, Default)]
pub(crate) struct GainState;

pub(crate) struct Gain;

impl Gain {
    pub const GAIN: u16 = 0;
    pub const BIAS: u16 = 1;
    pub const OUT_VALUE: u16 = 2;
}

impl NodeType for Gain {
    type Params = GainParams;
    type Outputs = GainOut;
    type Slots = NoSlots;
    type Produces = ();
    type State = GainState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("gain", Gain::GAIN),
        ("bias", Gain::BIAS),
        ("value", Gain::OUT_VALUE),
    ];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let gain: f32 = ports.read(ContinuousIdx(Gain::GAIN as u32));
        let bias: f32 = ports.read(ContinuousIdx(Gain::BIAS as u32));
        ports.write(ContinuousIdx(Gain::OUT_VALUE as u32), gain * bias);
    }
}

// --- Sink --------------------------------------------------------------
//
// params `pulse: Event<NoteMsg>`; outputs none; state records every
// occurrence's offset, this tick only (spec §4: events clear each tick).

#[derive(Reflect, Component, Default)]
pub(crate) struct SinkParams {
    pub pulse: Event<NoteMsg>,
}

#[derive(Reflect, Default)]
pub(crate) struct SinkOut {}

#[derive(Component, Default)]
pub(crate) struct SinkState {
    pub offsets: Vec<f32>,
}

pub(crate) struct Sink;

impl Sink {
    pub const IN_PULSE: u16 = 0;
}

impl NodeType for Sink {
    type Params = SinkParams;
    type Outputs = SinkOut;
    type Slots = NoSlots;
    type Produces = ();
    type State = SinkState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("pulse", Sink::IN_PULSE)];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let offsets: Vec<f32> = ports
            .events::<NoteMsg>(EventIdx(Sink::IN_PULSE as u32))
            .map(|e| e.offset)
            .collect();
        if let Some(mut state) = world.get_mut::<SinkState>(node) {
            state.offsets = offsets;
        }
    }
}

// --- Blob / Source / SinkGeo / Group ----------------------------------------
//
// Task 4's structure-pass fixtures. `sway-graph` cannot depend on `sway-geo`,
// so these carry their own capability marker and produced component rather
// than the real `Geometry`/`Attribute` types.

/// A stand-in capability. `sway-graph` cannot depend on `sway-geo`, so its
/// structural tests carry their own marker and their own produced component.
#[derive(TypePath)]
pub(crate) struct Blob;

/// What a `Source`/`SinkGeo` cook writes. Its change tick is what
/// `produced_change_tick` reports.
#[derive(Component, Default, Debug, Clone, PartialEq)]
pub(crate) struct BlobData(pub u32);

#[derive(Reflect, Component, Default)]
pub(crate) struct SourceParams {
    pub seed: f32,
}

#[derive(Component, Default)]
pub(crate) struct SourceState;

pub(crate) struct Source;

impl Source {
    pub(crate) const SEED: u16 = 0;
}

impl NodeType for Source {
    type Params = SourceParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = Blob;
    type State = SourceState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("seed", Self::SEED)];
    const COOKS: bool = true;

    fn register(_app: &mut App) {}

    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _slots: &SlotView) {
        let seed = world.get::<SourceParams>(node).map(|p| p.seed).unwrap_or(0.0);
        world.entity_mut(node).insert(BlobData(seed as u32));
        world.resource_mut::<CookCounter>().0 += 1;
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<BlobData>()
            .map(|t| t.changed)
    }
}

/// A second, distinct capability — never equal to `Blob`. Exists so a
/// genuine `SlotTypeMismatch` (two non-unit capabilities that simply differ)
/// can be tested, as opposed to `SourceProducesNothing` (a `()` producer),
/// which the check order in `structure::validate` reaches first for any
/// non-producing source.
#[derive(TypePath)]
pub(crate) struct Sludge;

#[derive(Reflect, Component, Default)]
pub(crate) struct SludgeSourceParams {
    pub seed: f32,
}

#[derive(Component, Default)]
pub(crate) struct SludgeSourceState;

pub(crate) struct SludgeSource;

impl SludgeSource {
    pub(crate) const SEED: u16 = 0;
}

impl NodeType for SludgeSource {
    type Params = SludgeSourceParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = Sludge;
    type State = SludgeSourceState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("seed", Self::SEED)];
    const COOKS: bool = true;

    fn register(_app: &mut App) {}

    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _slots: &SlotView) {
        let seed = world.get::<SludgeSourceParams>(node).map(|p| p.seed).unwrap_or(0.0);
        world.entity_mut(node).insert(BlobData(seed as u32));
        world.resource_mut::<CookCounter>().0 += 1;
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<BlobData>()
            .map(|t| t.changed)
    }
}

#[derive(Reflect, Default)]
pub(crate) struct SinkGeoSlots {
    pub input: Slot<Blob>,
}

#[derive(Reflect, Component, Default)]
pub(crate) struct SinkGeoParams {
    pub scale: f32,
}

#[derive(Component, Default)]
pub(crate) struct SinkGeoState;

pub(crate) struct SinkGeo;

impl SinkGeo {
    pub(crate) const SCALE: u16 = 0;
    pub(crate) const IN_INPUT: u16 = 0;
}

impl NodeType for SinkGeo {
    type Params = SinkGeoParams;
    type Outputs = NoOutputs;
    type Slots = SinkGeoSlots;
    type Produces = Blob;
    type State = SinkGeoState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("scale", Self::SCALE)];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("input", Self::IN_INPUT)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_slot::<Blob>(app);
    }

    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        let upstream = slots
            .source(SinkGeo::IN_INPUT)
            .and_then(|src| world.get::<BlobData>(src))
            .map(|b| b.0)
            .unwrap_or(0);
        let scale = world.get::<SinkGeoParams>(node).map(|p| p.scale).unwrap_or(1.0);
        world
            .entity_mut(node)
            .insert(BlobData(upstream * scale as u32));
        world.resource_mut::<CookCounter>().0 += 1;
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<BlobData>()
            .map(|t| t.changed)
    }
}

#[derive(Reflect, Component, Default)]
pub(crate) struct GroupParams {
    pub y: f32,
}

#[derive(Component, Default)]
pub(crate) struct GroupState;

pub(crate) struct Group;

impl NodeType for Group {
    type Params = GroupParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = GroupState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("y", 0)];
    const SPATIAL: bool = true;

    fn register(_app: &mut App) {}
    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
}

/// Counts cooks, so the gate's negative assertions have something to assert
/// on rather than an output that merely happens to be unchanged (§7).
#[derive(Resource, Default)]
pub(crate) struct CookCounter(pub u32);

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
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            ProbeParams::default(),
            ProbeState,
        ))
        .id()
}

pub(crate) fn spawn_int_probe(world: &mut World) -> Entity {
    let node_type = node_type_id::<IntProbe>(world);
    world
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            IntProbeParams::default(),
            IntProbeState,
        ))
        .id()
}

pub(crate) fn spawn_emitter(world: &mut World) -> Entity {
    let node_type = node_type_id::<Emitter>(world);
    world
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            EmitterParams::default(),
            EmitterState,
        ))
        .id()
}

pub(crate) fn spawn_emitter_at(world: &mut World, at: f32) -> Entity {
    let node_type = node_type_id::<Emitter>(world);
    world
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            EmitterParams { at },
            EmitterState,
        ))
        .id()
}

pub(crate) fn spawn_gain(world: &mut World, gain: f32, bias: f32) -> Entity {
    let node_type = node_type_id::<Gain>(world);
    world
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            GainParams { gain, bias },
            GainState,
        ))
        .id()
}

pub(crate) fn spawn_source(world: &mut World) -> Entity {
    let node_type = node_type_id::<Source>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            SourceParams { seed: 1.0 },
            SourceState,
        ))
        .id()
}

pub(crate) fn spawn_sludge_source(world: &mut World) -> Entity {
    let node_type = node_type_id::<SludgeSource>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            SludgeSourceParams { seed: 1.0 },
            SludgeSourceState,
        ))
        .id()
}

pub(crate) fn spawn_sinkgeo(world: &mut World) -> Entity {
    let node_type = node_type_id::<SinkGeo>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            SinkGeoParams { scale: 1.0 },
            SinkGeoState,
        ))
        .id()
}

pub(crate) fn spawn_group(world: &mut World) -> Entity {
    let node_type = node_type_id::<Group>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            GroupParams::default(),
            GroupState,
        ))
        .id()
}

pub(crate) fn spawn_sink(world: &mut World) -> Entity {
    let node_type = node_type_id::<Sink>(world);
    world
        .spawn((
            GraphNode {
                id: next_node_id(),
                node_type,
            },
            SinkParams::default(),
            SinkState::default(),
        ))
        .id()
}

// --- Wiring ------------------------------------------------------------------

/// Spawns a continuous `ParamEdge` from `src`'s output ordinal `src_port` to
/// `dst`'s input ordinal `dst_port`.
pub(crate) fn connect(
    world: &mut World,
    src: Entity,
    src_port: u16,
    dst: Entity,
    dst_port: u16,
) -> Entity {
    world
        .spawn((
            ParamEdge {
                source_port: src_port,
                target_port: dst_port,
                kind: PortKind::Continuous,
            },
            EdgeFrom(src),
            EdgeTo(dst),
        ))
        .id()
}

/// Spawns an event `ParamEdge` from `src`'s output ordinal `src_port` to
/// `dst`'s input ordinal `dst_port`.
pub(crate) fn connect_event(
    world: &mut World,
    src: Entity,
    src_port: u16,
    dst: Entity,
    dst_port: u16,
) -> Entity {
    world
        .spawn((
            ParamEdge {
                source_port: src_port,
                target_port: dst_port,
                kind: PortKind::Event,
            },
            EdgeFrom(src),
            EdgeTo(dst),
        ))
        .id()
}

// --- Assertions ----------------------------------------------------------------

/// Runs `compile`, inserts the result as a resource, and resizes `PortArena`
/// to the new layout. `compile` deliberately does neither (Task 4's design:
/// the compiler produces the layout, the runner applies it) — this is that
/// application step, for tests.
pub(crate) fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("compiles");
    let continuous_len = compiled.continuous_len;
    let events_len = compiled.events_len;
    app.world_mut()
        .resource_mut::<PortArena>()
        .resize(continuous_len, events_len);
    app.world_mut().insert_resource(compiled);
}

/// Reads a node's continuous port (by ordinal) straight from the arena, via
/// its compiled `NodeRuntime` base.
pub(crate) fn port_value(app: &App, node: Entity, ordinal: u16) -> f32 {
    let base = app
        .world()
        .get::<NodeRuntime>(node)
        .expect("node is compiled")
        .continuous_base;
    let arena = app.world().resource::<PortArena>();
    *arena.continuous[base + ordinal as usize]
        .try_downcast_ref::<f32>()
        .expect("port holds an f32")
}

/// Number of occurrences currently sitting in a node's event port (by
/// ordinal), straight from the arena.
pub(crate) fn event_count(app: &App, node: Entity, ordinal: u16) -> usize {
    let base = app
        .world()
        .get::<NodeRuntime>(node)
        .expect("node is compiled")
        .event_base;
    let arena = app.world().resource::<PortArena>();
    arena.events[base + ordinal as usize].len()
}

/// Every offset `Sink` has recorded this tick, in arrival order.
pub(crate) fn sink_offsets(app: &App, sink: Entity) -> Vec<f32> {
    app.world()
        .get::<SinkState>(sink)
        .expect("sink spawned")
        .offsets
        .clone()
}

// --- App builders -------------------------------------------------------------

/// A bare `App` with `Probe`, `IntProbe` and `Emitter` registered — everything
/// Task 4's compiler tests need. `compile` only reads the `World`, so this
/// does not need a `GraphPlugin` or a fixed timestep.
pub(crate) fn probe_app() -> App {
    let mut app = App::new();
    register_node_type::<Probe>(&mut app);
    register_node_type::<IntProbe>(&mut app);
    register_node_type::<Emitter>(&mut app);
    app
}

/// A headless, tick-capable `App`: `GraphPlugin` plus a fixed timestep driven
/// one step at a time. There is no real `MinimalPlugins` available from
/// `bevy_app` alone — it is only a doc-comment alias for `NoopPluginGroup`,
/// with no `TimePlugin` bundled — and `sway-graph` cannot depend on the
/// `bevy` facade crate that provides the real one (spec §2), so this adds
/// `bevy_time::TimePlugin` directly instead.
///
/// Frame 0 runs no fixed tick (the accumulator is empty until real time has
/// advanced once — see `crates/sway-app/src/graph.rs`'s `headless` for the
/// same recipe), so one warm-up `update()` is burned here before returning,
/// making the caller's first `app.update()` run exactly one fixed tick.
fn headless_app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins(GraphPlugin);
    app
}

/// Headless app with `Gain` registered.
pub(crate) fn gain_app() -> App {
    let mut app = headless_app();
    register_node_type::<Gain>(&mut app);
    app.update();
    app
}

/// Headless app with `Emitter` and `Sink` registered.
pub(crate) fn emitter_app() -> App {
    let mut app = headless_app();
    register_node_type::<Emitter>(&mut app);
    register_node_type::<Sink>(&mut app);
    app.update();
    app
}

/// App with `Probe`, `Source`, `SludgeSource`, `SinkGeo` and `Group`
/// registered — everything Task 4's structure-pass tests need, plus Task 7's
/// cook-pass tests, which do call `app.update()` and need `graph_tick` to
/// actually run. `TimePlugin` plus a single-step `Fixed` timestep is
/// `headless_app`'s recipe (see its doc comment for why `bevy_app` alone does
/// not supply this); the warm-up `update()` burns frame 0's empty
/// accumulator so the caller's first `app.update()` runs exactly one fixed
/// tick.
pub(crate) fn structure_app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins(crate::tick::GraphPlugin);
    app.init_resource::<CookCounter>();
    register_node_type::<Probe>(&mut app);
    register_node_type::<Source>(&mut app);
    register_node_type::<SludgeSource>(&mut app);
    register_node_type::<SinkGeo>(&mut app);
    register_node_type::<Group>(&mut app);
    app.update();
    app
}
