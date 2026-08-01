//! `StandardMaterialNode` — one node per material type, per §2.4. Design §8.
//!
//! Named `StandardMaterialNode` rather than `StandardMaterial` because the
//! material type itself is in scope in every file that uses it. §2.4's
//! eventual `MaterialNode<M>` generalisation keeps this shape.

use core::marker::PhantomData;

use bevy::prelude::*;
use sway_graph::{ContinuousIdx, NoSlots, NodeType, PortView, TickCtx};

/// The capability a material node produces: "a handle to a material of type
/// `M`". A `Mesh` node's `material` slot accepts exactly this.
#[derive(TypePath)]
pub struct MaterialOf<M: TypePath + Send + Sync + 'static>(PhantomData<fn() -> M>);

#[derive(Reflect, Component)]
pub struct StandardMaterialParams {
    pub base_color: Color,
    pub emissive: Color,
    pub metallic: f32,
    pub perceptual_roughness: f32,
}

impl Default for StandardMaterialParams {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            emissive: Color::BLACK,
            metallic: 0.0,
            perceptual_roughness: 0.5,
        }
    }
}

#[derive(Reflect, Default)]
pub struct StandardMaterialOutputs {}

/// Owns the handle. `Option` rather than `Handle::default()` so "not created
/// yet" is representable without relying on what a default handle points at.
#[derive(Component, Default)]
pub struct MaterialState {
    pub handle: Option<Handle<StandardMaterial>>,
}

pub struct StandardMaterialNode;

impl StandardMaterialNode {
    pub const BASE_COLOR: u16 = 0;
    pub const EMISSIVE: u16 = 1;
    pub const METALLIC: u16 = 2;
    pub const PERCEPTUAL_ROUGHNESS: u16 = 3;
}

impl NodeType for StandardMaterialNode {
    type Params = StandardMaterialParams;
    type Outputs = StandardMaterialOutputs;
    type Slots = NoSlots;
    type Produces = MaterialOf<StandardMaterial>;
    type State = MaterialState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("base_color", Self::BASE_COLOR),
        ("emissive", Self::EMISSIVE),
        ("metallic", Self::METALLIC),
        ("perceptual_roughness", Self::PERCEPTUAL_ROUGHNESS),
    ];

    fn register(app: &mut App) {
        app.register_type::<Color>();
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let base_color: Color = ports.read(ContinuousIdx(Self::BASE_COLOR as u32));
        let emissive: Color = ports.read(ContinuousIdx(Self::EMISSIVE as u32));
        let metallic: f32 = ports.read(ContinuousIdx(Self::METALLIC as u32));
        let perceptual_roughness: f32 =
            ports.read(ContinuousIdx(Self::PERCEPTUAL_ROUGHNESS as u32));

        let handle = world
            .get::<MaterialState>(node)
            .and_then(|s| s.handle.clone());
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();

        let handle = match handle {
            Some(handle) => handle,
            None => {
                let handle = materials.add(StandardMaterial {
                    base_color,
                    emissive: emissive.into(),
                    metallic,
                    perceptual_roughness,
                    ..default()
                });
                let handle_for_state = handle.clone();
                if let Some(mut state) = world.get_mut::<MaterialState>(node) {
                    state.handle = Some(handle_for_state);
                }
                return;
            }
        };

        // Read, compare, and only then `get_mut` — `get_mut` marks the asset
        // changed by the act of being called (parent §2.11).
        let Some(current) = materials.get(&handle) else {
            return;
        };
        let unchanged = current.base_color == base_color
            && current.emissive == emissive.into()
            && current.metallic == metallic
            && current.perceptual_roughness == perceptual_roughness;
        if unchanged {
            return;
        }
        if let Some(mut material) = materials.get_mut(&handle) {
            material.base_color = base_color;
            material.emissive = emissive.into();
            material.metallic = metallic;
            material.perceptual_roughness = perceptual_roughness;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::{PortArena, PortView, TickCtx};

    fn app_with_material() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<StandardMaterial>();
        let node = app
            .world_mut()
            .spawn((StandardMaterialParams::default(), MaterialState::default()))
            .id();
        (app, node)
    }

    /// Runs the node's tick with an arena holding the given base colour.
    ///
    /// `Assets::get_mut` only queues an `AssetEvent::Modified`; Bevy 0.19
    /// flushes that queue into `Messages<AssetEvent<A>>` from the
    /// `asset_events` system, which runs on `Update` (bevy_asset's own
    /// tests drive this the same way — via `app.update()` — before reading
    /// `Messages`). So this helper drives one update after the tick to
    /// match what a real frame would observe.
    fn tick_with(app: &mut App, node: Entity, colour: Color) {
        let mut arena = PortArena::new(4, 0);
        arena.continuous[StandardMaterialNode::BASE_COLOR as usize] = Box::new(colour);
        arena.continuous[StandardMaterialNode::EMISSIVE as usize] = Box::new(Color::BLACK);
        arena.continuous[StandardMaterialNode::METALLIC as usize] = Box::new(0.0_f32);
        arena.continuous[StandardMaterialNode::PERCEPTUAL_ROUGHNESS as usize] = Box::new(0.5_f32);
        let world = app.world_mut();
        let mut view = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 4]);
        StandardMaterialNode::tick(
            world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );
        app.update();
    }

    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<StandardMaterial>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn the_node_creates_and_drives_its_own_material() {
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));

        let handle = app
            .world()
            .get::<MaterialState>(node)
            .and_then(|s| s.handle.clone())
            .expect("the node owns a handle");
        let colour = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .unwrap()
            .base_color;
        assert_eq!(colour, Color::srgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn a_changed_colour_modifies_the_asset() {
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));
        let _ = count_modified(&mut app);

        tick_with(&mut app, node, Color::srgb(0.0, 1.0, 0.0));

        assert!(count_modified(&mut app) > 0, "a real change must write through");
    }

    #[test]
    fn an_unchanged_colour_does_not_touch_the_asset() {
        // Parent §2.11: `Assets::get_mut` marks the asset changed by the act
        // of being called, so an unconditional write re-uploads a material
        // that nothing moved.
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));
        let _ = count_modified(&mut app);

        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));

        assert_eq!(count_modified(&mut app), 0, "an unchanged colour must not rewrite");
    }
}
