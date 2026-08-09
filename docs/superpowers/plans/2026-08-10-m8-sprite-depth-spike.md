# M8 Spike: Per-Pixel Sprite Depth — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove that an alpha-blended sprite quad can write per-pixel depth from a depth channel, so it interpenetrates an opaque 3D mesh rather than sitting wholly in front of or behind it.

**Architecture:** A throwaway `SpriteDepthMaterial` alongside the existing M1 `SpriteLayerMaterial`, using Bevy's `Material` trait. Three pieces make it work: `Material::specialize` flips `depth_stencil.depth_write_enabled` to `Some(true)` (Bevy's mesh pipeline sets it `false` for every blended pass); the fragment shader returns `@builtin(frag_depth)`; and that depth is computed by displacing the fragment's world position along the camera's forward axis by the sampled depth channel, then re-projecting through Bevy's own `clip_from_world`. Re-projecting rather than computing depth by hand is what makes this correct under Bevy's reverse-Z convention without any manual reverse-Z math.

**Tech Stack:** Rust 2024, Bevy 0.19 (`bevy_pbr` `Material` trait), wgpu 29, WGSL, naga (dev-dependency, shader validation).

## Global Constraints

- Bevy is pinned at `=0.19.0` and wgpu at `=29.0.4` in the workspace. Do not change either.
- `sway-runtime` depends on `bevy`, `bytemuck`, `sway-gpu`; dev-dependency `naga`. Do not add dependencies.
- The workspace builds with `-D warnings`. Clippy lints are errors.
- This is a **spike**. Its deliverable is a verified answer plus a findings report, not production code. Do not refactor `sprite_layer.rs`, `point_cloud.rs`, or `scatter.rs` — the M1 demos are kept intact as regression signals.
- Do not modify `crates/sway-runtime/src/sprite_layer.rs` or its shader at all. The spike lives in new files beside them.
- Architecture §9 says rendering is verified by eye, with no pixel-diff regression tests. The readback test in Task 2 is exempt as the spike's **measurement instrument** — it answers a go/no-go question rather than guarding against regressions. It follows the precedent already set by `headless.rs`'s `bevy_render_output_reaches_the_viewport_texture`.

## Verified facts (do not re-derive)

These were checked against the vendored crate sources before this plan was written. Trust them.

1. **The transparent pass CAN write depth.** `bevy_core_pipeline-0.19.0/src/core_3d/main_transparent_pass_3d_node.rs:82` binds `depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store))`. It is a writable attachment, not read-only, so `depth_write_enabled: Some(true)` is not a validation error. This was the one risk that could have invalidated the whole approach.
2. **Bevy forces depth-write off for blended materials.** `bevy_pbr-0.19.0/src/render/mesh.rs:3392-3397`: for `MeshPipelineKey::BLEND_ALPHA`, `depth_write_enabled = false`, with the comment "fragments that are closer will be alpha blended but their depth is not written". This is exactly what `specialize` must override.
3. **Bevy uses reverse-Z.** Same file, line ~3637: `format: CORE_3D_DEPTH_FORMAT` (`TextureFormat::Depth32Float`) and `depth_compare: Some(CompareFunction::GreaterEqual)`. Depth 1.0 is the near plane, 0.0 is far. The re-projection approach in this plan handles that automatically; **never hand-roll a depth value.**
4. **`depth_write_enabled` is an `Option<bool>`** in wgpu 29, not a bare `bool`. Assign `Some(true)`.
5. **The `specialize` signature** (`bevy_pbr-0.19.0/src/material.rs:272`) is:
   ```rust
   fn specialize(
       pipeline: &MaterialPipeline,
       descriptor: &mut RenderPipelineDescriptor,
       layout: &MeshVertexBufferLayoutRef,
       key: MaterialPipelineKey<Self>,
   ) -> Result<(), SpecializedMeshPipelineError>
   ```
   `RenderPipelineDescriptor` here is `bevy_material::descriptor::RenderPipelineDescriptor` (re-exported as `bevy::render::render_resource::RenderPipelineDescriptor`), **not** wgpu's. It derives `Default`, which is what makes Task 1's unit test possible.
6. **The material bind group is `@group(3)`.** `MATERIAL_BIND_GROUP_INDEX` is a fixed 3 in this Bevy version. `@group(0)` is the view, `@group(1)` the mesh-view binding arrays, `@group(2)` the mesh.
7. **Async pipeline compilation means a readback test needs a bounded poll loop**, not a fixed update count. A cold shader cache needed as many as 60 `app.update()` calls in this codebase; a warm one needed 3. See `headless.rs`'s test doc comment. Copy that pattern.

## File Structure

| File | Responsibility |
|---|---|
| `crates/sway-runtime/src/sprite_depth_spike.rs` (create) | The spike material, its `specialize` override, the pure depth-write helper, the generated test atlases, and the demo scene. Self-contained. |
| `crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl` (create) | Billboard vertex shader + a fragment shader returning colour and `frag_depth`. |
| `crates/sway-runtime/tests/sprite_depth_interpenetration.rs` (create) | The experiment: headless render, GPU readback, per-pixel assertions. An integration test, not a unit test — it needs a real device. |
| `crates/sway-runtime/src/lib.rs` (modify) | Export the new module. |
| `crates/sway-runtime/src/shader_validation.rs:29` (modify) | Add the new shader to `PREPROCESSOR_SHADERS`. |
| `crates/sway-app/src/main.rs` (modify) | A `--demo sprite-depth` variant for by-eye confirmation. |
| `docs/superpowers/reports/2026-08-10-sprite-depth-spike-findings.md` (create) | The go/no-go answer and what M8 inherits. |

## The scene, and why it discriminates

A 2×2×2 unlit green cube at the origin. A 3.0-wide red billboard quad, also centred at the origin, so its plane bisects the cube. Camera at `(0, 0, 6)` looking at the origin.

The depth atlas is a **step function**: the left half is `0.0`, the right half is `1.0`. Convention for this spike: **a higher channel value means farther from the camera.** With pivot `0.5` and range `4.0` world units:

- Left half: offset `(0.0 - 0.5) * 4.0 = -2.0` → 2 units toward the camera → **in front of** the cube (half-extent 1.0).
- Right half: offset `(1.0 - 0.5) * 4.0 = +2.0` → 2 units away → **behind** the cube.

So over the cube's screen area, the left half of the sprite must be visible (red dominant) and the right half must be hidden (green dominant). **Without** `frag_depth`, the whole quad sits at plane depth 0 — in front of nothing, behind nothing, tied with the cube centre — and both halves would render identically. That asymmetry is the proof.

Note that only depth changes: the fragment's screen position is unaffected, so the sprite does not shrink as its depth increases. That is correct and intended for a depth-channel billboard.

---

### Task 1: The depth-writing material and shader

**Files:**
- Create: `crates/sway-runtime/src/sprite_depth_spike.rs`
- Create: `crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl`
- Modify: `crates/sway-runtime/src/lib.rs`
- Modify: `crates/sway-runtime/src/shader_validation.rs:29`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces, for Tasks 2 and 3:
  - `pub struct SpriteDepthMaterial { pub layer: SpriteDepthUniform, pub color_texture: Handle<Image>, pub depth_texture: Handle<Image> }`
  - `pub struct SpriteDepthUniform { pub placement: Vec4, pub tint: Vec4, pub atlas: Vec4, pub depth_params: Vec4 }`
  - `pub struct SpriteDepthPlugin;`
  - `pub fn enable_depth_write(descriptor: &mut RenderPipelineDescriptor)`
  - `pub fn depth_step_rgba(size: u32) -> Vec<u8>`
  - `pub fn depth_step_image(size: u32) -> Image`
  - `pub fn solid_white_image(size: u32) -> Image`
  - `pub const SPIKE_DEPTH_PIVOT: f32 = 0.5;`
  - `pub const SPIKE_DEPTH_RANGE: f32 = 4.0;`

  Import paths, verified against the vendored sources: `MaterialPipeline` and
  `MaterialPipelineKey` from `bevy::pbr`; `MeshVertexBufferLayoutRef` from
  `bevy::mesh`; `RenderPipelineDescriptor` and `SpecializedMeshPipelineError`
  from `bevy::render::render_resource`.

- [ ] **Step 1: Write the failing unit tests**

Create `crates/sway-runtime/src/sprite_depth_spike.rs` containing only this test module for now, so the tests fail to compile against absent items:

```rust
//! M8 spike: does an alpha-blended sprite quad that writes per-pixel
//! `frag_depth` interpenetrate an opaque mesh?
//!
//! Throwaway by design. `sprite_layer.rs` is left untouched as an M1
//! regression signal; M8 rewrites it properly using whatever this proves.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{
        CompareFunction, DepthBiasState, DepthStencilState, RenderPipelineDescriptor,
        StencilState, TextureFormat,
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
        assert!(offset(0.0) <= -2.0, "near half must sit in front of the cube");
        assert!(offset(1.0) >= 2.0, "far half must sit behind the cube");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-runtime --lib sprite_depth_spike`
Expected: FAIL — compilation errors, `cannot find function 'enable_depth_write' in this scope` and similar for `depth_step_rgba`, `SPIKE_DEPTH_PIVOT`, `SPIKE_DEPTH_RANGE`.

- [ ] **Step 3: Write the shader**

Create `crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl`:

```wgsl
// M8 spike: a billboard quad that writes per-pixel depth from a depth sheet.
//
// Same three Material-trait constraints as sprite_layer.wgsl, for the same
// reasons documented there: Bevy's own `View` is imported rather than
// redeclared (a hand-rolled struct would misalign every field after the
// first); camera right/up come from `world_from_view`'s columns; and the
// material bind group is @group(3), because MATERIAL_BIND_GROUP_INDEX is a
// fixed 3 in this Bevy version.
//
// What is new here is the fragment stage. It returns @builtin(frag_depth)
// alongside colour, computed by displacing the fragment's world position
// along the camera's forward axis by the sampled depth channel and
// re-projecting through Bevy's own clip_from_world.
//
// Re-projecting rather than computing a depth value directly is deliberate.
// Bevy renders reverse-Z (Depth32Float, CompareFunction::GreaterEqual, so
// 1.0 is the near plane) with an infinite far plane. Hand-rolling a depth
// under that convention is easy to get subtly wrong and impossible to
// verify by eye. Pushing a world position back through the same matrix the
// vertex stage used cannot disagree with it.

#import bevy_pbr::mesh_view_bindings::view

struct Layer {
    // xy = world centre, z = depth, w = uniform scale
    placement: vec4<f32>,
    tint: vec4<f32>,
    // xy = atlas cell size in UV, zw = atlas cell offset
    atlas: vec4<f32>,
    // x = the sheet value meaning "at the quad's plane", y = world units
    // spanned by the full 0..1 depth channel, zw = unused
    depth_params: vec4<f32>,
};

@group(3) @binding(0) var<uniform> layer: Layer;
@group(3) @binding(1) var color_texture: texture_2d<f32>;
@group(3) @binding(2) var color_sampler: sampler;
@group(3) @binding(3) var depth_texture: texture_2d<f32>;
@group(3) @binding(4) var depth_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Carried through so the fragment stage can displace it and reproject.
    @location(1) world_position: vec3<f32>,
};

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let corner = in.position.xy;
    let camera_right = view.world_from_view[0].xyz;
    let camera_up = view.world_from_view[1].xyz;

    let centre = layer.placement.xyz;
    let scale = layer.placement.w;
    let world = centre
        + camera_right * corner.x * scale
        + camera_up * corner.y * scale;

    var out: VertexOut;
    out.clip_position = view.clip_from_world * vec4<f32>(world, 1.0);
    out.world_position = world;
    let cell_uv = corner + vec2<f32>(0.5, 0.5);
    out.uv = layer.atlas.zw + cell_uv * layer.atlas.xy;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> FragmentOut {
    let sampled = textureSample(color_texture, color_sampler, in.uv);
    let c = sampled * layer.tint;
    if (c.a < 0.001) {
        discard;
    }

    // Higher channel value means farther from the camera.
    let sheet_depth = textureSample(depth_texture, depth_sampler, in.uv).r;

    // `world_from_view[2]` is view-space +z expressed in world space, which
    // points back toward the viewer in a right-handed view space. Negating
    // it gives the direction away from the camera.
    let camera_forward = -view.world_from_view[2].xyz;
    let offset = (sheet_depth - layer.depth_params.x) * layer.depth_params.y;
    let displaced = in.world_position + camera_forward * offset;

    let clip = view.clip_from_world * vec4<f32>(displaced, 1.0);

    var out: FragmentOut;
    out.color = c;
    // Clamped because a fragment pushed behind the camera would divide by a
    // non-positive w and hand wgpu an out-of-range depth, which is
    // undefined. Clamping degrades to "pinned at the near or far plane",
    // which is the sane reading of an over-large depth range.
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
```

- [ ] **Step 4: Write the material**

Replace the contents of `crates/sway-runtime/src/sprite_depth_spike.rs`, keeping the `mod tests` block from Step 1 at the bottom:

```rust
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
    asset::{embedded_asset, embedded_path, AssetPath, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    },
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
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
/// (1.0). Stored `Rgba8UnormSrgb` for one reason — see `depth_step_image`.
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
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
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
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![255u8; (size * size * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}
```

Keep the `#[cfg(test)] mod tests` block from Step 1 at the end of the file.

- [ ] **Step 5: Export the module and allowlist the shader**

In `crates/sway-runtime/src/lib.rs`, add the module beside the others and re-export the plugin:

```rust
pub mod sprite_depth_spike;
```
```rust
pub use sprite_depth_spike::SpriteDepthPlugin;
```

In `crates/sway-runtime/src/shader_validation.rs:29`, extend the allowlist — the new shader uses `#import`, which naga cannot parse, and the harness fails any unlisted `#import` shader by design:

```rust
const PREPROCESSOR_SHADERS: &[&str] =
    &["point_cloud.wgsl", "sprite_layer.wgsl", "sprite_depth_spike.wgsl"];
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-runtime --lib`
Expected: PASS — the four new tests, plus `every_shader_parses_and_validates` still passing with `sprite_depth_spike.wgsl` reported in the "NOT VALIDATED (allowlisted...)" line.

Then check the build is clean: `cargo clippy -p sway-runtime --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-runtime/src/sprite_depth_spike.rs \
        crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl \
        crates/sway-runtime/src/lib.rs \
        crates/sway-runtime/src/shader_validation.rs
git commit -m "spike(runtime): a sprite material that writes per-pixel depth

Bevy's mesh pipeline forces depth_write_enabled = false for every blended
pass; Material::specialize flips it back. The fragment stage returns
frag_depth from a re-projected displaced world position rather than from
arithmetic on a depth value, because Bevy renders reverse-Z and the direct
route is easy to get subtly wrong and impossible to check by eye.

Proves nothing on its own -- the experiment is the next commit."
```

---

### Task 2: The experiment

**Files:**
- Create: `crates/sway-runtime/tests/sprite_depth_interpenetration.rs`

**Interfaces:**
- Consumes: `SpriteDepthMaterial`, `SpriteDepthUniform`, `SpriteDepthPlugin`, `depth_step_image`, `solid_white_image`, `SPIKE_DEPTH_PIVOT`, `SPIKE_DEPTH_RANGE` from Task 1. Also `sway_runtime::headless::build_app`, `sway_gpu::GpuContext::new`, `sway_gpu::ViewportTexture::new` and its `texture()` accessor.
- Produces: the spike's answer. Nothing later depends on its symbols.

This is an integration test (`tests/`, not `#[cfg(test)]`) because it needs `sway_runtime`'s public API plus a real GPU device.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-runtime/tests/sprite_depth_interpenetration.rs`:

```rust
//! The M8 spike's measurement instrument.
//!
//! Architecture §9 says rendering is verified by eye with no pixel-diff
//! tests. This is exempt: it answers a go/no-go question rather than
//! guarding a regression, and squinting at a screenshot is not a
//! trustworthy way to answer it. It follows the readback precedent set by
//! `headless.rs`'s `bevy_render_output_reaches_the_viewport_texture`.
//!
//! The scene: a 2x2x2 unlit green cube at the origin, and a 3.0-wide red
//! billboard also centred at the origin, so its plane bisects the cube. The
//! depth sheet is a step -- left half near, right half far -- which with a
//! pivot of 0.5 and a range of 4.0 puts the left half 2 units in front of
//! the cube and the right half 2 units behind it.
//!
//! So over the cube, the left half of the sprite must win and the right
//! half must lose. Without frag_depth the whole quad sits at plane depth 0
//! and both halves render identically -- that asymmetry is the entire
//! proof.

use bevy::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use sway_gpu::wgpu;
use sway_runtime::sprite_depth_spike::{
    depth_step_image, solid_white_image, SpriteDepthMaterial, SpriteDepthPlugin,
    SpriteDepthUniform, SPIKE_DEPTH_PIVOT, SPIKE_DEPTH_RANGE,
};

const VIEWPORT: u32 = 64;
const ATLAS: u32 = 16;
const QUAD_SCALE: f32 = 3.0;

/// Reads the whole viewport back as RGBA8 rows.
///
/// `bytes_per_row` must be padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256);
/// wgpu does not do it for you. Mapping is async, so `device.poll` has to
/// drive the callback or the recv below hangs forever.
fn read_pixels(gpu: &sway_gpu::GpuContext, viewport: &sway_gpu::ViewportTexture) -> Vec<[u8; 4]> {
    let bytes_per_pixel = 4u32;
    let unpadded = VIEWPORT * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite depth spike readback"),
        size: u64::from(padded) * u64::from(VIEWPORT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: viewport.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(VIEWPORT),
            },
        },
        wgpu::Extent3d {
            width: VIEWPORT,
            height: VIEWPORT,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    rx.recv().expect("map_async never ran").expect("mapping failed");

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((VIEWPORT * VIEWPORT) as usize);
    for row in 0..VIEWPORT {
        let start = (row * padded) as usize;
        for col in 0..VIEWPORT {
            let at = start + (col * bytes_per_pixel) as usize;
            pixels.push([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        }
    }
    drop(data);
    readback.unmap();
    pixels
}

fn pixel(pixels: &[[u8; 4]], x: u32, y: u32) -> [u8; 4] {
    pixels[(y * VIEWPORT + x) as usize]
}

fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut sprites: ResMut<Assets<SpriteDepthMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Unlit so the assertion does not depend on a light rig.
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
            placement: Vec3::ZERO.extend(QUAD_SCALE),
            tint: Vec4::new(1.0, 0.0, 0.0, 0.85),
            atlas: Vec4::new(1.0, 1.0, 0.0, 0.0),
            depth_params: Vec4::new(SPIKE_DEPTH_PIVOT, SPIKE_DEPTH_RANGE, 0.0, 0.0),
        },
        color_texture: images.add(solid_white_image(ATLAS)),
        depth_texture: images.add(depth_step_image(ATLAS)),
    });

    commands.spawn((
        Mesh3d(meshes.add(Rectangle::default())),
        MeshMaterial3d(material),
        // The shader billboards out of the uniform and never reads this,
        // but the transparent pass sorts by GlobalTransform, so the two
        // must agree. Same trade-off sprite_layer.rs documents.
        Transform::default(),
        NoFrustumCulling,
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[test]
fn a_sprite_with_a_depth_channel_interpenetrates_a_cube() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(VIEWPORT, VIEWPORT);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size);
    app.add_plugins(SpriteDepthPlugin)
        .add_systems(Startup, setup_scene);
    app.finish();
    app.cleanup();

    // At distance 6 with the default 45-degree FOV the visible half-height
    // is 6 * tan(22.5) ~= 2.49, so the 2.0-wide cube spans roughly the
    // middle 40% of the frame: x in [0.3, 0.7] of the width. These two
    // samples sit inside the cube, one either side of the sprite's centre
    // (and therefore either side of the depth sheet's step).
    let near_half = (VIEWPORT as f32 * 0.40) as u32;
    let far_half = (VIEWPORT as f32 * 0.60) as u32;
    let mid = VIEWPORT / 2;

    // Bounded poll, not a fixed count: bevy_core_pipeline's upscaling
    // pipeline compiles asynchronously, and until it is ready the viewport
    // is cleared to the wrong colour with no validation error. Cold caches
    // in this codebase have needed as many as 60 updates. See the doc
    // comment on headless.rs's readback test.
    const MAX_UPDATES: u32 = 300;
    let mut pixels = Vec::new();
    let mut converged = None;
    for updates in 1..=MAX_UPDATES {
        app.update();
        pixels = read_pixels(&gpu, &viewport);
        let near = pixel(&pixels, near_half, mid);
        if near[0] > 150 && near[1] < 100 {
            converged = Some(updates);
            break;
        }
    }

    let near = pixel(&pixels, near_half, mid);
    let far = pixel(&pixels, far_half, mid);

    assert!(
        converged.is_some(),
        "the sprite's near half never rendered red after {MAX_UPDATES} updates \
         (last read {near:?}); either nothing drew at all or the pipeline never \
         finished compiling"
    );

    // The near half is 2 units in front of the cube: red must win.
    assert!(
        near[0] > near[1],
        "near half at x={near_half} should be sprite-red, got {near:?}"
    );

    // The far half is 2 units behind the cube: green must win. This is the
    // assertion that fails without frag_depth -- the quad would be one flat
    // plane and this pixel would be red too.
    assert!(
        far[1] > far[0],
        "far half at x={far_half} should be cube-green (the sprite is behind \
         the cube there), got {far:?}. If this is red, per-pixel depth is not \
         reaching the depth buffer: check that specialize actually ran and that \
         frag_depth is being written."
    );

    eprintln!(
        "sprite depth spike: converged after {} update(s); near={near:?} far={far:?}",
        converged.unwrap()
    );
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p sway-runtime --test sprite_depth_interpenetration -- --nocapture`

**This step is the spike.** Three outcomes, all informative:

- **PASS** — per-pixel depth works. Record the converged update count and both pixel values; go to Task 3.
- **FAIL on the `far` assertion** (far pixel is red) — depth is not reaching the buffer. Check in this order: (a) is `specialize` running at all — add a `dbg!` inside `enable_depth_write`; (b) is `AlphaMode::Blend` putting this in `Transparent3d` as expected; (c) does the sprite draw *before* the cube, so it depth-tests against nothing — the transparent phase runs after opaque, so it should not, but confirm.
- **A wgpu validation error** — capture the full message verbatim. It contradicts verified fact 1 and reshapes M8. Go to Task 3 and write the report anyway; a negative result is the spike's deliverable too.

If the near-half assertion fails while the far half passes, the sign convention is inverted: flip `camera_forward` in the shader and note it.

- [ ] **Step 3: Commit**

```bash
git add crates/sway-runtime/tests/sprite_depth_interpenetration.rs
git commit -m "spike(runtime): prove per-pixel sprite depth against a cube

A step depth sheet puts one half of a billboard in front of an opaque cube
and the other half behind it. Over the cube, the near half must render as
sprite and the far half as cube -- an asymmetry that is impossible without
frag_depth, since a flat quad would render both halves identically.

Readback rather than by-eye: architecture section 9 exempts a spike's own
measurement instrument, and squinting is not a trustworthy way to answer a
go/no-go."
```

---

### Task 3: By-eye confirmation and the findings report

**Files:**
- Modify: `crates/sway-app/src/main.rs` (the `Demo` enum, `parse_args`, and the demo-dispatch match)
- Create: `docs/superpowers/reports/2026-08-10-sprite-depth-spike-findings.md`

**Interfaces:**
- Consumes: `SpriteDepthPlugin` and the material types from Task 1; the verdict from Task 2.
- Produces: the report M8's plan will be written against.

The readback test proves the depth test. It does not prove the result *looks* right — blending, tint and sort order still want a human. Hence a `--demo` variant, matching the existing pattern where demo spawning is a `pub fn` outside `Plugin::build`.

- [ ] **Step 1: Add the demo scene function**

Append to `crates/sway-runtime/src/sprite_depth_spike.rs` (before the `mod tests` block). This is the same scene as the test but at a viewable scale, with the camera slightly off-axis so the interpenetration reads as depth rather than as a flat mask:

```rust
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
```

- [ ] **Step 2: Wire up the `--demo` flag**

In `crates/sway-app/src/main.rs`, add a variant to the `Demo` enum:

```rust
enum Demo {
    PointCloud,
    Sprites,
    SpriteDepth,
    Scatter,
    All,
}
```

Add its parse arm in `parse_args`, beside `"sprites"`:

```rust
"sprite-depth" => Demo::SpriteDepth,
```

Add its dispatch arm in the `match demo` block, beside `Some(Demo::Sprites)`. It spawns its own camera inside `spawn_depth_spike_scene`, so — per the camera-collision hazard documented on that match — it must not also get `setup_scene`:

```rust
Some(Demo::SpriteDepth) => {
    app.add_plugins(sway_runtime::SpriteDepthPlugin)
        .add_systems(
            Startup,
            sway_runtime::sprite_depth_spike::spawn_depth_spike_scene,
        );
}
```

Leave `Demo::All` alone — the spike scene spawns a third camera and would collide with the others.

- [ ] **Step 3: Look at it**

Run: `cargo run -p sway-app -- --demo sprite-depth --windowed`

Expected: a green cube with a red rectangle through it. The rectangle's left half floats in front of the cube; its right half is hidden where the cube covers it and visible where it overhangs. The boundary between the two halves should sit exactly at the sprite's vertical midline, and the cube's silhouette should cut the right half cleanly.

If it renders but looks wrong, screenshot it and describe the discrepancy in the report — that is a finding, not a failure of the task.

- [ ] **Step 4: Write the findings report**

Create `docs/superpowers/reports/2026-08-10-sprite-depth-spike-findings.md`. Fill in every bracketed value from what actually happened; do not leave a bracket in the committed file:

```markdown
# M8 spike — per-pixel sprite depth: findings

**Date:** 2026-08-10
**Verdict:** [GO / GO WITH CAVEATS / NO-GO]
**Plan:** [`2026-08-10-m8-sprite-depth-spike.md`](../plans/2026-08-10-m8-sprite-depth-spike.md)
**Spec:** decision D3 in [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)

## Question

Can an alpha-blended sprite quad write per-pixel depth from a depth channel,
so it interpenetrates opaque meshes and other sprite layers, rather than
sitting wholly in front of or behind them?

## Answer

[Two or three sentences. Lead with the verdict.]

## What was built

- `SpriteDepthMaterial` in `crates/sway-runtime/src/sprite_depth_spike.rs`
- `crates/sway-runtime/assets/shaders/sprite_depth_spike.wgsl`
- `crates/sway-runtime/tests/sprite_depth_interpenetration.rs`
- `--demo sprite-depth` in `sway-app`

## Measured

- Integration test: [PASS / FAIL]
- Converged after [N] `app.update()` calls (cold cache: [N])
- Near-half pixel: [RGBA]. Far-half pixel: [RGBA].
- By-eye check: [what was actually on screen]

## What M8 inherits

- [Does `specialize` need anything beyond the depth-write flip?]
- [Does the depth sheet want its own sampler settings — filtering across a
  depth discontinuity interpolates between near and far, which will fringe.
  Was that visible at the step? Does M8 need NEAREST filtering, or a
  separate depth sampler?]
- [How does layer-vs-layer interpenetration behave, as opposed to
  layer-vs-mesh? Untested here — one quad only.]
- [Anything about the sRGB-vs-linear choice for the depth texture that only
  showed up once real midtone depth values were involved.]

## Surprises

[Anything that contradicted the pre-verified facts in the plan, or that cost
more than ten minutes. If nothing did, say so.]

## Not answered

- Atlas-cell animation — out of scope here, M8 proper.
- More than one sprite layer at once.
- Performance. One quad says nothing about the real layer count.
```

- [ ] **Step 5: Verify the whole workspace is still green**

Run: `cargo test --workspace`
Expected: PASS, including the M1 demos' own tests, which this spike must not have touched.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-runtime/src/sprite_depth_spike.rs \
        crates/sway-app/src/main.rs \
        docs/superpowers/reports/2026-08-10-sprite-depth-spike-findings.md
git commit -m "spike(app): --demo sprite-depth, and the spike's findings

The readback test proves the depth test; it does not prove the result looks
right. The demo is the by-eye half, off-axis because straight on a correct
per-pixel depth and a flat alpha mask are indistinguishable.

Findings recorded for M8 to be planned against."
```

---

## After the spike

The verdict decides what happens to M8 as specified in D3:

- **GO** — M8 proceeds as written. `sprite_layer.rs` is rewritten around what this proved, adding the colour+depth atlas pair, atlas-cell animation, and the graph-authored `SpriteLayer` / `SpriteAnim` components. The spike files are deleted in that commit; the findings report is what survives.
- **GO WITH CAVEATS** — record the caveats in the report's "What M8 inherits" section and re-scope M8 before planning it.
- **NO-GO** — D3 is reopened. The fallback ranking, from the brainstorm that produced D3: alpha-tested per-pixel depth in the opaque pass (keeps interpenetration, loses soft edges), then per-layer flat depth (keeps blending, loses interpenetration). Both are cheaper than what was attempted here; neither needs a further spike.
