//! Wire-model fixtures for the editor snapshot tests.

use bevy_app::App;
use bevy_ecs::change_detection::{DetectChangesMut, Mut};
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::name::Name;
use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_transform::components::Transform;
use sway_graph::order::{TopologyDirty, rebuild_order};
use sway_graph::{EditorPos, Wire, WiresPlugin, register_wire};

#[derive(Component, Default)]
pub(crate) struct Emit;

impl Emit {
    pub const OUT_VALUE: u16 = 0;
}

#[derive(Component, Default)]
pub(crate) struct Recv;

impl Recv {
    pub const AMOUNT: u16 = 0;
}

#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
struct FloatOut(f32);

#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
struct Gain {
    factor: f32,
}

#[derive(Component)]
#[relationship(relationship_target = DrivesGain)]
struct GainFrom(#[entities] Entity);

#[derive(Component)]
#[relationship_target(relationship = GainFrom)]
struct DrivesGain(Vec<Entity>);

impl Wire for GainFrom {
    type Source = FloatOut;
    type Target = Gain;
    const NAME: &'static str = "amount";

    fn propagate(src: &FloatOut, dst: Mut<Gain>) {
        dst.map_unchanged(|gain| &mut gain.factor).set_if_neq(src.0);
    }
}

pub(crate) fn app() -> App {
    let mut app = App::new();
    app.add_plugins(WiresPlugin);
    register_wire::<GainFrom>(&mut app);
    register_wire::<ChildOf>(&mut app);
    app
}

pub(crate) fn spawn_emit(world: &mut World, _id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((Name::new("Emit"), Emit, FloatOut(0.75)));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn spawn_recv(world: &mut World, _id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((Name::new("Recv"), Recv, Gain::default()));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn connect(
    world: &mut World,
    from: Entity,
    _from_field: u16,
    to: Entity,
    _to_field: u16,
) {
    world.entity_mut(to).insert(GainFrom(from));
}

pub(crate) fn recompile(app: &mut App) {
    app.world_mut().resource_mut::<TopologyDirty>().0 = true;
    rebuild_order(app.world_mut());
}

pub(crate) fn spawn_spatial(
    world: &mut World,
    _id: u32,
    parent: Option<Entity>,
) -> Entity {
    let mut entity = world.spawn(Transform::default());
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    entity.id()
}

pub(crate) fn spawn_named_spatial(world: &mut World, name: &str) -> Entity {
    world
        .spawn((Name::new(name.to_string()), Transform::default()))
        .id()
}

pub(crate) struct ParentingIds {
    pub child: Entity,
    pub parent: Entity,
}

pub(crate) fn fixture_with_parenting() -> (App, ParentingIds) {
    let mut app = app();
    let emit = spawn_emit(app.world_mut(), 0, None);
    let recv = spawn_recv(app.world_mut(), 1, None);
    connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
    let parent = spawn_spatial(app.world_mut(), 2, None);
    let child = spawn_spatial(app.world_mut(), 3, Some(parent));
    recompile(&mut app);
    (app, ParentingIds { child, parent })
}
