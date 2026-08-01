//! Scene structure nodes: `Group` and `Rgb`. Design §8.

use bevy::prelude::*;
use sway_graph::{ContinuousIdx, NoSlots, NodeType, PortView, TickCtx};

/// Rotation is three scalar ports rather than one `Vec3`, because rotation is
/// the thing a signal actually drives and every M2a signal node outputs `f32`.
/// A `Vec3` port would need a vector-producing node that does not exist, and
/// §2.4's rule is that a node's ports are simply its fields. Translation and
/// scale stay `Vec3`: nothing drives them at M2b.
#[derive(Reflect, Component)]
pub struct GroupParams {
    pub translation: Vec3,
    /// Euler angles in radians, applied XYZ.
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

impl Default for GroupParams {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Reflect, Default)]
pub struct GroupOutputs {}

#[derive(Component, Default)]
pub struct GroupState;

pub struct Group;

impl Group {
    pub const TRANSLATION: u16 = 0;
    pub const ROTATION_X: u16 = 1;
    pub const ROTATION_Y: u16 = 2;
    pub const ROTATION_Z: u16 = 3;
    pub const SCALE: u16 = 4;
}

impl NodeType for Group {
    type Params = GroupParams;
    type Outputs = GroupOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = GroupState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("translation", Self::TRANSLATION),
        ("rotation_x", Self::ROTATION_X),
        ("rotation_y", Self::ROTATION_Y),
        ("rotation_z", Self::ROTATION_Z),
        ("scale", Self::SCALE),
    ];
    const SPATIAL: bool = true;

    fn register(app: &mut App) {
        app.register_type::<Vec3>();
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let translation: Vec3 = ports.read(ContinuousIdx(Self::TRANSLATION as u32));
        let rx: f32 = ports.read(ContinuousIdx(Self::ROTATION_X as u32));
        let ry: f32 = ports.read(ContinuousIdx(Self::ROTATION_Y as u32));
        let rz: f32 = ports.read(ContinuousIdx(Self::ROTATION_Z as u32));
        let scale: Vec3 = ports.read(ContinuousIdx(Self::SCALE as u32));
        let want = Transform {
            translation,
            rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
            scale,
        };
        // set_if_neq, per parent §2.11: an unconditional assignment re-runs
        // transform propagation for a scene that is not moving.
        match world.get_mut::<Transform>(node) {
            Some(mut transform) => {
                transform.set_if_neq(want);
            }
            None => {
                world.entity_mut(node).insert(want);
            }
        }
    }
}

#[derive(Reflect, Component, Default)]
pub struct RgbParams {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Reflect, Default)]
pub struct RgbOutputs {
    pub color: Color,
}

#[derive(Component, Default)]
pub struct RgbState;

/// Signal → `Color`. §2.4 fixes a material node's ports as the material's own
/// fields, so `base_color` is a `Color` port and something must produce one;
/// nothing in M2a's signal set does (design §8).
pub struct Rgb;

impl Rgb {
    pub const R: u16 = 0;
    pub const G: u16 = 1;
    pub const B: u16 = 2;
    pub const OUT_COLOR: u16 = 3;
}

impl NodeType for Rgb {
    type Params = RgbParams;
    type Outputs = RgbOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = RgbState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("r", Self::R),
        ("g", Self::G),
        ("b", Self::B),
        ("color", Self::OUT_COLOR),
    ];

    fn register(app: &mut App) {
        app.register_type::<Color>();
    }

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let r: f32 = ports.read(ContinuousIdx(Self::R as u32));
        let g: f32 = ports.read(ContinuousIdx(Self::G as u32));
        let b: f32 = ports.read(ContinuousIdx(Self::B as u32));
        ports.write(ContinuousIdx(Self::OUT_COLOR as u32), Color::srgb(r, g, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::{PortArena, PortView, TickCtx};

    /// Fills a five-slot arena with a Group's ports.
    fn group_arena(translation: Vec3) -> PortArena {
        let mut arena = PortArena::new(5, 0);
        arena.continuous[Group::TRANSLATION as usize] = Box::new(translation);
        arena.continuous[Group::ROTATION_X as usize] = Box::new(0.0_f32);
        arena.continuous[Group::ROTATION_Y as usize] = Box::new(0.0_f32);
        arena.continuous[Group::ROTATION_Z as usize] = Box::new(0.0_f32);
        arena.continuous[Group::SCALE as usize] = Box::new(Vec3::ONE);
        arena
    }

    #[test]
    fn a_group_writes_its_transform() {
        let mut world = World::new();
        let node = world.spawn((GroupParams::default(), GroupState)).id();
        let mut arena = group_arena(Vec3::new(1.0, 2.0, 3.0));
        let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);

        Group::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            world.get::<Transform>(node).map(|t| t.translation),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn an_unchanged_transform_is_not_rewritten() {
        // set_if_neq, per parent §2.11: an unconditional assignment sets the
        // change tick every tick, re-running propagation and making
        // `Changed<Transform>` worthless downstream.
        let mut world = World::new();
        let node = world.spawn((GroupParams::default(), GroupState)).id();
        let mut arena = group_arena(Vec3::ZERO);

        for _ in 0..2 {
            let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);
            Group::tick(
                &mut world,
                node,
                &mut view,
                &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
            );
            // A raw World, unlike an App running a schedule, never advances
            // its own change tick on its own; without this, every write in
            // this test — spurious or not — would be stamped with the same
            // Tick value and last_changed() comparisons below would hold
            // trivially, discriminating nothing.
            world.increment_change_tick();
        }
        let first = world.entity(node).get_ref::<Transform>().unwrap().last_changed();

        let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);
        Group::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            world.entity(node).get_ref::<Transform>().unwrap().last_changed(),
            first,
            "an unchanged Transform must not be re-marked"
        );
    }

    #[test]
    fn rgb_writes_a_color_to_its_output_port() {
        // The first struct-typed value across a continuous edge (design §8).
        let mut world = World::new();
        let node = world.spawn((RgbParams::default(), RgbState)).id();
        let mut arena = PortArena::new(4, 0);
        arena.continuous[Rgb::R as usize] = Box::new(1.0_f32);
        arena.continuous[Rgb::G as usize] = Box::new(0.5_f32);
        arena.continuous[Rgb::B as usize] = Box::new(0.0_f32);
        arena.continuous[Rgb::OUT_COLOR as usize] = Box::new(Color::BLACK);
        let mut view = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 3]);

        Rgb::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            arena.continuous[Rgb::OUT_COLOR as usize].try_downcast_ref::<Color>(),
            Some(&Color::srgb(1.0, 0.5, 0.0))
        );
    }
}
