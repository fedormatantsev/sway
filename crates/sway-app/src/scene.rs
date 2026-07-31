//! The M0 scene: one cube, one camera, one light. Replaced by graph-authored
//! scene nodes at M5 (spec §2.10).

use crate::bridge::CubeGraphOutput;
use bevy::prelude::*;
use sway_graph::{NodeRuntime, PortArena};

/// Marks the cube whose colour the graph drives.
#[derive(Component)]
pub struct Cube;

/// The colour for a given graph level. Pulled out so tests can assert against
/// it without duplicating the formula.
pub fn colour_for_level(level: f32) -> Color {
    Color::srgb(level, 0.1, 1.0 - level)
}

pub fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: colour_for_level(0.0),
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Cube,
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 2.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Writes the graph's envelope output into the cube's material.
///
/// Reads and compares before calling `get_mut`, because `get_mut` marks the
/// asset modified purely by being called — an unconditional write would
/// re-upload the material every frame (spec §2.11).
///
/// DEVIATION from spec §2.11: the spec puts "apply state to components"
/// inside the `FixedUpdate` tick; this runs in `Update` instead, so it fires
/// once per frame rather than once per graph tick. This is a deliberate
/// coalescing choice, not an oversight — the tick runs at 120 Hz (see
/// `main::TICK_HZ`), faster than the frame rate, so applying on every tick
/// would just mean redundant writes of intermediate states nothing ever
/// sees.
pub fn apply_level(
    output: Res<CubeGraphOutput>,
    arena: Res<PortArena>,
    runtimes: Query<&NodeRuntime>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<&MeshMaterial3d<StandardMaterial>, With<Cube>>,
) {
    let Ok(runtime) = runtimes.get(output.entity) else {
        return;
    };
    let Some(level) = arena.continuous[runtime.continuous_base + output.ordinal as usize]
        .try_downcast_ref::<f32>()
        .copied()
    else {
        return;
    };
    let want = colour_for_level(level);
    for handle in &q {
        let Some(current) = materials.get(&handle.0) else {
            continue;
        };
        if current.base_color == want {
            continue;
        }
        if let Some(mut mat) = materials.get_mut(&handle.0) {
            mat.base_color = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::CubeGraphOutput;
    use sway_graph::{NodeRuntime, PortArena};
    use sway_nodes::Envelope;

    /// Headless app with assets but no renderer, enough to exercise the
    /// material write path.
    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .add_systems(Update, apply_level);
        let envelope = app.world_mut().spawn(NodeRuntime::default()).id();
        let mut arena = PortArena::new(Envelope::OUT_VALUE as usize + 1, 0);
        arena.continuous[Envelope::OUT_VALUE as usize] = Box::new(0.0_f32);
        app.insert_resource(arena).insert_resource(CubeGraphOutput {
            entity: envelope,
            ordinal: Envelope::OUT_VALUE,
        });
        app
    }

    fn set_level(app: &mut App, level: f32) {
        app.world_mut().resource_mut::<PortArena>().continuous[Envelope::OUT_VALUE as usize] =
            Box::new(level);
    }

    fn spawn_cube(app: &mut App) -> Handle<StandardMaterial> {
        let handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::BLACK,
                ..default()
            });
        app.world_mut()
            .spawn((MeshMaterial3d(handle.clone()), Cube));
        handle
    }

    /// Drains asset-modified notifications since the last call. Note that in
    /// Bevy 0.19 `AssetEvent` is a `Message`, so the collection is `Messages`,
    /// not `Events`.
    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<StandardMaterial>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn level_drives_base_color() {
        let mut app = headless();
        let handle = spawn_cube(&mut app);

        set_level(&mut app, 1.0);
        app.update();

        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let colour = materials.get(&handle).unwrap().base_color;
        assert_eq!(colour, colour_for_level(1.0));
    }

    #[test]
    fn changed_level_modifies_the_asset() {
        let mut app = headless();
        let _handle = spawn_cube(&mut app);

        set_level(&mut app, 0.5);
        app.update();

        assert!(
            count_modified(&mut app) > 0,
            "a real colour change must write through"
        );
    }

    #[test]
    fn unchanged_level_does_not_touch_the_asset() {
        let mut app = headless();
        let _handle = spawn_cube(&mut app);

        set_level(&mut app, 0.5);
        app.update();
        let _ = count_modified(&mut app);

        // Same level again: apply_level must short-circuit before get_mut.
        app.update();
        assert_eq!(
            count_modified(&mut app),
            0,
            "apply_level must not rewrite an unchanged colour"
        );
    }
}
