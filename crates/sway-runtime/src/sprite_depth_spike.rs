//! M8 spike: does an alpha-blended sprite quad that writes per-pixel
//! `frag_depth` interpenetrate an opaque mesh?
//!
//! Throwaway by design. `sprite_layer.rs` is left untouched as an M1
//! regression signal; M8 rewrites it properly using whatever this proves.
//!
//! Three pieces make it work:
//!
//! 1. `Material::specialize` flips `depth_stencil.depth_write_enabled` to
//!    `Some(true)`. Bevy's mesh pipeline sets it `false` for every blended
//!    pass (`bevy_pbr::render::mesh`, the `BLEND_ALPHA` branch, whose
//!    comment reads "their depth is not written to the depth buffer").
//!    Verified safe: the transparent pass binds its depth attachment with
//!    `StoreOp::Store`, writable, not read-only
//!    (`main_transparent_pass_3d_node.rs`).
//! 2. The fragment shader returns `@builtin(frag_depth)`.
//! 3. That depth comes from re-projecting a displaced world position, not
//!    from arithmetic on a depth value — see the shader's header for why
//!    reverse-Z makes the direct route a trap.

use bevy::{
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::visibility::NoFrustumCulling,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};

/// The sheet value that means "exactly at the quad's plane".
pub const SPIKE_DEPTH_PIVOT: f32 = 0.5;
/// World units spanned by the full 0..1 depth channel.
pub const SPIKE_DEPTH_RANGE: f32 = 4.0;

/// Matches `Layer` in `sprite_depth_spike.wgsl` field for field.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct SpriteDepthUniform {
    /// xy = world centre, z = depth, w = uniform scale.
    pub placement: Vec4,
    pub tint: Vec4,
    /// xy = atlas cell size in UV, zw = atlas cell offset.
    pub atlas: Vec4,
    /// x = pivot, y = world-unit range, zw = unused.
    pub depth_params: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SpriteDepthMaterial {
    #[uniform(0)]
    pub layer: SpriteDepthUniform,
    #[texture(1)]
    #[sampler(2)]
    pub color_texture: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub depth_texture: Handle<Image>,
}

/// Turns depth writes back on.
///
/// Split out of `specialize` purely so it can be unit-tested: the real
/// `specialize` takes a `&MaterialPipeline` and a `MaterialPipelineKey`,
/// neither of which is constructible outside a render world, while
/// `RenderPipelineDescriptor` derives `Default`.
pub fn enable_depth_write(descriptor: &mut RenderPipelineDescriptor) {
    if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
        depth_stencil.depth_write_enabled = Some(true);
    }
}

impl Material for SpriteDepthMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("../assets/shaders/sprite_depth_spike.wgsl"))
                .with_source("embedded"),
        )
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("../assets/shaders/sprite_depth_spike.wgsl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        enable_depth_write(descriptor);
        Ok(())
    }
}

pub struct SpriteDepthPlugin;

impl Plugin for SpriteDepthPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "../assets/shaders/sprite_depth_spike.wgsl");
        app.add_plugins(MaterialPlugin::<SpriteDepthMaterial>::default());
    }
}

/// A single-channel-in-R depth atlas: left half near (0.0), right half far
/// (1.0). Raw bytes consumed by `depth_step_image`, which deliberately does
/// *not* interpret them as sRGB — see that function's doc comment for why.
pub fn depth_step_rgba(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size as usize) * (size as usize) * 4);
    for _y in 0..size {
        for x in 0..size {
            let value = if x < size / 2 { 0u8 } else { 255u8 };
            data.extend_from_slice(&[value, value, value, 255]);
        }
    }
    data
}

/// `Rgba8Unorm`, deliberately **not** `Rgba8UnormSrgb`: a depth channel is
/// data, not colour, and an sRGB view would apply a transfer curve to it.
/// The step atlas only holds 0 and 1 (both sRGB fixed points) so it would
/// survive either way, but M8's real depth sheets will hold midtones and
/// would not.
pub fn depth_step_image(size: u32) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        depth_step_rgba(size),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A flat opaque white colour atlas. The tint carries the colour, so the
/// texture only has to not interfere.
pub fn solid_white_image(size: u32) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255u8; (size as usize) * (size as usize) * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// The spike scene, for looking at. Same geometry as the integration test,
/// but off-axis: viewed straight on, correct per-pixel depth and a flat
/// alpha mask look identical. The angle is what makes the difference
/// visible.
pub fn spawn_depth_spike_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sprites: ResMut<Assets<SpriteDepthMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(0.0, 1.0, 0.0),
            unlit: true,
            ..default()
        })),
        Transform::default(),
    ));

    let material = sprites.add(SpriteDepthMaterial {
        layer: SpriteDepthUniform {
            placement: Vec3::ZERO.extend(3.0),
            tint: Vec4::new(1.0, 0.0, 0.0, 0.85),
            atlas: Vec4::new(1.0, 1.0, 0.0, 0.0),
            depth_params: Vec4::new(SPIKE_DEPTH_PIVOT, SPIKE_DEPTH_RANGE, 0.0, 0.0),
        },
        color_texture: images.add(solid_white_image(64)),
        depth_texture: images.add(depth_step_image(64)),
    });

    commands.spawn((
        Mesh3d(meshes.add(Rectangle::default())),
        MeshMaterial3d(material),
        Transform::default(),
        NoFrustumCulling,
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(3.5, 2.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{
        CompareFunction, DepthBiasState, DepthStencilState, RenderPipelineDescriptor, StencilState,
        TextureFormat,
    };

    /// The whole point of the spike, in one assertion: Bevy's mesh pipeline
    /// sets `depth_write_enabled = false` for every blended pass
    /// (bevy_pbr::render::mesh, the BLEND_ALPHA branch), and `specialize`
    /// must flip it back.
    #[test]
    fn enable_depth_write_flips_the_flag() {
        let mut descriptor = RenderPipelineDescriptor {
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            ..Default::default()
        };

        enable_depth_write(&mut descriptor);

        assert_eq!(
            descriptor.depth_stencil.unwrap().depth_write_enabled,
            Some(true),
        );
    }

    /// A pipeline with no depth attachment must not panic. Bevy does not
    /// build one for this material, but `specialize` is called on a
    /// descriptor we do not own and this keeps the helper total.
    #[test]
    fn enable_depth_write_tolerates_no_depth_attachment() {
        let mut descriptor = RenderPipelineDescriptor::default();
        enable_depth_write(&mut descriptor);
        assert!(descriptor.depth_stencil.is_none());
    }

    /// The step atlas is what makes the experiment discriminating: one half
    /// of the sprite must land in front of the cube and the other behind it.
    /// A uniform atlas would prove nothing.
    #[test]
    fn depth_step_atlas_is_near_on_the_left_and_far_on_the_right() {
        let size = 8;
        let data = depth_step_rgba(size);
        assert_eq!(data.len(), (size * size * 4) as usize);

        let red_at = |x: u32, y: u32| data[((y * size + x) * 4) as usize];

        for y in 0..size {
            for x in 0..size / 2 {
                assert_eq!(red_at(x, y), 0, "left half is near (0.0) at ({x}, {y})");
            }
            for x in size / 2..size {
                assert_eq!(red_at(x, y), 255, "right half is far (1.0) at ({x}, {y})");
            }
        }
    }

    /// Guards the sign convention the shader depends on. Higher channel
    /// value means farther from the camera, and the cube's half-extent is
    /// 1.0, so each half must clear it.
    #[test]
    fn the_depth_range_pushes_each_half_clear_of_the_cube() {
        let offset = |d: f32| (d - SPIKE_DEPTH_PIVOT) * SPIKE_DEPTH_RANGE;
        assert!(
            offset(0.0) <= -2.0,
            "near half must sit in front of the cube"
        );
        assert!(offset(1.0) >= 2.0, "far half must sit behind the cube");
    }
}
