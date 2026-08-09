//! The one thing the document cannot say yet.
//!
//! A `Handle<Mesh>` is asset flow and asset flow is M5 (project spec §8), so
//! the document authors a marker and this attaches the renderable parts. When
//! M5 lands, this file goes away.

use bevy::prelude::*;
use sway_graph::register_authorable;

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct DemoCube;

#[derive(Resource)]
struct CubeAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn create_cube_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(CubeAssets {
        mesh: meshes.add(Cuboid::new(0.6, 0.6, 0.6)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.7, 0.9),
            ..default()
        }),
    });
}

/// An ordinary `Added<T>` system — the second row of the parent spec's
/// behaviour table (§2.2): it consumes and produces nothing the graph reads,
/// so it has no business being in the order.
fn attach_cube_visuals(
    mut commands: Commands,
    assets: Res<CubeAssets>,
    added: Query<Entity, Added<DemoCube>>,
) {
    for entity in &added {
        commands.entity(entity).insert((
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Visibility::default(),
        ));
    }
}

pub struct DemoAssetsPlugin;

impl Plugin for DemoAssetsPlugin {
    fn build(&self, app: &mut App) {
        register_authorable::<DemoCube>(app, "DemoCube");
        app.add_systems(Startup, create_cube_assets)
            .add_systems(Update, attach_cube_visuals.run_if(resource_exists::<CubeAssets>));
    }
}
