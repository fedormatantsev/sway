//! Provisional render spike code for M1. Point cloud, z-depth sprite layer,
//! and a compute-cooked scatter operator, all with hardcoded parameters.
//!
//! Per spec §5 the goal here is knowledge, not architecture — expect most of
//! this to be rewritten at M5.

pub mod frame_sequence;
pub mod headless;
pub mod nodes;
pub mod point_cloud;
pub mod project;
pub mod scatter;
pub mod shader_validation;
pub mod sprite_depth_spike;
pub mod sprite_layer;
pub mod sprite_material;
pub mod viewport;

pub use frame_sequence::{
    ColorSpace, FrameSequence, FrameSequenceOut, FrameSequenceState, SequenceError,
    assemble_layers, sort_frames_by_name, sync_frame_sequences,
};
pub use point_cloud::PointCloudPlugin;
pub use scatter::ScatterPlugin;
pub use sprite_depth_spike::SpriteDepthPlugin;
pub use sprite_layer::SpriteLayerPlugin;
pub use sprite_material::{
    ColorRunFrom, DepthRunFrom, FrameFrom, OpacityFrom, SpriteMaterial, SpriteMaterialAsset,
    SpriteMaterialFrom, SpriteMaterialOut, SpriteMaterialPlugin, SpriteMaterialState,
    SpriteMaterialUniform, SpriteMeshBounds, TintFrom, inflate_local_aabb, layer_index,
    max_displacement, sync_sprite_material_bounds, sync_sprite_materials,
};
pub use viewport::EditorViewportPlugin;

// The new graph model's surface (`redesign-graph-model` group 5). It lands
// beside everything above; group 9 deletes the wire model, not this.
pub use nodes::RuntimeNodesPlugin;
pub use project::{NodeEntities, ProjectionPlugin, ProjectionSet};
