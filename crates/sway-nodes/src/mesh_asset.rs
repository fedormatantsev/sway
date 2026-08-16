//! `MeshAsset` — a mesh that comes from a file.

use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::EditorPos;

/// A mesh named by path. The sub-asset label is part of the path —
/// `"cube.gltf#Mesh0/Primitive0"` — because a glTF file holds many meshes.
///
/// `Mesh3d` and `MeshMaterial3d` are required rather than inserted by the
/// system below so that a `MaterialFrom` wire always has a target to write
/// into, even before anything has loaded.
#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Transform, Visibility, Mesh3d, MeshMaterial3d<StandardMaterial>, EditorPos)]
pub struct MeshAsset {
    pub path: String,
}

/// An ordinary `Changed<T>` system — the second row of the behaviour table
/// (architecture §2): it consumes nothing the graph produces within a tick.
pub fn load_mesh_assets(
    asset_server: Res<AssetServer>,
    mut meshes: Query<(&MeshAsset, &mut Mesh3d), Changed<MeshAsset>>,
) {
    for (asset, mut mesh) in &mut meshes {
        if asset.path.is_empty() {
            continue;
        }
        mesh.set_if_neq(Mesh3d(asset_server.load(asset.path.clone())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    /// `AssetPlugin` plus the one asset type, which is all the load system
    /// needs — no device, no renderer. The path never resolves to a real file
    /// here; `AssetServer::load` hands back its handle immediately either way,
    /// and that handle is what this system's contract is about.
    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.add_systems(Update, load_mesh_assets);
        app
    }

    #[test]
    fn a_path_becomes_a_mesh_handle() {
        let mut app = asset_app();
        let entity = app
            .world_mut()
            .spawn(MeshAsset {
                path: "cube.gltf#Mesh0/Primitive0".into(),
            })
            .id();

        app.update();

        let handle = app
            .world()
            .get::<Mesh3d>(entity)
            .expect("#[require] supplies Mesh3d");
        assert_ne!(
            handle.0,
            Handle::default(),
            "the load system replaced the default handle"
        );
    }

    #[test]
    fn an_empty_path_leaves_the_handle_alone() {
        // What a palette click produces before anyone types a path. It must not
        // ask the asset server to load "", which logs an error every frame.
        let mut app = asset_app();
        let entity = app.world_mut().spawn(MeshAsset::default()).id();

        app.update();

        assert_eq!(
            app.world().get::<Mesh3d>(entity).map(|m| m.0.clone()),
            Some(Handle::default())
        );
    }

    #[test]
    fn require_supplies_everything_the_renderer_needs() {
        // Mesh3d requires Transform but NOT Visibility, which is why Visibility
        // is on MeshAsset's own require list. Without it nothing draws.
        let mut app = asset_app();
        let entity = app.world_mut().spawn(MeshAsset::default()).id();

        assert!(app.world().get::<Transform>(entity).is_some());
        assert!(app.world().get::<Visibility>(entity).is_some());
        assert!(app.world().get::<Mesh3d>(entity).is_some());
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_some(),
            "the material wire needs a target component to write into"
        );
    }
}
