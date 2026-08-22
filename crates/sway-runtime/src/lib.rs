//! Bevy runtime for Sway: headless rendering to a wgpu texture, the
//! render-coupled node kinds, and their projection into ECS entities.

pub mod headless;
pub mod nodes;
pub mod project;
pub mod shader_validation;

pub use nodes::frame_sequence::{ColorSpace, SequenceError, assemble_layers, sort_frames_by_name};
pub use nodes::register_runtime_node_kinds;
pub use nodes::sprite_material::{
    SpriteMaterialAsset, SpriteMaterialUniform, SpriteMeshBounds, inflate_local_aabb, layer_index,
    max_displacement, sync_sprite_material_bounds,
};
pub use project::{
    CameraTargets, CaptureIntent, CaptureIntents, EditorCameraPreview, NodeEntities,
    PresentedCamera, PresentedTarget, ProducerSet, ProjectionSet, RuntimePlugin, source_camera,
};
