//! Bevy runtime for Sway: headless rendering to a wgpu texture, graph
//! projection into ECS entities, and the render pipelines for point clouds,
//! sprite layers, and scatter compute.

pub mod frame_sequence;
pub mod headless;
pub mod nodes;
pub mod point_cloud;
pub mod project;
pub mod scatter;
pub mod shader_validation;
pub mod sprite_layer;
pub mod sprite_material;

pub use frame_sequence::{ColorSpace, SequenceError, assemble_layers, sort_frames_by_name};
pub use point_cloud::PointCloudPlugin;
pub use scatter::ScatterPlugin;
pub use sprite_layer::SpriteLayerPlugin;
pub use sprite_material::{
    SpriteMaterialAsset, SpriteMaterialPlugin, SpriteMaterialUniform, SpriteMeshBounds,
    inflate_local_aabb, layer_index, max_displacement, sync_sprite_material_bounds,
};

pub use nodes::register_runtime_node_kinds;
pub use project::{NodeEntities, ProducerSet, ProjectionSet, RuntimePlugin};
