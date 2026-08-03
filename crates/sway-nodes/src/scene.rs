//! Scene structure nodes: `Group` and `Rgb`. Design §8.

use bevy::prelude::*;
use sway_graph::{NodeType, PortView, Product, Spatial, TickCtx, register_product};

/// Rotation is three scalar ports rather than one `Vec3`, because rotation is
/// the thing a signal actually drives and every M2a signal node outputs `f32`.
/// A `Vec3` port would need a vector-producing node that does not exist, and
/// §2.4's rule is that a node's ports are simply its fields. Translation and
/// scale stay `Vec3`: nothing drives them at M2b.
#[derive(Reflect, Component)]
pub struct GroupInlets {
    pub children: Vec<Product<Spatial>>,
    pub translation: Vec3,
    /// Euler angles in radians, applied XYZ.
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

impl Default for GroupInlets {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            translation: Vec3::ZERO,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale: Vec3::ONE,
        }
    }
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
    pub const TRANSLATION: u16 = 1;
    pub const ROTATION_X: u16 = 2;
    pub const ROTATION_Y: u16 = 3;
    pub const ROTATION_Z: u16 = 4;
    pub const SCALE: u16 = 5;
    pub const OUT_SPATIAL: u16 = 6;
}

impl NodeType for Group {
    type Inlets = GroupInlets;
    type Outlets = GroupOutlets;
    type State = GroupState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("children", Self::CHILDREN),
        ("translation", Self::TRANSLATION),
        ("rotation_x", Self::ROTATION_X),
        ("rotation_y", Self::ROTATION_Y),
        ("rotation_z", Self::ROTATION_Z),
        ("scale", Self::SCALE),
        ("spatial", Self::OUT_SPATIAL),
    ];

    fn register(app: &mut App) {
        app.register_type::<Vec3>();
        register_product::<Spatial>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let translation: Vec3 = ports.read(Self::TRANSLATION);
        let rx: f32 = ports.read(Self::ROTATION_X);
        let ry: f32 = ports.read(Self::ROTATION_Y);
        let rz: f32 = ports.read(Self::ROTATION_Z);
        let scale: Vec3 = ports.read(Self::SCALE);
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
pub struct RgbInlets {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Reflect, Default)]
pub struct RgbOutlets {
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
    type Inlets = RgbInlets;
    type Outlets = RgbOutlets;
    type State = RgbState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("r", Self::R),
        ("g", Self::G),
        ("b", Self::B),
        ("color", Self::OUT_COLOR),
    ];

    fn register(app: &mut App) {
        app.register_type::<Color>();
    }

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let r: f32 = ports.read(Self::R);
        let g: f32 = ports.read(Self::G);
        let b: f32 = ports.read(Self::B);
        ports.write(Self::OUT_COLOR, Color::srgb(r, g, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::{register_node_type, FieldSpec, NodeTypeRegistry, PortArena, PortView, TickCtx};

    fn node_fields<N: NodeType>() -> Vec<FieldSpec> {
        let mut app = App::new();
        let id = register_node_type::<N>(&mut app);
        let entry = app.world().resource::<NodeTypeRegistry>().get(id).expect("registered");
        let mut fields = entry.inlets.clone();
        fields.extend(entry.outlets.iter().cloned());
        fields
    }

    /// Fills a Group's fields, with `children` left at its (harmless, since
    /// `Group::tick` never reads it) fictitious single slot.
    fn group_arena() -> (PortArena, Vec<FieldSpec>, Vec<usize>, Vec<usize>, Vec<bool>) {
        let fields = node_fields::<Group>();
        let offsets: Vec<usize> = (0..fields.len()).collect();
        let lens = vec![1usize; fields.len()];
        let connected = vec![false; fields.len()];
        let arena = PortArena::new(fields.len());
        (arena, fields, offsets, lens, connected)
    }

    #[test]
    fn a_group_writes_its_transform() {
        let mut world = World::new();
        let node = world.spawn((GroupInlets::default(), GroupState)).id();
        let (mut arena, fields, offsets, lens, connected) = group_arena();
        arena.values[Group::TRANSLATION as usize] = Box::new(Vec3::new(1.0, 2.0, 3.0));
        arena.values[Group::ROTATION_X as usize] = Box::new(0.0_f32);
        arena.values[Group::ROTATION_Y as usize] = Box::new(0.0_f32);
        arena.values[Group::ROTATION_Z as usize] = Box::new(0.0_f32);
        arena.values[Group::SCALE as usize] = Box::new(Vec3::ONE);
        let mut view = PortView::new(&mut arena, 0, &fields, &offsets, &lens, &connected);

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
        let node = world.spawn((GroupInlets::default(), GroupState)).id();
        let (mut arena, fields, offsets, lens, connected) = group_arena();
        arena.values[Group::TRANSLATION as usize] = Box::new(Vec3::ZERO);
        arena.values[Group::ROTATION_X as usize] = Box::new(0.0_f32);
        arena.values[Group::ROTATION_Y as usize] = Box::new(0.0_f32);
        arena.values[Group::ROTATION_Z as usize] = Box::new(0.0_f32);
        arena.values[Group::SCALE as usize] = Box::new(Vec3::ONE);

        for _ in 0..2 {
            let mut view = PortView::new(&mut arena, 0, &fields, &offsets, &lens, &connected);
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

        let mut view = PortView::new(&mut arena, 0, &fields, &offsets, &lens, &connected);
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
        let node = world.spawn((RgbInlets::default(), RgbState)).id();
        let fields = node_fields::<Rgb>();
        let offsets: Vec<usize> = (0..fields.len()).collect();
        let lens = vec![1usize; fields.len()];
        let connected = vec![false; fields.len()];
        let mut arena = PortArena::new(fields.len());
        arena.values[Rgb::R as usize] = Box::new(1.0_f32);
        arena.values[Rgb::G as usize] = Box::new(0.5_f32);
        arena.values[Rgb::B as usize] = Box::new(0.0_f32);
        arena.values[Rgb::OUT_COLOR as usize] = Box::new(Color::BLACK);
        let mut view = PortView::new(&mut arena, 0, &fields, &offsets, &lens, &connected);

        Rgb::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            arena.values[Rgb::OUT_COLOR as usize].try_downcast_ref::<Color>(),
            Some(&Color::srgb(1.0, 0.5, 0.0))
        );
    }
}
