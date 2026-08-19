//! Z-depth billboarded sprite layers, via Bevy's `Material` trait.
//!
//! There is no `Sprite3d` in Bevy 0.19, so a "z-depth sprite layer" is a
//! textured, alpha-blended quad that always faces the camera (a billboard).
//! Billboarding is done entirely in the vertex shader — see
//! `assets/shaders/sprite_layer.wgsl` — by displacing a unit quad's local
//! corners along the camera's own right/up axes (read from the view uniform)
//! before projecting, rather than rotating a `Transform` on the CPU side
//! every frame. This is the standard approach and needs no extra system.
//!
//! This is deliberately the easier of M1's two custom-shader tasks: the
//! `Material` trait (used here) hands queuing, sorting and draw-command
//! generation to Bevy, in exchange for only being able to customize the
//! vertex/fragment shader and the material's own bind group. Task 3's point
//! cloud needed the low-level `SpecializedMeshPipeline` + custom
//! `RenderCommand` path instead because it required a custom *instance*
//! buffer; nothing here does.
//!
//! Three deviations from the brief's verbatim WGSL, all forced by the
//! `Material` trait's fixed bind-group conventions (verified against
//! `bevy_pbr::material` / `bevy_pbr::render::mesh` source — see the comment
//! block at the top of `sprite_layer.wgsl` for the full reasoning):
//!
//! 1. The shader imports Bevy's own `View` type (via
//!    `#import bevy_pbr::mesh_view_bindings::view`) instead of declaring a
//!    small self-contained `View` struct at @group(0). A custom struct there
//!    would silently misread Bevy's real (much larger) view uniform after
//!    its first field. This takes the shader out of naga's static validator,
//!    so `sprite_layer.wgsl` is added to `PREPROCESSOR_SHADERS` in
//!    `shader_validation.rs` alongside `point_cloud.wgsl`.
//! 2. The material's own uniform/texture/sampler bindings live at
//!    @group(3), not @group(1): `bevy_pbr::material::MATERIAL_BIND_GROUP_INDEX`
//!    is a fixed `3` in this Bevy version (@group(1) is Bevy's mesh-view
//!    binding-array group, @group(2) is the mesh group).
//! 3. The vertex input is Bevy's standard position attribute
//!    (`@location(0) position: vec3<f32>`) instead of a bespoke `corner:
//!    vec2<f32>` attribute, so a plain `Rectangle` mesh — positions already
//!    in [-0.5, 0.5], z = 0 — works with the pipeline's default vertex
//!    buffer layout and no custom `Material::specialize` override is needed.

use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

/// Matches `Layer` in `sprite_layer.wgsl` field-for-field: uniform-buffer
/// layout rules (16-byte-aligned vec4s) make a 1:1 Rust/WGSL struct the
/// simplest way to keep the two in sync.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct LayerUniform {
    /// xy = world centre, z = depth, w = uniform scale.
    pub placement: Vec4,
    pub tint: Vec4,
    /// xy = atlas cell size in UV, zw = atlas cell offset. The M1 demo has
    /// no atlas (one full-frame texture per material), so this is always
    /// `(1, 1, 0, 0)`.
    pub atlas: Vec4,
}

/// A textured, alpha-blended billboard quad pinned at a world-space depth.
/// See the module docs for why this uses the `Material` trait rather than
/// Task 3's custom pipeline, and for the three shader deviations this forces.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SpriteLayerMaterial {
    #[uniform(0)]
    pub layer: LayerUniform,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for SpriteLayerMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("../assets/shaders/sprite_layer.wgsl"))
                .with_source("embedded"),
        )
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("../assets/shaders/sprite_layer.wgsl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

/// Registers the `Material` render path for `SpriteLayerMaterial`. Nothing
/// else — no demo content — per the pattern set by Task 3: demo spawning is
/// a separate `pub fn`, not part of `build()`, so `sway-app` can opt in
/// behind its `--demo` flag (Task 6).
pub struct SpriteLayerPlugin;

impl Plugin for SpriteLayerPlugin {
    fn build(&self, app: &mut App) {
        // Compiles `sprite_layer.wgsl` into the binary and registers it
        // under the `embedded://` asset source — see the matching
        // `embedded_path!` calls in `SpriteLayerMaterial`'s
        // `vertex_shader`/`fragment_shader` above.
        embedded_asset!(app, "../assets/shaders/sprite_layer.wgsl");

        app.add_plugins(MaterialPlugin::<SpriteLayerMaterial>::default());
    }
}
