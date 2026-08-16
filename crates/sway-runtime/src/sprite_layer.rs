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
    asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path},
    camera::visibility::NoFrustumCulling,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat},
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

// --- Demo: five overlapping layers at distinct depths -----------------

/// Side length, in pixels, of the generated demo texture.
const DEMO_TEXTURE_SIZE: u32 = 64;

/// Number of demo sprite layers. M1 exit condition (see task-4-brief.md):
/// five layers, correctly depth-sorted, alpha-blended, at frame rate.
const DEMO_LAYER_COUNT: usize = 5;
/// World-space xy step between successive layers' centres. Kept small
/// relative to `DEMO_LAYER_SCALE` so that even the two most-distant layers
/// (index 0 and `DEMO_LAYER_COUNT - 1`) still overlap on screen: total
/// spread is `(COUNT - 1) * XY_STEP` = 2.4, comfortably less than one quad's
/// full width (`DEMO_LAYER_SCALE` = 3.5).
const DEMO_LAYER_XY_STEP: f32 = 0.6;
/// World-space z step between successive layers (negative: each later layer
/// sits farther from the demo camera).
const DEMO_LAYER_Z_STEP: f32 = -2.5;
/// Uniform scale applied to every demo quad.
const DEMO_LAYER_SCALE: f32 = 3.5;

/// One RGBA tint per demo layer, nearest-to-farthest. Alpha < 1 so
/// overlapping layers visibly blend rather than fully occlude, which is
/// part of what makes correct depth *order* (not just correct depth test)
/// visible by eye.
const DEMO_LAYER_TINTS: [[f32; 4]; DEMO_LAYER_COUNT] = [
    [0.90, 0.20, 0.20, 0.85], // red    — nearest
    [0.95, 0.55, 0.15, 0.85], // orange
    [0.25, 0.80, 0.30, 0.85], // green
    [0.25, 0.45, 0.95, 0.85], // blue
    [0.65, 0.30, 0.85, 0.85], // violet — farthest
];

/// Placement + tint for one demo layer.
struct DemoLayer {
    position: Vec3,
    tint: Vec4,
}

/// Hardcoded demo layout: five layers on a diagonal line receding from the
/// demo camera, each offset from the last in both xy (so they don't sit
/// exactly on top of one another on screen) and z (so distance-based
/// transparent-pass sorting has real work to do). Pure and GPU-free, so it
/// is unit-tested directly below.
fn demo_layers() -> [DemoLayer; DEMO_LAYER_COUNT] {
    core::array::from_fn(|i| {
        let i_f = i as f32;
        DemoLayer {
            position: Vec3::new(
                i_f * DEMO_LAYER_XY_STEP,
                i_f * DEMO_LAYER_XY_STEP,
                i_f * DEMO_LAYER_Z_STEP,
            ),
            tint: Vec4::from(DEMO_LAYER_TINTS[i]),
        }
    })
}

/// Builds a soft radial-gradient RGBA image: opaque white at the centre,
/// fading to fully transparent at the edge. Generated in code — no asset
/// file — per the brief's step 4. Pure and GPU-free, so it is unit-tested
/// directly below.
fn radial_gradient_rgba(size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size as usize) * (size as usize) * 4);
    let centre = (size as f32 - 1.0) / 2.0;
    let max_radius = centre.max(1.0);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let normalized = (dx * dx + dy * dy).sqrt() / max_radius;
            let t = (1.0 - normalized).clamp(0.0, 1.0);
            // smoothstep: a soft falloff rather than a hard-edged disc.
            let alpha = t * t * (3.0 - 2.0 * t);
            data.extend_from_slice(&[255, 255, 255, (alpha * 255.0).round() as u8]);
        }
    }
    data
}

fn radial_gradient_image(size: u32) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        radial_gradient_rgba(size),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Demo setup: spawns five `SpriteLayerMaterial` billboards sharing one quad
/// mesh and one generated texture. Kept out of `SpriteLayerPlugin::build`
/// (see its docs) — Task 6 wires this up behind `sway-app`'s `--demo` flag.
pub fn spawn_demo_sprite_layers(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SpriteLayerMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let quad = meshes.add(Rectangle::default());
    let texture = images.add(radial_gradient_image(DEMO_TEXTURE_SIZE));

    for layer in demo_layers() {
        let material = materials.add(SpriteLayerMaterial {
            layer: LayerUniform {
                placement: layer.position.extend(DEMO_LAYER_SCALE),
                tint: layer.tint,
                atlas: Vec4::new(1.0, 1.0, 0.0, 0.0),
            },
            texture: texture.clone(),
        });

        commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(material),
            // The shader computes world position straight from the `Layer`
            // uniform above and never reads this Transform — but Bevy's
            // transparent-pass sort (needed for correct back-to-front alpha
            // blending, since AlphaMode::Blend disables depth *write*) is
            // computed from each entity's GlobalTransform via its mesh AABB
            // centre, not from shader-side data. The two positions must be
            // kept in agreement or the layers blend in the wrong order
            // despite each rendering at the right depth.
            Transform::from_translation(layer.position),
            // The mesh here is `Rectangle::default()` (local half-size 0.5,
            // flat in z), but the shader billboards it out to a half-size of
            // `DEMO_LAYER_SCALE` *along the camera's own right/up axes* (see
            // the vertex shader), not along this entity's local axes. Bevy's
            // frustum culling checks the mesh's local AABB against
            // `GlobalTransform` — a `Transform::with_scale` could inflate
            // that AABB to match at the current axis-aligned demo camera,
            // but since the flat mesh has zero local z-extent, a uniform
            // local-axis scale stops bounding the actual billboard as soon
            // as the camera's right/up vectors pick up any out-of-plane
            // (roll/tilt) component — exactly the kind of repositioning
            // Task 6 might introduce. `NoFrustumCulling` sidesteps that by
            // exempting the entity from the check entirely, camera
            // orientation notwithstanding; the same trade-off `point_cloud.rs`
            // makes for its own camera-relative instanced draw.
            NoFrustumCulling,
        ));
    }
}

/// A camera positioned to see the whole demo stack. Separate from
/// `spawn_demo_sprite_layers` because `sway-app`'s existing scene already
/// spawns its own camera; Task 6 decides whether to use this one or that
/// one when wiring up `--demo`.
pub fn spawn_demo_camera(mut commands: Commands) {
    let stack_centre = Vec3::new(
        DEMO_LAYER_XY_STEP * 2.0,
        DEMO_LAYER_XY_STEP * 2.0,
        DEMO_LAYER_Z_STEP * 2.0,
    );
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(stack_centre.x, stack_centre.y, 14.0).looking_at(stack_centre, Vec3::Y),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_layers_are_distinct_and_recede_in_depth() {
        let layers = demo_layers();
        assert_eq!(layers.len(), DEMO_LAYER_COUNT);

        // Each layer sits strictly farther (more negative z) than the last,
        // and strictly offset in xy — otherwise there would be nothing for
        // depth sorting to prove.
        for pair in layers.windows(2) {
            assert!(pair[1].position.z < pair[0].position.z);
            assert!(pair[1].position.x > pair[0].position.x);
            assert!(pair[1].position.y > pair[0].position.y);
        }

        // Tints must actually differ, or overlap wouldn't be visible by eye.
        for i in 0..layers.len() {
            for j in (i + 1)..layers.len() {
                assert_ne!(layers[i].tint, layers[j].tint);
            }
        }
    }

    #[test]
    fn radial_gradient_is_opaque_at_centre_and_transparent_at_corners() {
        let size = 16;
        let rgba = radial_gradient_rgba(size);
        assert_eq!(rgba.len(), (size as usize) * (size as usize) * 4);

        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let i = ((y * size + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };

        let centre = size / 2;
        let [r, g, b, a] = pixel(centre, centre);
        assert_eq!((r, g, b), (255, 255, 255));
        assert!(a > 240, "centre alpha should be near-opaque, was {a}");

        let [.., corner_a] = pixel(0, 0);
        assert!(
            corner_a < 15,
            "corner alpha should be near-transparent, was {corner_a}"
        );
    }

    #[test]
    fn radial_gradient_falls_off_monotonically_along_a_radius() {
        // Sampling straight out from the centre, alpha should never increase
        // — a soft single-lobed falloff, not something with rings or noise.
        let size = 32;
        let rgba = radial_gradient_rgba(size);
        let centre = size / 2;
        let mut previous = 255u8;
        for x in centre..size {
            let i = ((centre * size + x) * 4 + 3) as usize;
            let alpha = rgba[i];
            assert!(
                alpha <= previous,
                "alpha rose from {previous} to {alpha} at x={x}"
            );
            previous = alpha;
        }
    }
}
