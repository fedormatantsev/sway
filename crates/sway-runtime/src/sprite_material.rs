//! `SpriteMaterial` — a sprite sheet as a material node, whose depth run
//! displaces the geometry it is wired to.
//!
//! The M8 spike (`sprite_depth_spike.rs`) proved exactly one thing that
//! survives into this module: `Material::specialize` can flip
//! `depth_stencil.depth_write_enabled` back on for an alpha-blended pass, so
//! sprite layers occlude and interpenetrate each other. Everything else about
//! the spike is deliberately abandoned — design D1 replaces its
//! `@builtin(frag_depth)` fragment displacement with honest vertex
//! displacement, because a fragment rasterized from a flat quad cannot show
//! parallax no matter what depth it writes. There is therefore no reverse-Z
//! reprojection here, no `placement` uniform, and no billboarding: placement
//! comes from the entity's own `Transform`, like any other mesh.

use crate::frame_sequence::changed_id;
use bevy::{
    asset::{AssetEvent, AssetId, AssetPath, embedded_asset, embedded_path},
    camera::{
        primitives::{Aabb, MeshAabb},
        visibility::NoAutoAabb,
    },
    ecs::change_detection::DetectChangesMut,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// Matches `SpriteMaterialUniform` in `sprite_material.wgsl` field for field.
///
/// `PartialEq` is not decoration: [`sync_sprite_materials`] compares the
/// uniform it is about to publish against the one it published last, and that
/// comparison is the whole of its change detection.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct SpriteMaterialUniform {
    /// rgb = tint in **linear** space, a = opacity. Packed together because
    /// they are always applied together, and because a lone `f32` after a
    /// `vec4` costs nothing while a second `vec4` would.
    pub tint: Vec4,
    /// The array layer to sample, already clamped by `layer_index`. The
    /// shader stays dumb about frame numbers (design D4): it receives an
    /// index, never a float to interpret.
    pub layer: u32,
    pub depth_pivot: f32,
    pub depth_range: f32,
}

/// Both runs bind as `texture_2d_array` (`dimension = "2d_array"` — the
/// attribute defaults to `D2`, which would silently disagree with the WGSL).
/// Array layers rather than a packed grid is design D7: layers have no
/// neighbours to bleed from, which matters far more than usual here because
/// the depth run is sampled in the *vertex* stage, where a neighbouring
/// frame's height would be pulled into the geometry itself.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SpriteMaterialAsset {
    #[uniform(0)]
    pub uniform: SpriteMaterialUniform,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    pub color_texture: Handle<Image>,
    #[texture(3, dimension = "2d_array")]
    #[sampler(4)]
    pub depth_texture: Handle<Image>,
}

/// Truncates a frame number toward negative infinity and clamps it into
/// `[0, layers)`.
///
/// **Clamp, never a wrap.** Modulo looks like the obvious choice for a frame
/// counter, but wrapping *is* looping, and looping is animation policy
/// (design D4). Putting it here would silently impose one playback behaviour
/// and make ping-pong or hold-at-end unreachable, because the wrap would
/// already have happened before any node could act on it. The graph expresses
/// playback instead — `MidiTime → Oscillator(Saw) → Remap(-1..1 → 0..layers)`
/// is a loop, and swapping `Saw` for `Triangle` is a ping-pong — where the
/// choice is visible on the canvas rather than buried in this function.
///
/// So this is a safeguard with no expressive content: the minimum needed to
/// guarantee the sequence is never sampled outside its own layers. A frame
/// number running monotonically past the end holds on the last layer instead
/// of looping. That is the correct failure for a safeguard — visibly stuck,
/// rather than quietly doing something the author did not ask for.
///
/// Clamping lives on the *read* side so an authored `37.5` and a wired `37.5`
/// select the same layer, which they could not if the writing side enforced
/// range.
///
/// `layers == 0` returns 0, which is not a valid layer of an empty texture:
/// a material with no usable run must not be drawn at all (the spec's "an
/// incomplete material renders nothing"), and there is no in-range answer to
/// give. Returning 0 keeps the function total rather than pretending it can
/// rescue that case.
pub fn layer_index(frame: f32, layers: u32) -> u32 {
    // Via i64 rather than a direct `as u32`: `f32 as u32` saturates negatives
    // to 0, which would accidentally do the right thing here and hide the
    // clamp, and `saturating_sub` keeps `layers == 0` from underflowing. NaN
    // and both infinities are total under this path — NaN casts to 0, ±inf
    // saturate — so no frame value can escape the range.
    let last = i64::from(layers.saturating_sub(1));
    (frame.floor() as i64).clamp(0, last) as u32
}

/// Turns depth writes back on.
///
/// Bevy's mesh pipeline sets `depth_write_enabled = false` for every blended
/// pass (`bevy_pbr::render::mesh`, the `BLEND_ALPHA` branch, whose comment
/// reads "their depth is not written to the depth buffer") — a reasonable
/// default for ordinary transparency, where writing depth would let a
/// transparent surface occlude the surfaces behind it that still need to be
/// blended in. A sprite layer wants exactly that occlusion, so the flag has
/// to be flipped back. Safe because the transparent pass binds its depth
/// attachment with `StoreOp::Store`, writable, not read-only
/// (`main_transparent_pass_3d_node.rs`). This is the one thing the M8 spike
/// proved that survives design D1.
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

fn sprite_material_shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("../assets/shaders/sprite_material.wgsl"))
            .with_source("embedded"),
    )
}

impl Material for SpriteMaterialAsset {
    fn vertex_shader() -> ShaderRef {
        sprite_material_shader()
    }

    fn fragment_shader() -> ShaderRef {
        sprite_material_shader()
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

/// Cached undisplaced mesh AABB, so a mesh edit can re-inflate without recomputing.
#[derive(Component, Debug, Clone)]
pub struct SpriteMeshBounds {
    mesh: AssetId<Mesh>,
    base: Aabb,
}

/// The largest distance, in world units, that the shader can move a vertex off
/// the undisplaced surface.
///
/// The shader computes `(height - depth_pivot) * depth_range` for a sampled
/// `height` in `[0, 1]`, so the extreme is at one end of that range or the
/// other; which one depends on the pivot, and both are taken because the
/// displacement follows the mesh's normals and can therefore point anywhere.
pub fn max_displacement(depth_range: f32, depth_pivot: f32) -> f32 {
    depth_range.abs() * depth_pivot.abs().max((1.0 - depth_pivot).abs())
}

/// Grows a local-space [`Aabb`] so its world image still contains geometry
/// displaced by `displacement` **world** units in any direction.
///
/// **The unit mismatch this exists to fix.** `sprite_material.wgsl` displaces
/// in world space, along a normal that `mesh_normal_local_to_world` has already
/// normalized, so `depth_range` is a length in world units. `Aabb` is in *local*
/// space: `Frustum::intersects_obb` transforms it by the entity's affine, and
/// `Aabb::relative_radius` shows exactly how — the extent it contributes along
/// a world direction `n` is `Σᵢ |n · colᵢ| · half_extentsᵢ`, where `colᵢ` are
/// the affine's basis columns. Adding `depth_range` straight to `half_extents`
/// would therefore bound correctly only at scale 1, and would under-bound
/// (culling visible geometry) at any scale above it.
///
/// So each half-extent grows by `displacement / scaleᵢ`. That is exact rather
/// than merely conservative: for a shear-free affine the columns are orthogonal
/// with lengths `scaleᵢ`, so growing axis `i` by `d / scaleᵢ` adds
/// `Σᵢ |n · colᵢ / scaleᵢ| · d ≥ d` for any unit `n`, the inequality holding
/// because the normalized columns are an orthonormal basis. Per-axis, not one
/// scalar divisor — a single divisor has to be the *smallest* scale component
/// to stay safe, and then over-inflates every other axis by the aspect ratio.
///
/// A scale component at or near zero has no honest answer: the entity's world
/// image is flat along that axis, while the displacement is not, so no finite
/// local half-extent bounds it. The divisor is floored at `MIN_SCALE`, which
/// makes the bound enormous rather than infinite — conservative, so a degenerate
/// entity is never culled, and finite, so no `NaN` reaches the frustum test.
pub fn inflate_local_aabb(base: Aabb, displacement: f32, scale: Vec3) -> Aabb {
    const MIN_SCALE: f32 = 1e-4;
    // A non-finite displacement means a non-finite `depth_range`, which already
    // produces non-finite vertex positions that the rasterizer drops. Bounding
    // it would only propagate `NaN` into the frustum test, where it compares
    // false and culls the entity outright.
    if !displacement.is_finite() {
        return base;
    }
    let scale = Vec3::new(scale.x.abs(), scale.y.abs(), scale.z.abs()).max(Vec3::splat(MIN_SCALE));
    Aabb {
        center: base.center,
        half_extents: base.half_extents + Vec3A::from(Vec3::splat(displacement.abs()) / scale),
    }
}

/// Keeps an explicit, displacement-aware [`Aabb`] on every mesh carrying a
/// sprite material.
///
/// Design, Risks: *"An explicit `Aabb` inflated by `depth_range` is inserted,
/// rather than inheriting M1's `NoFrustumCulling`."* `sprite_layer.rs` disabled
/// culling wholesale to paper over a bounds mismatch; this material composes
/// with arbitrary geometry, where switching culling off is a cost the author
/// never asked for and never sees.
///
/// **Not fighting `calculate_bounds`.** Bevy's own bounds system has two
/// queries (`bevy_camera::visibility`): one inserts an `Aabb` on entities that
/// lack one, and one *overwrites* the `Aabb` from the mesh whenever `Mesh3d` or
/// the mesh asset changes. The second is the real hazard — an inflated bound
/// would be silently replaced by the raw mesh bound on every mesh edit, which
/// is exactly when it matters. Both queries carry `Without<NoAutoAabb>`, so
/// this system inserts that marker alongside its first `Aabb` and Bevy's
/// bookkeeping stands down permanently. Ordering then does not matter: this
/// runs in `Update` and `calculate_bounds` in `PostUpdate`, and the one frame
/// before the marker lands is at worst a raw mesh bound that this system
/// immediately supersedes.
///
/// The depth parameters are read from the *asset* rather than from the
/// `SpriteMaterial` node, so this system needs no knowledge of which wire
/// delivered the material — and an incomplete material has no asset, which is
/// the same condition under which nothing is drawn.
///
/// *Accepted loose end:* disconnecting the material wire removes the
/// `MeshMaterial3d` (design D5) and this system stops seeing the entity, so the
/// inflated `Aabb` and the `NoAutoAabb` marker stay behind. The consequence is
/// bounded and one-directional — an over-large bound culls less, never more, so
/// nothing can disappear — and the entity draws nothing at all until it is
/// wired again anyway. Tearing them down would need a hook on a component this
/// module does not own.
#[allow(clippy::type_complexity)] // an ECS query tuple, not a type to simplify
pub fn sync_sprite_material_bounds(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<SpriteMaterialAsset>>,
    mut mesh_messages: MessageReader<AssetEvent<Mesh>>,
    mut consumers: Query<(
        Entity,
        &Mesh3d,
        &MeshMaterial3d<SpriteMaterialAsset>,
        &GlobalTransform,
        Option<&mut Aabb>,
        Option<&mut SpriteMeshBounds>,
        Has<NoAutoAabb>,
    )>,
) {
    let touched: Vec<AssetId<Mesh>> = mesh_messages.read().filter_map(changed_id).collect();

    for (entity, mesh, material, transform, aabb, bounds, marked) in &mut consumers {
        let Some(material) = materials.get(&material.0) else {
            continue;
        };
        let mesh_id = mesh.id();
        let stale = touched.contains(&mesh_id);
        let base = match bounds {
            Some(cache) if cache.mesh == mesh_id && !stale => cache.base,
            Some(mut cache) => {
                let Some(base) = meshes.get(mesh).and_then(Mesh::compute_aabb) else {
                    continue;
                };
                cache.mesh = mesh_id;
                cache.base = base;
                base
            }
            None => {
                let Some(base) = meshes.get(mesh).and_then(Mesh::compute_aabb) else {
                    continue;
                };
                commands.entity(entity).insert(SpriteMeshBounds {
                    mesh: mesh_id,
                    base,
                });
                base
            }
        };

        let inflated = inflate_local_aabb(
            base,
            max_displacement(material.uniform.depth_range, material.uniform.depth_pivot),
            transform.scale(),
        );
        match aabb {
            Some(mut existing) => {
                existing.set_if_neq(inflated);
            }
            None => {
                commands.entity(entity).insert(inflated);
            }
        }
        if !marked {
            commands.entity(entity).insert(NoAutoAabb);
        }
    }
}

/// Registers the shader, the render pipeline, and everything the graph needs to
/// know about sprite materials and frame sequences.
///
/// Design D6 is why this lives here rather than in `sway-nodes`: the material
/// needs its own shader and pipeline, which `sway-runtime` owns.
pub struct SpriteMaterialPlugin;

/// Registers the sprite material's embedded shader and its render pipeline,
/// once.
///
/// Split out of [`SpriteMaterialPlugin`] so the new graph model's
/// [`RuntimeNodesPlugin`](crate::nodes::RuntimeNodesPlugin) can ask for the
/// same pipeline while the two node models sit side by side: adding one Bevy
/// plugin twice panics, and `embedded_asset!` keys the shader by the *source
/// file* it is invoked from, so the call has to stay in this module for
/// [`sprite_material_shader`]'s `embedded_path!` to find it.
pub fn ensure_sprite_material_pipeline(app: &mut App) {
    if app.is_plugin_added::<MaterialPlugin<SpriteMaterialAsset>>() {
        return;
    }
    embedded_asset!(app, "../assets/shaders/sprite_material.wgsl");
    app.add_plugins(MaterialPlugin::<SpriteMaterialAsset>::default());
}

impl Plugin for SpriteMaterialPlugin {
    fn build(&self, app: &mut App) {
        ensure_sprite_material_pipeline(app);
        app.add_systems(
            Update,
            sync_sprite_material_bounds.run_if(resource_exists::<Assets<Mesh>>),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{
        CompareFunction, DepthBiasState, DepthStencilState, RenderPipelineDescriptor, StencilState,
        TextureFormat,
    };

    /// The spec's 30-layer sequence, so the scenarios below read as written.
    const LAYERS: u32 = 30;

    /// Spec: "Fractional frame numbers select one layer". Catches a `round`
    /// where the spec says truncate toward negative infinity — `3.7` would
    /// then land on layer 4 and the sequence would run half a frame ahead.
    #[test]
    fn a_fractional_frame_number_selects_the_layer_below_it() {
        assert_eq!(layer_index(3.7, LAYERS), 3);
    }

    /// Spec: "Frame numbers past the end clamp to the last layer". This is
    /// the test that catches a modulo: `37.5 % 30` is `7`, a plausible-looking
    /// answer that silently imposes looping on every author (design D4).
    #[test]
    fn a_frame_number_past_the_end_holds_on_the_last_layer() {
        assert_eq!(layer_index(37.5, LAYERS), 29);
    }

    /// Spec: "Negative frame numbers clamp to the first layer". Catches both
    /// a wrap (`-1.0` would become layer 29) and, more dangerously, an
    /// unclamped cast — `floor(-1.0) as u32` saturates to 0 by accident,
    /// so this passes for the wrong reason unless the clamp is really there,
    /// which is why the negative-infinity case below exists too.
    #[test]
    fn a_negative_frame_number_holds_on_the_first_layer() {
        assert_eq!(layer_index(-1.0, LAYERS), 0);
    }

    /// The boundary the clamp actually defends: the last valid index is
    /// `layers - 1`, and an off-by-one here samples one layer past the end of
    /// the array — in the vertex stage, where the consequence is geometry,
    /// not a stray pixel.
    #[test]
    fn the_last_valid_frame_number_is_one_below_the_layer_count() {
        assert_eq!(layer_index(29.0, LAYERS), 29);
        assert_eq!(layer_index(30.0, LAYERS), 29);
    }

    /// A single-layer sequence is the tightest range there is, and the one
    /// most likely to break a clamp written as `layers` rather than
    /// `layers - 1`.
    #[test]
    fn a_single_layer_sequence_never_yields_an_index_above_zero() {
        for frame in [-100.0, -0.5, 0.0, 0.9, 1.0, 5.0, 1e30] {
            assert_eq!(layer_index(frame, 1), 0, "frame {frame} on 1 layer");
        }
    }

    /// The degenerate case: an empty sequence has no valid layer at all, so
    /// the contract is only that the function stays total and does not
    /// underflow `layers - 1` into `u32::MAX`. Callers must decline to draw.
    #[test]
    fn an_empty_sequence_yields_zero_rather_than_underflowing() {
        for frame in [-1.0, 0.0, 37.5] {
            assert_eq!(layer_index(frame, 0), 0, "frame {frame} on 0 layers");
        }
    }

    /// Non-finite frame numbers reach here the moment a wire divides by zero
    /// upstream. `f32 as i64` is saturating and NaN-to-zero in Rust, and this
    /// pins that: an index of `u32::MAX` would be an out-of-bounds sample.
    #[test]
    fn non_finite_frame_numbers_stay_inside_the_range() {
        assert_eq!(layer_index(f32::NAN, LAYERS), 0);
        assert_eq!(layer_index(f32::NEG_INFINITY, LAYERS), 0);
        assert_eq!(layer_index(f32::INFINITY, LAYERS), 29);
    }

    /// The whole reason `specialize` exists on this material: Bevy's mesh
    /// pipeline sets `depth_write_enabled = false` for every blended pass, and
    /// without the flip a sprite layer neither occludes nor interpenetrates
    /// anything — it sits wholly in front of or behind, which is the failure
    /// the spec's "Sprite layers occlude and interpenetrate by depth"
    /// requirement forbids.
    #[test]
    fn enable_depth_write_flips_the_flag_bevy_clears_for_blended_passes() {
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

    /// A pipeline with no depth attachment must not panic. Bevy does not build
    /// one for this material today, but `specialize` is handed a descriptor we
    /// do not own, and a helper that unwraps would turn a future pass without
    /// a depth attachment into a crash rather than a no-op.
    #[test]
    fn enable_depth_write_tolerates_a_pipeline_with_no_depth_attachment() {
        let mut descriptor = RenderPipelineDescriptor::default();
        enable_depth_write(&mut descriptor);
        assert!(descriptor.depth_stencil.is_none());
    }
}
