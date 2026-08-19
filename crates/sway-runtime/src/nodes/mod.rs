//! The render-coupled node kinds (`redesign-graph-model` tasks 5.2, 5.4-5.6),
//! landing beside the wire-model equivalents in `sway-nodes` and at this
//! crate's root. Nothing here replaces anything else — group 9 deletes the
//! wire model, not this module tree.
//!
//! Every kind is one `#[derive(Reflect)]` struct with exactly the fields
//! `inlets`, `state`, `outlets` (design D3), registered with
//! [`sway_graph::graph::RegisterNodeKind`] by [`RuntimeNodesPlugin`].
//!
//! ## Why these live in `sway-runtime` and not `sway-nodes`
//!
//! They all name `bevy_render` types — `Handle<Mesh>`, `Handle<Image>`,
//! `StandardMaterial`, `MeshMaterial3d<M>` — and `SpriteMaterial` needs its
//! own shader and pipeline, which this crate owns. Putting every
//! render-coupled node kind here leaves `sway-nodes` as the pure value-node
//! crate, which is a cleaner boundary than splitting them across two.

use bevy::prelude::*;
use sway_graph::graph::RegisterNodeKind;

pub mod frame_sequence;
pub mod mesh;
pub mod pbr_material;
pub mod protocol;
pub mod scene;
pub mod sprite_material;

pub use frame_sequence::{FrameSequence, FrameSequenceIn, FrameSequenceState};
pub use mesh::{MeshAsset, MeshAssetIn, MeshAssetState, PlaneMesh, PlaneMeshIn, PlaneMeshState};
pub use pbr_material::{PbrMaterial, PbrMaterialIn, PbrMaterialState};
pub use scene::{
    Camera, CameraIn, DirectionalLight, DirectionalLightIn, Group, GroupIn, MeshNode, MeshNodeIn,
    PointLight, PointLightIn,
};
pub use sprite_material::{SpriteMaterial, SpriteMaterialIn, SpriteMaterialState};

// `protocol::MeshNode` (the mesh-source trait) is deliberately NOT re-exported
// here: `scene::MeshNode` (the scene node kind) already owns that name, and
// design D6 and D7 name both. Reach the trait as `protocol::MeshNode`.

/// Registers every render-coupled node kind, its part types, and the protocol
/// markers a document or the inspector may address by path.
///
/// Separate from [`RuntimeNodesPlugin`] so a test — or anything that only
/// needs the *schema* — can have the kinds without the render pipeline the
/// sprite material brings with it.
pub fn register_runtime_node_kinds(app: &mut App) {
    app
        // --- protocol markers and their shared outlet parts ------------
        .register_type::<protocol::SceneMaterial>()
        .register_type::<protocol::ImageSequence>()
        .register_type::<protocol::MeshSource>()
        .register_type::<protocol::SceneChild>()
        .register_type::<protocol::MeshSourceOut>()
        .register_type::<protocol::SceneMaterialOut>()
        .register_type::<protocol::ImageSequenceOut>()
        .register_type::<protocol::SceneNodeOut>()
        // --- producers -------------------------------------------------
        .register_node_kind::<MeshAsset>()
        .register_type::<MeshAssetIn>()
        .register_type::<MeshAssetState>()
        .register_node_kind::<PlaneMesh>()
        .register_type::<PlaneMeshIn>()
        .register_type::<PlaneMeshState>()
        .register_node_kind::<FrameSequence>()
        .register_type::<FrameSequenceIn>()
        .register_type::<FrameSequenceState>()
        .register_type::<crate::frame_sequence::ColorSpace>()
        // --- materials -------------------------------------------------
        .register_node_kind::<PbrMaterial>()
        .register_type::<PbrMaterialIn>()
        .register_type::<PbrMaterialState>()
        .register_node_kind::<SpriteMaterial>()
        .register_type::<SpriteMaterialIn>()
        .register_type::<SpriteMaterialState>()
        // --- scene nodes -----------------------------------------------
        .register_node_kind::<MeshNode>()
        .register_type::<MeshNodeIn>()
        .register_node_kind::<Group>()
        .register_type::<GroupIn>()
        .register_node_kind::<Camera>()
        .register_type::<CameraIn>()
        .register_node_kind::<DirectionalLight>()
        .register_type::<DirectionalLightIn>()
        .register_node_kind::<PointLight>()
        .register_type::<PointLightIn>()
        // Addressable by path from a document or the inspector.
        .register_type::<Transform>();
}

/// Registers every render-coupled node kind *and* the sprite material's
/// shader and render pipeline.
///
/// The pipeline half is skipped when
/// [`SpriteMaterialPlugin`](crate::sprite_material::SpriteMaterialPlugin)
/// already added it: the two node models land beside each other until group
/// 9, and adding one Bevy plugin twice panics.
pub struct RuntimeNodesPlugin;

impl Plugin for RuntimeNodesPlugin {
    fn build(&self, app: &mut App) {
        crate::sprite_material::ensure_sprite_material_pipeline(app);
        register_runtime_node_kinds(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::reflect::AppTypeRegistry;
    use sway_graph::graph::registry::registered_node_kinds;

    #[test]
    fn the_plugin_registers_every_new_node_kind_with_a_unique_short_name() {
        // The document keys nodes by the last `::` segment of the type path,
        // so a collision here is a document that cannot round-trip.
        let mut app = App::new();
        app.add_plugins((
            bevy::asset::AssetPlugin::default(),
            bevy::app::TaskPoolPlugin::default(),
        ));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<crate::sprite_material::SpriteMaterialAsset>();
        register_runtime_node_kinds(&mut app);

        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        let kinds = registered_node_kinds(&registry);

        let mut short_names: Vec<&str> = kinds
            .iter()
            .map(|path| path.rsplit("::").next().unwrap_or(path))
            .collect();
        let before_dedup = short_names.len();
        short_names.sort_unstable();
        short_names.dedup();
        assert_eq!(
            short_names.len(),
            before_dedup,
            "every node kind's short name must be unique: {kinds:?}"
        );

        for expected in [
            "MeshAsset",
            "PlaneMesh",
            "FrameSequence",
            "PbrMaterial",
            "SpriteMaterial",
            "MeshNode",
            "Group",
            "Camera",
            "DirectionalLight",
            "PointLight",
        ] {
            assert!(
                short_names.contains(&expected),
                "missing `{expected}` in {short_names:?}"
            );
        }
    }
}
