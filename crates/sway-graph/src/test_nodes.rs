//! Engine-only node fixtures. Deliberately not real nodes: these exist to
//! exercise the contract, not to do anything musical.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;

use crate::compile::{compile, CompiledGraph};
use crate::edges::{Edge, EdgeFrom, EdgeTo, Endpoint, GraphNode, NodeId};
use crate::ports::{Events, PortArena, Product, Spatial};
use crate::registry::{register_node_type, NodeType, NodeTypeId, NodeTypeRegistry};
use crate::schema::{register_events, register_product};
use crate::tick::GraphPlugin;
use crate::view::{PortView, TickCtx};

/// A capability no real node uses, for slot-typing tests.
#[derive(Reflect, Default)]
pub struct Blob;

/// A second one, so a mismatch names two real capabilities.
#[derive(Reflect, Default)]
pub struct Sludge;

#[derive(Reflect, Default, Debug, Clone, PartialEq)]
pub struct Ping {
    pub seq: u32,
}

// --- Gain: two value inlets, one value outlet -------------------------

#[derive(Reflect, Component, Default)]
pub struct GainInlets {
    pub gain: f32,
    pub bias: f32,
}

#[derive(Reflect, Default)]
pub struct GainOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct GainState;

pub struct Gain;

impl Gain {
    pub const GAIN: u16 = 0;
    pub const BIAS: u16 = 1;
    pub const OUT_VALUE: u16 = 2; // outlets follow inlets in one field space
}

impl NodeType for Gain {
    type Inlets = GainInlets;
    type Outlets = GainOutlets;
    type State = GainState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("gain", Gain::GAIN), ("bias", Gain::BIAS), ("value", Gain::OUT_VALUE)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let gain: f32 = ports.read(Gain::GAIN);
        let bias: f32 = ports.read(Gain::BIAS);
        ports.write(Gain::OUT_VALUE, gain * bias);
    }
}

// --- Sum: one variadic value inlet ------------------------------------

#[derive(Reflect, Component, Default)]
pub struct SumInlets {
    pub terms: Vec<f32>,
}

#[derive(Reflect, Default)]
pub struct SumOutlets {
    pub total: f32,
}

#[derive(Component, Default)]
pub struct SumState;

pub struct Sum;

impl Sum {
    pub const TERMS: u16 = 0;
    pub const OUT_TOTAL: u16 = 1;
}

impl NodeType for Sum {
    type Inlets = SumInlets;
    type Outlets = SumOutlets;
    type State = SumState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("terms", Sum::TERMS), ("total", Sum::OUT_TOTAL)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        // The combining rule lives here, in the node, not in the engine.
        let mut total = 0.0;
        for i in 0..ports.len(Sum::TERMS) {
            total += ports.read_at::<f32>(Sum::TERMS, i as u16);
        }
        ports.write(Sum::OUT_TOTAL, total);
    }
}

// --- Emitter / Sink: event out, event in ------------------------------

#[derive(Reflect, Component, Default)]
pub struct EmitterInlets {
    pub period: f32,
}

#[derive(Reflect, Default)]
pub struct EmitterOutlets {
    pub pulse: Events<Ping>,
}

#[derive(Component, Default)]
pub struct EmitterState {
    pub seq: u32,
}

pub struct Emitter;

impl Emitter {
    pub const PERIOD: u16 = 0;
    pub const OUT_PULSE: u16 = 1;
}

impl NodeType for Emitter {
    type Inlets = EmitterInlets;
    type Outlets = EmitterOutlets;
    type State = EmitterState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("period", Emitter::PERIOD), ("pulse", Emitter::OUT_PULSE)];

    fn register(app: &mut App) {
        register_events::<Ping>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let offset = ports.read::<f32>(Emitter::PERIOD);
        let seq = {
            let mut state = world.get_mut::<EmitterState>(node).expect("state");
            state.seq += 1;
            state.seq
        };
        ports.emit(Emitter::OUT_PULSE, offset, Ping { seq });
    }
}

#[derive(Reflect, Component, Default)]
pub struct SinkInlets {
    pub pulse: Events<Ping>,
}

#[derive(Reflect, Default)]
pub struct SinkOutlets {}

#[derive(Component, Default)]
pub struct SinkState;

pub struct Sink;

impl Sink {
    pub const PULSE: u16 = 0;
}

impl NodeType for Sink {
    type Inlets = SinkInlets;
    type Outlets = SinkOutlets;
    type State = SinkState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("pulse", Sink::PULSE)];

    fn register(app: &mut App) {
        register_events::<Ping>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

// --- Producer / Consumer: Product edges and the cook ------------------

#[derive(Reflect, Component, Default)]
pub struct ProducerInlets {
    pub scale: f32,
}

#[derive(Reflect, Default)]
pub struct ProducerOutlets {
    pub blob: Product<Blob>,
}

#[derive(Component, Default)]
pub struct ProducerState {
    pub cooks: u32,
}

pub struct Producer;

impl Producer {
    pub const SCALE: u16 = 0;
    pub const OUT_BLOB: u16 = 1;
}

impl NodeType for Producer {
    type Inlets = ProducerInlets;
    type Outlets = ProducerOutlets;
    type State = ProducerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("scale", Producer::SCALE), ("blob", Producer::OUT_BLOB)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _ports: &PortView) {
        world.get_mut::<ProducerState>(node).expect("state").cooks += 1;
    }
}

/// Produces `Sludge`, so a mismatch names two real capabilities.
#[derive(Reflect, Default)]
pub struct SludgeOutlets {
    pub sludge: Product<Sludge>,
}

pub struct SludgeSource;

impl SludgeSource {
    pub const SCALE: u16 = 0;
    pub const OUT_SLUDGE: u16 = 1;
}

impl NodeType for SludgeSource {
    type Inlets = ProducerInlets;
    type Outlets = SludgeOutlets;
    type State = ProducerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("scale", SludgeSource::SCALE), ("sludge", SludgeSource::OUT_SLUDGE)];

    fn register(app: &mut App) {
        register_product::<Sludge>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

#[derive(Reflect, Component, Default)]
pub struct ConsumerInlets {
    pub input: Product<Blob>,
    pub scale: f32,
}

#[derive(Reflect, Default)]
pub struct ConsumerOutlets {
    pub blob: Product<Blob>,
}

#[derive(Component, Default)]
pub struct ConsumerState {
    pub cooks: u32,
}

pub struct Consumer;

impl Consumer {
    pub const INPUT: u16 = 0;
    pub const SCALE: u16 = 1;
    pub const OUT_BLOB: u16 = 2;
}

impl NodeType for Consumer {
    type Inlets = ConsumerInlets;
    type Outlets = ConsumerOutlets;
    type State = ConsumerState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("input", Consumer::INPUT),
        ("scale", Consumer::SCALE),
        ("blob", Consumer::OUT_BLOB),
    ];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _ports: &PortView) {
        world.get_mut::<ConsumerState>(node).expect("state").cooks += 1;
    }
}

// --- Group: a variadic Spatial inlet and a Spatial outlet -------------

#[derive(Reflect, Component, Default)]
pub struct GroupInlets {
    pub children: Vec<Product<Spatial>>,
    pub rotation_y: f32,
}

#[derive(Reflect, Default)]
pub struct GroupOutlets {
    pub spatial: Product<Spatial>,
}

#[derive(Component, Default)]
pub struct GroupState;

pub struct Group;

impl Group {
    pub const CHILDREN: u16 = 0;
    pub const ROTATION_Y: u16 = 1;
    pub const OUT_SPATIAL: u16 = 2;
}

impl NodeType for Group {
    type Inlets = GroupInlets;
    type Outlets = GroupOutlets;
    type State = GroupState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("children", Group::CHILDREN),
        ("rotation_y", Group::ROTATION_Y),
        ("spatial", Group::OUT_SPATIAL),
    ];

    fn register(app: &mut App) {
        register_product::<Spatial>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

// --- Helpers ----------------------------------------------------------

/// Graph tick rate used by every test app. Matches
/// `crates/sway-app/src/graph.rs`'s M0 provisional value.
const TICK_HZ: f64 = 120.0;

pub fn engine_app() -> App {
    let mut app = App::new();
    // `bevy_time::TimePlugin` alone leaves `FixedUpdate` driven by wall-clock
    // time, so a fast-running test's `app.update()` calls may accumulate
    // less than one fixed timestep and never run `graph_tick` at all.
    // Pinning the fixed timestep and stepping it manually makes each
    // `app.update()` run `graph_tick` exactly once, deterministically.
    app.add_plugins(bevy_time::TimePlugin)
        .insert_resource(bevy_time::Time::<bevy_time::Fixed>::from_hz(TICK_HZ))
        .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1));
    app.add_plugins(GraphPlugin);
    register_node_type::<Gain>(&mut app);
    register_node_type::<Sum>(&mut app);
    register_node_type::<Emitter>(&mut app);
    register_node_type::<Sink>(&mut app);
    register_node_type::<Producer>(&mut app);
    register_node_type::<SludgeSource>(&mut app);
    register_node_type::<Consumer>(&mut app);
    register_node_type::<Group>(&mut app);
    // Frame 0 runs no fixed tick (the accumulator is empty until real time
    // has advanced once), so one warm-up `update()` is burned here — no
    // nodes exist yet, and `graph_tick` no-ops with no `CompiledGraph` —
    // making the caller's first `app.update()` after `recompile` run exactly
    // one fixed tick.
    app.update();
    app
}

fn type_id_of<N: NodeType>(world: &World) -> NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

fn next_id(world: &mut World) -> NodeId {
    let mut query = world.query::<&GraphNode>();
    NodeId(query.iter(world).count() as u32)
}

pub fn spawn_gain(world: &mut World, gain: f32, bias: f32) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Gain>(world);
    world
        .spawn((GraphNode { id, node_type }, GainInlets { gain, bias }, GainState))
        .id()
}

pub fn spawn_sum(world: &mut World, terms: Vec<f32>) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Sum>(world);
    world
        .spawn((GraphNode { id, node_type }, SumInlets { terms }, SumState))
        .id()
}

pub fn spawn_emitter(world: &mut World, period: f32) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Emitter>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            EmitterInlets { period },
            EmitterState::default(),
        ))
        .id()
}

pub fn spawn_sink(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Sink>(world);
    world
        .spawn((GraphNode { id, node_type }, SinkInlets::default(), SinkState))
        .id()
}

pub fn spawn_producer(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Producer>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ProducerInlets::default(),
            ProducerState::default(),
        ))
        .id()
}

pub fn spawn_sludge_source(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<SludgeSource>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ProducerInlets::default(),
            ProducerState::default(),
        ))
        .id()
}

pub fn spawn_consumer(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Consumer>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ConsumerInlets::default(),
            ConsumerState::default(),
        ))
        .id()
}

/// `children` is sized here, because a variadic field's slot count comes
/// from the instance.
pub fn spawn_group(world: &mut World, children: usize) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Group>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            GroupInlets {
                children: vec![Product::<Spatial>::default(); children],
                rotation_y: 0.0,
            },
            GroupState,
        ))
        .id()
}

pub fn connect(world: &mut World, from: Entity, from_field: u16, to: Entity, to_field: u16) -> Entity {
    connect_at(world, from, from_field, to, to_field, 0)
}

pub fn connect_at(
    world: &mut World,
    from: Entity,
    from_field: u16,
    to: Entity,
    to_field: u16,
    to_index: u16,
) -> Entity {
    world
        .spawn((
            Edge {
                from: Endpoint::field(from_field),
                to: Endpoint { field: to_field, index: to_index },
            },
            EdgeFrom(from),
            EdgeTo(to),
        ))
        .id()
}

pub fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("compiles");
    let slots_len = compiled.slots_len;
    app.world_mut().resource_mut::<PortArena>().resize(slots_len);
    app.world_mut().insert_resource(compiled);
}

/// Reads a node's value slot out of the arena, by field ordinal.
pub fn port_value(app: &App, node: Entity, field: u16) -> f32 {
    let compiled = app.world().resource::<CompiledGraph>();
    let plan = compiled
        .plans
        .iter()
        .find(|p| p.entity == node)
        .expect("node is compiled");
    let slot = plan.base + plan.field_offsets[field as usize];
    app.world().resource::<PortArena>().values[slot]
        .try_downcast_ref::<f32>()
        .copied()
        .expect("slot holds an f32")
}

/// The occurrences on a node's event slot this tick.
pub fn event_offsets(app: &App, node: Entity, field: u16) -> Vec<f32> {
    let compiled = app.world().resource::<CompiledGraph>();
    let plan = compiled
        .plans
        .iter()
        .find(|p| p.entity == node)
        .expect("node is compiled");
    let slot = plan.base + plan.field_offsets[field as usize];
    app.world().resource::<PortArena>().values[slot]
        .try_downcast_ref::<Events<Ping>>()
        .expect("slot holds Events<Ping>")
        .occurrences
        .iter()
        .map(|o| o.offset)
        .collect()
}
