//! Provisional render spike code for M1. Point cloud, z-depth sprite layer,
//! and a compute-cooked scatter operator, all with hardcoded parameters.
//!
//! Per spec §5 the goal here is knowledge, not architecture — expect most of
//! this to be rewritten at M5.

pub mod headless;
pub mod point_cloud;
pub mod scatter;
pub mod shader_validation;
pub mod sprite_depth_spike;
pub mod sprite_layer;

pub use point_cloud::PointCloudPlugin;
pub use scatter::ScatterPlugin;
pub use sprite_depth_spike::SpriteDepthPlugin;
pub use sprite_layer::SpriteLayerPlugin;
