//! Node types and world fixtures for `snapshot`'s tests.
//!
//! Deliberately local rather than reusing `sway-nodes`: those pull the `bevy`
//! facade and `bevy_render` through `sway-geo`, which this crate must not
//! link (see the crate doc). A handful of node types and a headless `App` is
//! all the read path needs to be tested against.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::name::Name;
use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use bevy_transform::components::Transform;
use sway_graph::{
    compile, register_node_type, register_product, Edge, EdgeFrom, EdgeTo, EditorPos, Endpoint,
    GraphNode, GraphPlugin, NodeId, NodeType, NodeTypeId, NodeTypeRegistry, PortArena, PortView,
    Product, Spatial, TickCtx,
};

/// Graph tick rate for the fixture app. Matches `sway-graph`'s own test
/// harness; nothing here depends on the value.
const TICK_HZ: f64 = 120.0;

// --- Emit: no inlets, one value outlet (f32). ---------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct EmitInlets {}

#[derive(Reflect, Default)]
pub(crate) struct EmitOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub(crate) struct EmitState;

pub(crate) struct Emit;

impl Emit {
    pub const OUT_VALUE: u16 = 0;
}

impl NodeType for Emit {
    type Inlets = EmitInlets;
    type Outlets = EmitOutlets;
    type State = EmitState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("value", Emit::OUT_VALUE)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        ports.write(Emit::OUT_VALUE, 0.75_f32);
    }
}

// --- Recv: one value inlet (f32), no outlets. ----------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct RecvInlets {
    pub amount: f32,
}

#[derive(Reflect, Default)]
pub(crate) struct RecvOutlets {}

#[derive(Component, Default)]
pub(crate) struct RecvState;

pub(crate) struct Recv;

impl Recv {
    pub const AMOUNT: u16 = 0;
}

impl NodeType for Recv {
    type Inlets = RecvInlets;
    type Outlets = RecvOutlets;
    type State = RecvState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("amount", Recv::AMOUNT)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Producer / Consumer: a non-spatial Product edge. --------------------

/// A capability distinct from `Spatial`, so a `Product` edge and a `Spatial`
/// edge are told apart in the snapshot's edge-kind tests.
#[derive(Reflect, Default)]
pub(crate) struct Blob;

#[derive(Reflect, Component, Default)]
pub(crate) struct ProducerInlets {}

#[derive(Reflect, Default)]
pub(crate) struct ProducerOutlets {
    pub blob: Product<Blob>,
}

#[derive(Component, Default)]
pub(crate) struct ProducerState;

pub(crate) struct Producer;

impl Producer {
    pub const OUT_BLOB: u16 = 0;
}

impl NodeType for Producer {
    type Inlets = ProducerInlets;
    type Outlets = ProducerOutlets;
    type State = ProducerState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("blob", Producer::OUT_BLOB)];

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

#[derive(Reflect, Component, Default)]
pub(crate) struct ConsumerInlets {
    pub input: Product<Blob>,
}

#[derive(Reflect, Default)]
pub(crate) struct ConsumerOutlets {}

#[derive(Component, Default)]
pub(crate) struct ConsumerState;

pub(crate) struct Consumer;

impl Consumer {
    pub const INPUT: u16 = 0;
}

impl NodeType for Consumer {
    type Inlets = ConsumerInlets;
    type Outlets = ConsumerOutlets;
    type State = ConsumerState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("input", Consumer::INPUT)];

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Group: a variadic Spatial inlet and a Spatial outlet. ---------------
//
// The one node type that stands in for both "scene node" (`spawn_spatial`
// layers a `Transform` onto it) and "parenting" (a `Product<Spatial>` edge
// between two of them is what `compile` reads to apply `ChildOf` -- design
// §3/§4).

#[derive(Reflect, Component, Default)]
pub(crate) struct GroupInlets {
    pub children: Vec<Product<Spatial>>,
}

#[derive(Reflect, Default)]
pub(crate) struct GroupOutlets {
    pub spatial: Product<Spatial>,
}

#[derive(Component, Default)]
pub(crate) struct GroupState;

pub(crate) struct Group;

impl Group {
    pub const CHILDREN: u16 = 0;
    pub const OUT_SPATIAL: u16 = 1;
}

impl NodeType for Group {
    type Inlets = GroupInlets;
    type Outlets = GroupOutlets;
    type State = GroupState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("children", Group::CHILDREN), ("spatial", Group::OUT_SPATIAL)];

    fn register(app: &mut App) {
        register_product::<Spatial>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Fixtures -------------------------------------------------------------

/// Headless `App` with every fixture node type registered, warmed up one
/// frame.
///
/// The warm-up matters: Bevy's very first `Time::<Real>` update always
/// reports a zero delta, so without it the fixed-timestep accumulator can
/// never reach its threshold on frame 0 and `graph_tick` never runs. Same
/// recipe as `sway-graph`'s own test harness.
pub(crate) fn app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins(GraphPlugin);
    register_node_type::<Emit>(&mut app);
    register_node_type::<Recv>(&mut app);
    register_node_type::<Producer>(&mut app);
    register_node_type::<Consumer>(&mut app);
    register_node_type::<Group>(&mut app);
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
        EmitInlets {},
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
        RecvInlets::default(),
        RecvState,
    ));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn spawn_producer(world: &mut World, id: u32) -> Entity {
    world
        .spawn((
            GraphNode { id: NodeId(id), node_type: type_id::<Producer>(world) },
            ProducerInlets {},
            ProducerState,
        ))
        .id()
}

pub(crate) fn spawn_consumer(world: &mut World, id: u32) -> Entity {
    world
        .spawn((
            GraphNode { id: NodeId(id), node_type: type_id::<Consumer>(world) },
            ConsumerInlets::default(),
            ConsumerState,
        ))
        .id()
}

/// `children` is sized here, because a variadic field's slot count comes
/// from the instance.
pub(crate) fn spawn_group(world: &mut World, id: u32, children: usize) -> Entity {
    world
        .spawn((
            GraphNode { id: NodeId(id), node_type: type_id::<Group>(world) },
            GroupInlets { children: vec![Product::<Spatial>::default(); children] },
            GroupState,
        ))
        .id()
}

pub(crate) fn connect(world: &mut World, from: Entity, from_field: u16, to: Entity, to_field: u16) {
    connect_at(world, from, from_field, to, to_field, 0);
}

pub(crate) fn connect_at(
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

/// Compiles the world's graph and resizes the arena to match. Call after
/// every structural change, exactly as `sway-app` does.
pub(crate) fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("the fixture graph must compile");
    app.world_mut().resource_mut::<PortArena>().resize(compiled.slots_len);
    app.world_mut().insert_resource(compiled);
}

/// A graph node that is also a scene entity: carries a `Transform`, and so
/// lands in the tree's `Scene` group (design §8). Built on `Group`, whose
/// `Product<Spatial>` outlet is what a parent's edge actually needs (a plain
/// value/event node has no such port to connect).
///
/// When `parent` is given, the parent's own `children` field is grown by one
/// slot and a genuine `Product<Spatial>` edge connects this node to it --
/// `compile`'s structure pass (crates/sway-graph/src/compile.rs, "apply
/// structure") derives `ChildOf` from exactly that edge, not from any
/// component set directly, so a manually-inserted `ChildOf` would be wiped
/// the moment a caller recompiles.
pub(crate) fn spawn_spatial(world: &mut World, id: u32, parent: Option<Entity>) -> Entity {
    let entity = spawn_group(world, id, 0);
    world.entity_mut(entity).insert(Transform::default());
    if let Some(parent) = parent {
        let index = {
            let mut inlets =
                world.get_mut::<GroupInlets>(parent).expect("parent is a Group node");
            inlets.children.push(Product::default());
            (inlets.children.len() - 1) as u16
        };
        connect_at(world, entity, Group::OUT_SPATIAL, parent, Group::CHILDREN, index);
    }
    entity
}

/// A plain, non-graph entity carrying a `Name` -- stands in for the camera
/// and light `sway-app`'s `setup_scene` spawns outside the graph.
pub(crate) fn spawn_named_spatial(world: &mut World, name: &str) -> Entity {
    world.spawn((Name::new(name.to_string()), Transform::default())).id()
}

/// The `NodeId`s of the parenting fixture: a `Group` child feeding a `Group`
/// parent's `children[0]` through a `Product<Spatial>` edge.
pub(crate) struct ParentingIds {
    pub child: NodeId,
    pub parent: NodeId,
}

/// Builds a graph exercising every `EdgeKind`: a `Value` edge (`Emit` ->
/// `Recv`), a non-spatial `Product` edge (`Producer` -> `Consumer`), and a
/// `Spatial` product edge -- a `Group` child parented under a `Group`.
pub(crate) fn fixture_with_parenting() -> (App, ParentingIds) {
    let mut app = app();
    let world = app.world_mut();

    let emit = spawn_emit(world, 0, None);
    let recv = spawn_recv(world, 1, None);
    connect(world, emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);

    let producer = spawn_producer(world, 2);
    let consumer = spawn_consumer(world, 3);
    connect(world, producer, Producer::OUT_BLOB, consumer, Consumer::INPUT);

    let parent = spawn_group(world, 4, 0);
    let child = spawn_spatial(world, 5, Some(parent));

    recompile(&mut app);
    app.update();

    let parent_id = app.world().get::<GraphNode>(parent).expect("parent is a node").id;
    let child_id = app.world().get::<GraphNode>(child).expect("child is a node").id;

    (app, ParentingIds { child: child_id, parent: parent_id })
}
