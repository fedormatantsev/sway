//! The live graph: `GraphAsset` loads once into the `Graph` resource.
//!
//! Design D1: the asset is a loading mechanism only. It is not kept in sync
//! with the resource and is not consulted after initialization. `AssetEvent::
//! Modified` is therefore ignored — reloading a project is an explicit action
//! (architecture: Reloading a project is an explicit action).

use std::path::{Path, PathBuf};

use bevy_app::{App, Plugin, PreUpdate, Startup};
use bevy_asset::{AssetServer, Assets, Handle};
use bevy_ecs::change_detection::{Mut, ResMut};
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::resource::Resource;
use bevy_ecs::system::{Commands, Res};
use bevy_ecs::world::World;
use sway_graph::graph::Graph;

use crate::v4::asset::{GraphAsset, GraphAssetPlugin};
use crate::v4::ids::StableIds;
use crate::v4::{load, save_to_path};

/// Relative path of the graph file inside the project directory.
#[derive(Resource, Clone, Debug)]
pub struct GraphFile {
    pub relative: String,
}

/// The loaded `GraphAsset` handle. Identity for save, not a live copy.
#[derive(Resource, Clone, Debug)]
pub struct GraphHandle(pub Handle<GraphAsset>);

/// Session-stable ids, seeded at load and extended on save.
#[derive(Resource, Debug, Default)]
pub struct SessionIds(pub StableIds);

/// Whether [`init_graph_from_asset`] has built the live `Graph`.
#[derive(Resource, Debug, Default)]
pub struct GraphInitialized(pub bool);

/// Absolute project directory — the asset root.
#[derive(Resource, Clone, Debug)]
pub struct ProjectDirectory(pub PathBuf);

/// Loads the named graph file through the asset pipeline and, once, copies it
/// into the live [`Graph`] resource.
pub struct LiveGraphPlugin {
    pub graph_file: String,
}

impl Plugin for LiveGraphPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(GraphAssetPlugin)
            .insert_resource(GraphFile {
                relative: self.graph_file.clone(),
            })
            .init_resource::<SessionIds>()
            .init_resource::<GraphInitialized>()
            .add_systems(Startup, start_graph_load)
            .add_systems(PreUpdate, init_graph_from_asset);
    }
}

fn start_graph_load(server: Res<AssetServer>, file: Res<GraphFile>, mut commands: Commands) {
    let relative = file.relative.clone();
    commands.insert_resource(GraphHandle(server.load(relative)));
}

fn init_graph_from_asset(
    assets: Res<Assets<GraphAsset>>,
    handle: Option<Res<GraphHandle>>,
    registry: Res<AppTypeRegistry>,
    mut graph: ResMut<Graph>,
    mut ids: ResMut<SessionIds>,
    mut initialized: ResMut<GraphInitialized>,
) {
    if initialized.0 {
        return;
    }
    let Some(handle) = handle else {
        return;
    };
    let Some(asset) = assets.get(&handle.0) else {
        return;
    };
    let registry = registry.read();
    let (loaded, stable, diagnostics) = load(&asset.doc, &registry);
    if !diagnostics.is_clean() {
        eprintln!("graph load diagnostics: {:?}", diagnostics.items);
    }
    *graph = loaded;
    ids.0 = stable;
    initialized.0 = true;
}

/// Writes the live graph back to the file it was opened from.
pub fn save_open_graph(world: &mut World) -> Result<(), String> {
    let relative = world
        .get_resource::<GraphFile>()
        .ok_or("no graph file is open")?
        .relative
        .clone();
    let directory = world
        .get_resource::<ProjectDirectory>()
        .ok_or("no project directory")?
        .0
        .clone();
    let path = directory.join(relative);
    save_graph_to(world, &path)
}

fn save_graph_to(world: &mut World, path: &Path) -> Result<(), String> {
    let type_registry = world
        .get_resource::<AppTypeRegistry>()
        .ok_or("no type registry")?
        .clone();
    world.resource_scope(|world, mut ids: Mut<SessionIds>| {
        let graph = world.get_resource::<Graph>().ok_or("no graph")?;
        let registry = type_registry.read();
        save_to_path(graph, &registry, &mut ids.0, path)
    })
}
