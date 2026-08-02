//! Node types and world fixtures for `snapshot`'s tests.
//!
//! Deliberately local rather than reusing `sway-nodes`: those pull the `bevy`
//! facade and `bevy_render` through `sway-geo`, which this crate must not
//! link (see the crate doc). Two node types and a headless `App` is all the
//! read path needs to be tested against.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::name::Name;
use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use bevy_transform::components::Transform;
use sway_graph::{
    EdgeFrom, EdgeTo, EditorPos, GraphNode, GraphPlugin, NoOutputs, NoSlots, NodeId, NodeType,
    NodeTypeId, NodeTypeRegistry, ParamEdge, ParentEdge, PortArena, PortKind, PortView, TickCtx,
    compile, register_node_type,
};

/// Graph tick rate for the fixture app. Matches `sway-graph`'s own test
/// harness; nothing here depends on the value.
const TICK_HZ: f64 = 120.0;

// --- Emit: no inputs, one continuous f32 output. ------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct EmitParams;

#[derive(Reflect, Default)]
pub(crate) struct EmitOut {
    pub value: f32,
}

#[derive(Component, Default)]
pub(crate) struct EmitState;

pub(crate) struct Emit;

impl Emit {
    pub const OUT_VALUE: u16 = 0;
}

impl NodeType for Emit {
    type Params = EmitParams;
    type Outputs = EmitOut;
    type Slots = NoSlots;
    type Produces = ();
    type State = EmitState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("value", Emit::OUT_VALUE)];
    // `spawn_spatial` layers a `Transform` onto this type to stand in for a
    // scene node (snapshot.rs's tree tests); `compile`'s structure pass
    // rejects parenting a non-spatial node type (design §4), so this must be
    // `true` for those fixtures' `ParentEdge`s to validate.
    const SPATIAL: bool = true;

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        ports.write(sway_graph::ContinuousIdx(Emit::OUT_VALUE as u32), 0.75_f32);
    }
}

// --- Recv: one continuous f32 input, no outputs. ------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct RecvParams {
    pub amount: f32,
}

#[derive(Component, Default)]
pub(crate) struct RecvState;

pub(crate) struct Recv;

impl Recv {
    pub const AMOUNT: u16 = 0;
}

impl NodeType for Recv {
    type Params = RecvParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = RecvState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("amount", Recv::AMOUNT)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Fixtures -----------------------------------------------------------

/// Headless `App` with `Emit` and `Recv` registered, warmed up one frame.
///
/// The warm-up matters: Bevy's very first `Time::<Real>` update always
/// reports a zero delta, so without it the fixed-timestep accumulator can
/// never reach its threshold on frame 0 and `graph_tick` never runs. Same
/// recipe as `sway-graph`'s own `headless_app`.
pub(crate) fn app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins(GraphPlugin);
    register_node_type::<Emit>(&mut app);
    register_node_type::<Recv>(&mut app);
    app.update();
    app
}

fn type_id<N: NodeType>(world: &World) -> NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

pub(crate) fn spawn_emit(world: &mut World, id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Emit>(world) },
        EmitParams,
        EmitState,
    ));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn spawn_recv(world: &mut World, id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Recv>(world) },
        RecvParams::default(),
        RecvState,
    ));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn connect(world: &mut World, from: Entity, sp: u16, to: Entity, tp: u16) {
    world.spawn((
        ParamEdge { source_port: sp, target_port: tp, kind: PortKind::Continuous },
        EdgeFrom(from),
        EdgeTo(to),
    ));
}

/// Compiles the world's graph and resizes the arena to match. Call after
/// every structural change, exactly as `sway-app` does.
pub(crate) fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("the fixture graph must compile");
    app.world_mut()
        .resource_mut::<PortArena>()
        .resize(compiled.continuous_len, compiled.events_len);
    app.world_mut().insert_resource(compiled);
}

/// A graph node that is also a scene entity: carries a `Transform`, and so
/// lands in the tree's `Scene` group (design §8).
///
/// `ChildOf` is inserted directly so the relationship is visible even before
/// a `compile()`, but `compile`'s structure pass (crates/sway-graph/src/
/// compile.rs, "apply structure") unconditionally rewrites every `GraphNode`
/// entity's `ChildOf` from its `ParentEdge`s, clearing it when none exists --
/// so a `ParentEdge` is spawned too, or the relationship would vanish the
/// moment a caller recompiles.
pub(crate) fn spawn_spatial(world: &mut World, id: u32, parent: Option<Entity>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Emit>(world) },
        EmitParams,
        EmitState,
        Transform::default(),
    ));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    let child = entity.id();
    if let Some(parent) = parent {
        world.spawn((ParentEdge, EdgeFrom(child), EdgeTo(parent)));
    }
    child
}

/// A plain, non-graph entity carrying a `Name` -- stands in for the camera
/// and light `sway-app`'s `setup_scene` spawns outside the graph.
pub(crate) fn spawn_named_spatial(world: &mut World, name: &str) -> Entity {
    world.spawn((Name::new(name.to_string()), Transform::default())).id()
}
