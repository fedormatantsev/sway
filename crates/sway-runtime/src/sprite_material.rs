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

use bevy::{
    asset::{AssetEvent, AssetId, AssetPath, embedded_asset, embedded_path},
    camera::{
        primitives::{Aabb, MeshAabb},
        visibility::NoAutoAabb,
    },
    ecs::change_detection::DetectChangesMut,
    ecs::name::Name,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use sway_graph::{EditorPos, ReflectWire, Wire};
use sway_nodes::field_wire;
use sway_nodes::{FloatOut, Vec3Out};

use crate::frame_sequence::{
    ColorSpace, FrameSequence, FrameSequenceOut, changed_id, sync_frame_sequences,
};

/// A sprite sheet as a node. Carries no paths and no grid: the colour and
/// depth runs arrive over wires from `FrameSequence` nodes, and the number of
/// frames is the wired sequence's own layer count rather than an authored
/// number (design D7). That is what makes a sequence that failed to load
/// impossible to sample out of range.
///
/// There is no `columns`/`rows`, no tessellation count and no animation mode
/// here either: cell arithmetic dies with the array texture (D7), density
/// belongs to the mesh that was wired in (D2), and looping, ping-pong and
/// hold-at-end are expressed in the graph rather than as fields (D3, D4).
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(SpriteMaterialOut, SpriteMaterialState, EditorPos)]
pub struct SpriteMaterial {
    /// Which frame of the wired sequences to show. `f32` rather than an
    /// integer because D5 keeps genuinely scalar fields scalar, so any
    /// `FloatOut` can drive it; `layer_index` clamps it on the way to the
    /// GPU.
    pub frame: f32,
    /// A `Vec3` rather than a `Color` for the same reason as
    /// `PbrMaterial::base_color`: roadmap D5 makes every colour inlet a
    /// `Vec3` wire, and the field a wire writes has to be the type the wire
    /// carries. Authored as sRGB — what a human types — so whoever writes the
    /// sync system must linearize it, since the colour run is sampled through
    /// an sRGB view and is already linear where the multiply happens.
    pub tint: Vec3,
    pub opacity: f32,
    /// World units spanned by the full 0..1 depth channel — the same meaning
    /// the spike's `SPIKE_DEPTH_RANGE` had. This is the knob that decides how
    /// far a layer can interpenetrate, and (see design, Risks) how badly two
    /// interleaved layers can blend out of order.
    pub depth_range: f32,
    /// The depth value that leaves a vertex sitting on the undisplaced
    /// surface. Defaults to the midpoint so an 8-bit run can push in both
    /// directions; the design's open question notes this may later be fixed
    /// at 0.5 and the field removed, which changes no requirement.
    pub depth_pivot: f32,
}

impl Default for SpriteMaterial {
    fn default() -> Self {
        Self {
            frame: 0.0,
            tint: Vec3::ONE,
            opacity: 1.0,
            depth_range: 1.0,
            depth_pivot: 0.5,
        }
    }
}

/// The outlet, in the sense of architecture §2: an entity is a sprite-material
/// producer because it has one of these. Not authorable — a handle has no
/// business round-tripping through a document.
#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct SpriteMaterialOut(pub Handle<SpriteMaterialAsset>);

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

// ---------------------------------------------------------------------------
// Wires
// ---------------------------------------------------------------------------

field_wire!(
    /// Hands a sprite material's asset to a mesh. The second real material
    /// wire, and the reason `field_wire!` grew `supplies_target` at all: under
    /// design D5 no mesh node hands out a `MeshMaterial3d<M>` any more, because
    /// that component is typed per material kind and a mesh carrying two kinds
    /// is drawn twice.
    ///
    /// Sourced from `SpriteMaterialOut` rather than from
    /// `MeshMaterial3d<SpriteMaterialAsset>` for the same reason `MaterialFrom`
    /// is: the latter is what a material *consumer* ends up with, so sourcing
    /// from it would make every wired mesh look like a legal producer.
    SpriteMaterialFrom / DrivesSpriteMaterial,
    SpriteMaterialOut => MeshMaterial3d<SpriteMaterialAsset>,
    "0",
    supplies_target
);

field_wire!(
    /// The colour run inlet. **Carries no field copy** — the target path is
    /// empty, exactly as `sway_graph`'s own `impl Wire for ChildOf` is — so
    /// this wire is pure topology and [`sync_sprite_materials`] reads
    /// `FrameSequenceOut` through the relationship instead.
    ///
    /// Why, since every other wire copies a field: `field_wire!` copies outlet
    /// *tuple field 0* onto a named inlet field, and `FrameSequenceOut` is a
    /// named-field struct (`texture`, `layers`) with no field `0`. Delivering
    /// it whole would mean either giving the outlet a tuple shape or adding an
    /// inlet component on the material to copy into — and both mirror `layers`
    /// into a second place, which is precisely what design D7 forbids: the
    /// layer count must be the wired sequence's own, so a sequence that failed
    /// to load cannot be sampled out of range. A copied `layers` would go stale
    /// for exactly one tick after a sequence finishes loading, and that tick is
    /// an out-of-range array sample in the *vertex* stage.
    ///
    /// Reading through the relationship keeps the count derived and costs
    /// nothing else: `source_type`/`target_type` are unchanged, so the editor's
    /// connect-legality rule (`dispatch::connect_is_legal`) is exactly as exact
    /// as for any other wire — the producer must have `FrameSequenceOut` and
    /// the consumer must have `SpriteMaterial`.
    ColorRunFrom / DrivesColorRun,
    FrameSequenceOut => SpriteMaterial,
    ""
);

field_wire!(
    /// The depth run inlet. Same shape and same reasoning as [`ColorRunFrom`];
    /// two wire types rather than one so the two inlets are distinguishable in
    /// the editor and a sequence can feed either.
    DepthRunFrom / DrivesDepthRun,
    FrameSequenceOut => SpriteMaterial,
    ""
);

field_wire!(
    /// The frame number, from any `FloatOut`. Design D4 is why this is a plain
    /// float wire and not an animation node: looping, ping-pong and hold-at-end
    /// are whatever the graph upstream of here does.
    FrameFrom / DrivesFrame,
    FloatOut => SpriteMaterial,
    "frame"
);

field_wire!(
    /// The tint, from any `Vec3Out` — roadmap D5 makes every colour inlet a
    /// `Vec3` wire. Authored sRGB; [`sync_sprite_materials`] linearizes it.
    TintFrom / DrivesTint,
    Vec3Out => SpriteMaterial,
    "tint"
);

field_wire!(
    /// Opacity, from any `FloatOut`. A genuinely scalar field, so it stays a
    /// scalar wire (D5).
    OpacityFrom / DrivesOpacity,
    FloatOut => SpriteMaterial,
    "opacity"
);

// ---------------------------------------------------------------------------
// Material sync
// ---------------------------------------------------------------------------

/// Everything about a published `SpriteMaterialAsset` that can change.
///
/// Compared whole against what the next run would produce, which is what keeps
/// the asset from being rewritten — and therefore re-uploaded — every frame.
#[derive(Debug, Clone, PartialEq)]
struct Published {
    color_texture: Handle<Image>,
    depth_texture: Handle<Image>,
    uniform: SpriteMaterialUniform,
}

/// Per-material sync state.
///
/// Deliberately not `Reflect`, like `FrameSequenceState`: it is invisible to
/// the document and to the inspector, so the authorable surface stays
/// `SpriteMaterial` alone.
#[derive(Component, Default, Debug)]
pub struct SpriteMaterialState {
    /// What was last written into the asset. `None` while the material is
    /// incomplete, so becoming complete always writes.
    published: Option<Published>,
    /// The last diagnostic reported for this material, so a permanent
    /// disagreement between the two runs is logged once rather than once per
    /// frame — the same brake `FrameSequenceState::reported` applies.
    reported: Option<String>,
}

/// What the diagnostic calls the material. `Name` is the document's node id
/// (`sway_document::apply` inserts it), which is what an author would search
/// for; a runtime-spawned material has none and falls back to the entity.
fn material_label(entity: Entity, name: Option<&Name>) -> String {
    match name {
        Some(name) => format!("'{}'", name.as_str()),
        None => format!("on {entity}"),
    }
}

/// Builds and maintains each `SpriteMaterial`'s asset.
///
/// **DEVIATION from the task brief**, which asked for `Changed<SpriteMaterial>`
/// on the query. That filter cannot see the change that matters most: a wired
/// `FrameSequence` publishing its texture some frames after the material was
/// authored, or re-publishing it under `file_watcher`. The material itself did
/// not change, so a `Changed<SpriteMaterial>` system would leave the asset
/// pointing at whatever it had — for a fresh material, nothing at all, forever.
///
/// Watching `Changed<FrameSequenceOut>` on the producer as well would cover
/// that but not a wire being connected or disconnected, which changes neither
/// component. So the guard moved off the query and onto the *result*: the
/// system visits every material each frame, computes what the asset should be,
/// and writes only when that differs from what it last wrote ([`Published`]).
/// One comparison subsumes every source of change, and it is the "never write
/// an equal value" rule the wire model states, applied to an asset.
///
/// **Handle discipline follows `sync_pbr_materials`.** Allocate only when the
/// outlet still holds `Handle::default()`, otherwise mutate in place, so one
/// material wired to two meshes stays one asset and an edit reaches both. The
/// default-handle trap `sync_pbr_materials` describes does not bite for
/// `SpriteMaterialAsset` — `MaterialPlugin` only calls `init_asset::<M>()` and
/// seeds nothing — but it bites hard for the *textures*: `ImagePlugin` seeds a
/// real 1×1 white `Image` at `Handle::default()`, so a material published with
/// an unwired run would render a plain white quad, which is exactly the
/// "renders incorrectly" the spec forbids.
///
/// **An incomplete material renders nothing** by publishing nothing: the outlet
/// is reset to `Handle::default()`, so the mesh's `MeshMaterial3d` resolves to
/// no asset and the material extraction skips the entity. Not a fallback
/// material and not a zero-alpha one — `layer_index(f, 0)` returns 0, which is
/// documented as *not* a valid layer precisely so callers decline to draw
/// rather than sample an empty texture.
#[allow(clippy::type_complexity)] // an ECS query tuple, not a type to simplify
pub fn sync_sprite_materials(
    mut assets: ResMut<Assets<SpriteMaterialAsset>>,
    sequences: Query<&FrameSequenceOut>,
    mut nodes: Query<(
        Entity,
        Option<&Name>,
        &SpriteMaterial,
        Option<&ColorRunFrom>,
        Option<&DepthRunFrom>,
        &mut SpriteMaterialOut,
        &mut SpriteMaterialState,
    )>,
) {
    for (entity, name, node, color_wire, depth_wire, mut out, mut state) in &mut nodes {
        // A sequence with no layers has published nothing — either it has not
        // finished loading or it failed to assemble. Either way there is no
        // texture to sample, so it counts as unavailable rather than as an
        // empty run.
        let run = |wire: Option<&dyn Wire>| -> Option<FrameSequenceOut> {
            let out = sequences.get(wire?.producer()).ok()?;
            (out.layers > 0).then(|| out.clone())
        };
        let color = run(color_wire.map(|wire| wire as &dyn Wire));
        let depth = run(depth_wire.map(|wire| wire as &dyn Wire));

        let (Some(color), Some(depth)) = (color, depth) else {
            // Dropping the handle is what makes "renders nothing" happen: the
            // asset goes with it and the mesh's copy of the handle resolves to
            // nothing on the next propagate.
            out.set_if_neq(SpriteMaterialOut::default());
            state.published = None;
            state.reported = None;
            continue;
        };

        // Spec: "Where the two disagree in length, the system SHALL report a
        // diagnostic and use the shorter." Layer counts only — differing
        // *resolutions* are legal (both runs are addressed by normalized UVs,
        // and per D1/D2 the depth run's useful resolution is bounded by the
        // mesh's tessellation, not by the colour run), so nothing here looks at
        // width or height.
        let layers = color.layers.min(depth.layers);
        if color.layers == depth.layers {
            state.reported = None;
        } else {
            let message = format!(
                "sprite material {} has a {}-layer colour run and a {}-layer depth run; \
                 the frame number is bounded by {layers}",
                material_label(entity, name),
                color.layers,
                depth.layers,
            );
            // `warn!`, where `sync_frame_sequences` uses `error!`: there the
            // diagnostic accompanies publishing nothing, here the material
            // still renders, from the shorter run. The anti-spam discipline is
            // the same — compare against the last message for this node, so a
            // permanent mismatch on an animating material (whose frame number,
            // and therefore whose published uniform, changes every tick) is
            // logged once and not 120 times a second.
            if state.reported.as_deref() != Some(message.as_str()) {
                warn!("{message}");
                state.reported = Some(message);
            }
        }

        // sRGB in, linear out. `tint` is authored the way a human types a
        // colour, like `PbrMaterial::base_color`, but the colour run is sampled
        // through an sRGB view and is therefore already linear where the shader
        // multiplies — so the tint has to be brought into the same space or the
        // multiply is between two different encodings and mid-tones come out
        // visibly wrong. `Color::srgb(..).to_linear()` is the same conversion
        // `to_standard_material` gets for free by handing `Color::srgb` to
        // Bevy. Opacity is not a colour and carries no transfer curve, so it
        // goes into `a` untouched.
        let tint = Color::srgb(node.tint.x, node.tint.y, node.tint.z).to_linear();
        let desired = Published {
            color_texture: color.texture.clone(),
            depth_texture: depth.texture.clone(),
            uniform: SpriteMaterialUniform {
                tint: Vec4::new(tint.red, tint.green, tint.blue, node.opacity),
                // Bounded by the shorter run, which is what makes the spec's
                // "colour 30 / depth 24" case sample layer 23 at most.
                layer: layer_index(node.frame, layers),
                depth_pivot: node.depth_pivot,
                depth_range: node.depth_range,
            },
        };
        if state.published.as_ref() == Some(&desired) {
            continue;
        }

        let asset = SpriteMaterialAsset {
            uniform: desired.uniform,
            color_texture: desired.color_texture.clone(),
            depth_texture: desired.depth_texture.clone(),
        };
        if out.0 == Handle::default() {
            out.set_if_neq(SpriteMaterialOut(assets.add(asset)));
        } else if let Some(mut existing) = assets.get_mut(&out.0) {
            *existing = asset;
        }
        state.published = Some(desired);
    }
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

/// The undisplaced mesh bounds, cached per consumer entity.
///
/// Not for correctness but for cost: `Mesh::compute_aabb` walks every vertex,
/// and a default sprite quad is 63×63 subdivisions (~4k vertices, and D2 puts
/// 255×255 in the useful range). [`sync_sprite_material_bounds`] has to re-run
/// whenever the entity's *scale* changes — every frame, for anything animated —
/// so recomputing from vertices each time would put a mesh walk in the frame
/// budget of every moving sprite. Not `Reflect`, for the same reason
/// `FrameSequenceState` is not: it is derived, not authored.
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
/// needs its own shader and pipeline, which `sway-runtime` owns, *and*
/// `FloatOut`/`Vec3Out` for its inlets, which `sway-nodes` owns. `sway-midi`
/// set the precedent — it depends on `sway-nodes`, declares its own authorable
/// component and calls `sway_graph::register_authorable` itself — and this
/// follows it. `FrameSequence` needs no pipeline, but it is meaningless apart
/// from the material that consumes it and its wires target that material, so it
/// is registered from here too.
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

        // What a project document may name (M4). Short names, not type paths.
        sway_graph::register_authorable::<FrameSequence>(app, "FrameSequence");
        sway_graph::register_authorable::<SpriteMaterial>(app, "SpriteMaterial");
        app.register_type::<ColorSpace>();
        app.register_type::<FrameSequenceOut>();
        app.register_type::<SpriteMaterialOut>();
        app.register_type::<MeshMaterial3d<SpriteMaterialAsset>>();

        sway_graph::register_wire_type::<SpriteMaterialFrom>(app);
        sway_graph::register_wire_type::<ColorRunFrom>(app);
        sway_graph::register_wire_type::<DepthRunFrom>(app);
        sway_graph::register_wire_type::<FrameFrom>(app);
        sway_graph::register_wire_type::<TintFrom>(app);
        sway_graph::register_wire_type::<OpacityFrom>(app);

        app.add_systems(
            Update,
            (
                // Chained, not merely both in `Update`: a sequence that
                // finishes loading should reach the material in the same frame
                // rather than the next one. Correct either way — the material's
                // published-signature guard notices a late change whenever it
                // arrives — but a frame of lag on every asset reload is a
                // needless flicker.
                // Both guards, not just one: the sequence loader needs the
                // asset server to enumerate a folder *and* `Assets<Image>` to
                // publish into, and only `AssetPlugin` supplies the first while
                // only `ImagePlugin` supplies the second. A headless or
                // device-free app can have either without the other.
                sync_frame_sequences.run_if(
                    resource_exists::<AssetServer>.and_then(resource_exists::<Assets<Image>>),
                ),
                sync_sprite_materials.run_if(resource_exists::<Assets<SpriteMaterialAsset>>),
                // After the materials, so a freshly published asset is bounded
                // in the same frame it appears, before `calculate_bounds` runs
                // in `PostUpdate`.
                sync_sprite_material_bounds.run_if(resource_exists::<Assets<Mesh>>),
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::render::render_resource::{
        CompareFunction, DepthBiasState, DepthStencilState, RenderPipelineDescriptor, StencilState,
        TextureFormat,
    };
    use std::any::TypeId;

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

    /// The pivot is what makes an 8-bit run able to push in both directions,
    /// and 0.5 is the value the design's open question proposes fixing it at.
    /// A default of 0.0 would make every sprite bulge toward the camera only.
    #[test]
    fn the_default_pivot_leaves_a_mid_range_depth_value_undisplaced() {
        let material = SpriteMaterial::default();
        assert_eq!(
            (0.5 - material.depth_pivot) * material.depth_range,
            0.0,
            "a mid-range depth value must sit on the undisplaced surface"
        );
        assert_eq!(material.tint, Vec3::ONE, "the default tint must not tint");
        assert_eq!(material.opacity, 1.0);
    }

    // -----------------------------------------------------------------------
    // Material sync
    // -----------------------------------------------------------------------

    /// `AssetPlugin` plus the three asset types the two systems touch — no
    /// device, no renderer, following `frame_sequence.rs`'s pattern.
    ///
    /// The default `Image` handle is seeded because `ImagePlugin` seeds a real
    /// 1×1 white fallback there in every real app. Without it a material
    /// published with an unwired run would look like it published nothing, and
    /// the "renders nothing" tests below would pass for the wrong reason.
    fn sprite_material_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Image>();
        app.init_asset::<Mesh>();
        app.init_asset::<SpriteMaterialAsset>();
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&Handle::default(), Image::default())
            .expect("seeding the default handle succeeds");
        app.register_type::<SpriteMaterial>();
        app.register_type::<SpriteMaterialOut>();
        app.register_type::<FrameSequenceOut>();
        app.register_type::<MeshMaterial3d<SpriteMaterialAsset>>();
        sway_graph::register_wire_type::<SpriteMaterialFrom>(&mut app);
        sway_graph::register_wire_type::<ColorRunFrom>(&mut app);
        sway_graph::register_wire_type::<DepthRunFrom>(&mut app);
        app.add_systems(
            Update,
            (sync_sprite_materials, sync_sprite_material_bounds).chain(),
        );
        app
    }

    /// A published sequence, as `sync_frame_sequences` would leave it. The
    /// width is a parameter so a test can prove that differing *resolutions*
    /// are invisible here.
    fn spawn_sequence(app: &mut App, layers: u32, width: u32) -> Entity {
        let texture = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new(
                bevy::render::render_resource::Extent3d {
                    width,
                    height: width,
                    depth_or_array_layers: layers,
                },
                bevy::render::render_resource::TextureDimension::D2,
                vec![0u8; (width * width * layers * 4) as usize],
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            ));
        app.world_mut()
            .spawn(FrameSequenceOut { texture, layers })
            .id()
    }

    fn spawn_material(
        app: &mut App,
        node: SpriteMaterial,
        color: Option<Entity>,
        depth: Option<Entity>,
    ) -> Entity {
        let entity = app.world_mut().spawn(node).id();
        if let Some(color) = color {
            app.world_mut()
                .entity_mut(entity)
                .insert(ColorRunFrom(color));
        }
        if let Some(depth) = depth {
            app.world_mut()
                .entity_mut(entity)
                .insert(DepthRunFrom(depth));
        }
        entity
    }

    fn published_handle(app: &App, node: Entity) -> Handle<SpriteMaterialAsset> {
        app.world()
            .get::<SpriteMaterialOut>(node)
            .expect("required")
            .0
            .clone()
    }

    fn published_uniform(app: &App, node: Entity) -> SpriteMaterialUniform {
        let handle = published_handle(app, node);
        app.world()
            .resource::<Assets<SpriteMaterialAsset>>()
            .get(&handle)
            .expect("the handle resolves")
            .uniform
    }

    /// Connects a wire and runs its reflected propagate once, standing in for
    /// `sway-nodes`' test-only `wire_testing::propagate_wire`, which is not
    /// reachable from this crate.
    fn propagate_wire<W>(app: &mut App, producer: Entity, consumer: Entity)
    where
        W: Component + From<Entity>,
    {
        app.world_mut()
            .entity_mut(consumer)
            .insert(W::from(producer));
        sway_graph::propagate_reflected(app.world_mut(), producer, consumer, TypeId::of::<W>())
            .expect("the wire propagates");
    }

    #[test]
    fn a_sprite_material_takes_its_layer_count_from_the_wired_sequences() {
        // Design D7: the layer count is derived, never authored — there is no
        // frame-count field to get wrong. Spec's 30-layer case, with the frame
        // number past the end so a bound that came from anywhere else (a
        // hardcoded 256, an authored number, the texture's own descriptor)
        // would produce a different layer.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 30, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial {
                frame: 37.5,
                ..default()
            },
            Some(color),
            Some(depth),
        );

        app.update();

        assert_eq!(published_uniform(&app, node).layer, 29);
    }

    #[test]
    fn disagreeing_run_lengths_are_reported_and_bound_the_frame_by_the_shorter() {
        // Spec scenario "Disagreeing run lengths are reported": colour 30,
        // depth 24, frame past both ends. The failure this catches is bounding
        // by the colour run — the depth run is sampled in the *vertex* stage,
        // so layer 29 of a 24-layer array is out-of-bounds geometry, not a
        // stray pixel.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 24, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial {
                frame: 37.5,
                ..default()
            },
            Some(color),
            Some(depth),
        );

        app.update();

        assert_eq!(published_uniform(&app, node).layer, 23);
        let reported = app
            .world()
            .get::<SpriteMaterialState>(node)
            .expect("required")
            .reported
            .clone()
            .expect("the mismatch is reported");
        assert!(reported.contains("30"), "the diagnostic names both lengths");
        assert!(reported.contains("24"));
    }

    #[test]
    fn a_permanent_length_mismatch_is_reported_once_even_while_the_frame_animates() {
        // The anti-spam property, and the reason the remembered message exists
        // at all: an animating frame number changes the published uniform every
        // tick, so a naive "report whenever we write" would `warn!` at the tick
        // rate for as long as the two folders disagree.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 24, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );

        app.update();
        let first = app
            .world()
            .get::<SpriteMaterialState>(node)
            .expect("required")
            .reported
            .clone();

        app.world_mut()
            .get_mut::<SpriteMaterial>(node)
            .expect("present")
            .frame = 5.0;
        app.update();

        assert_eq!(
            app.world()
                .get::<SpriteMaterialState>(node)
                .expect("required")
                .reported,
            first,
            "the same mismatch must not be re-reported"
        );
        assert_eq!(
            published_uniform(&app, node).layer,
            5,
            "and the material must still be following the frame number"
        );
    }

    #[test]
    fn runs_of_differing_resolution_are_legal_and_report_nothing() {
        // Spec scenario "Runs of differing resolution are legal": 512² colour
        // against 64² depth. Both are addressed by normalized UVs, and per
        // D1/D2 the depth run's useful resolution is bounded by the mesh's
        // tessellation rather than by the colour run — so a mismatch check
        // written against the texture descriptor instead of the layer count
        // would reject the memory saving the design is built around.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 8, 512);
        let depth = spawn_sequence(&mut app, 8, 64);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );

        app.update();

        assert!(
            app.world()
                .get::<SpriteMaterialState>(node)
                .expect("required")
                .reported
                .is_none(),
            "differing resolutions must not warn"
        );
        assert_ne!(published_handle(&app, node), Handle::default());
    }

    #[test]
    fn an_incomplete_material_publishes_no_asset_at_all() {
        // Spec scenario "An incomplete material renders nothing". The trap this
        // catches: publishing an asset whose missing run is `Handle::default()`.
        // `ImagePlugin` seeds a real 1×1 white image there, so that material
        // renders — a plain white quad, undisplaced — which is "renders
        // incorrectly", exactly what the requirement forbids.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let node = spawn_material(&mut app, SpriteMaterial::default(), Some(color), None);

        app.update();

        assert_eq!(
            published_handle(&app, node),
            Handle::default(),
            "no asset is published while a run is missing"
        );
        assert!(
            app.world()
                .resource::<Assets<SpriteMaterialAsset>>()
                .is_empty(),
            "and none was allocated and then abandoned"
        );
    }

    #[test]
    fn a_sequence_that_finishes_loading_reaches_a_material_that_did_not_change() {
        // The change `Changed<SpriteMaterial>` cannot see, and the reason this
        // system's guard is a comparison against what it last published rather
        // than a query filter. Frames arrive asynchronously (design, Risks), so
        // a material is authored complete and its runs turn up several frames
        // later; under a `Changed<SpriteMaterial>` filter the material would
        // never be revisited and would render nothing forever.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = app.world_mut().spawn(FrameSequenceOut::default()).id();
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );

        app.update();
        assert_eq!(
            published_handle(&app, node),
            Handle::default(),
            "the depth run has not resolved yet"
        );

        // The depth sequence assembles, exactly as `sync_frame_sequences`
        // leaves it. Nothing about `SpriteMaterial` changed.
        let arrived = spawn_sequence(&mut app, 30, 4);
        let ready = app
            .world()
            .get::<FrameSequenceOut>(arrived)
            .cloned()
            .expect("required");
        *app.world_mut()
            .get_mut::<FrameSequenceOut>(depth)
            .expect("present") = ready;
        app.update();

        assert_ne!(published_handle(&app, node), Handle::default());
    }

    #[test]
    fn losing_a_run_takes_the_published_asset_away_again() {
        // The other half of "renders nothing": a material that was complete and
        // is disconnected must stop drawing rather than keep the last frame it
        // managed to build, which would look like a working material.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 30, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );
        app.update();
        assert_ne!(published_handle(&app, node), Handle::default());

        app.world_mut().entity_mut(node).remove::<DepthRunFrom>();
        app.update();

        assert_eq!(published_handle(&app, node), Handle::default());
    }

    #[test]
    fn editing_a_sprite_material_mutates_the_asset_in_place() {
        // Handle discipline, copied from `sync_pbr_materials` and load-bearing
        // for the same reason: one material wired to two meshes is one asset,
        // so an edit has to reach both without the handle moving.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 30, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );
        app.update();
        let before = published_handle(&app, node);
        assert_ne!(before, Handle::default(), "an asset was allocated");

        app.world_mut()
            .get_mut::<SpriteMaterial>(node)
            .expect("present")
            .frame = 7.0;
        app.update();

        assert_eq!(
            published_handle(&app, node),
            before,
            "the handle must not change under an edit"
        );
        assert_eq!(published_uniform(&app, node).layer, 7);
    }

    #[test]
    fn a_settled_material_stops_rewriting_its_asset() {
        // A system that wrote unconditionally would mark the asset `Modified`
        // every frame, re-preparing its bind group and re-uploading its uniform
        // for a value that did not move. Scribbling on the asset and finding the
        // scribble intact is the direct way to see that no write happened.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 30, 4);
        let depth = spawn_sequence(&mut app, 30, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );
        app.update();
        let handle = published_handle(&app, node);

        app.world_mut()
            .resource_mut::<Assets<SpriteMaterialAsset>>()
            .get_mut(&handle)
            .expect("present")
            .uniform
            .layer = 7;
        app.update();
        app.update();

        assert_eq!(
            published_uniform(&app, node).layer,
            7,
            "an idle material must not rewrite its asset"
        );
    }

    #[test]
    fn the_authored_tint_reaches_the_uniform_in_linear_space() {
        // `tint` is authored as sRGB, the way a human types a colour, while the
        // colour run is sampled through an sRGB view and is already linear
        // where the shader multiplies. Handing the authored value straight to
        // the uniform would multiply two different encodings, and the error is
        // invisible at 0 and 1 and largest in the mid-tones — hence 0.5, whose
        // linear value is ~0.2140, and hence the endpoints alongside it, which
        // a missing conversion would pass.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial {
                tint: Vec3::new(0.5, 1.0, 0.0),
                opacity: 0.25,
                ..default()
            },
            Some(color),
            Some(depth),
        );

        app.update();

        let uniform = published_uniform(&app, node);
        assert!(
            (uniform.tint.x - 0.2140).abs() < 1e-3,
            "mid-tones must be linearized, got {}",
            uniform.tint.x
        );
        assert!((uniform.tint.y - 1.0).abs() < 1e-6);
        assert!((uniform.tint.z - 0.0).abs() < 1e-6);
        assert_eq!(
            uniform.tint.w, 0.25,
            "opacity is not a colour and carries no transfer curve"
        );
    }

    // -----------------------------------------------------------------------
    // Wires
    // -----------------------------------------------------------------------

    #[test]
    fn connecting_a_sprite_material_wire_supplies_the_component_and_the_handle() {
        // The second `supplies_target` wire, and the reason 1.4 generalized the
        // hook. Under D5 no mesh node hands out a `MeshMaterial3d<M>` any more,
        // so a wire that only copied fields would find no target component, the
        // copy would silently do nothing, and the mesh would never render.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );
        app.update();
        let mesh = app.world_mut().spawn_empty().id();

        propagate_wire::<SpriteMaterialFrom>(&mut app, node, mesh);

        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<SpriteMaterialAsset>>(mesh)
                .map(|material| material.0.clone()),
            Some(published_handle(&app, node)),
            "the hook supplied the component and the field copy filled it"
        );

        app.world_mut()
            .entity_mut(mesh)
            .remove::<SpriteMaterialFrom>();
        assert!(
            app.world()
                .get::<MeshMaterial3d<SpriteMaterialAsset>>(mesh)
                .is_none(),
            "and the wire takes it away again on disconnect"
        );
    }

    #[test]
    fn a_run_wire_copies_no_field_but_still_constrains_what_may_connect() {
        // The shape decision for `ColorRunFrom`/`DepthRunFrom`: an empty target
        // path, like `sway_graph`'s own `impl Wire for ChildOf`, so the wire is
        // pure topology and the sync system reads `FrameSequenceOut` through the
        // relationship — which keeps `layers` derived from the wired sequence
        // (design D7) instead of mirrored onto the material, where it would go
        // stale for a tick every time a sequence finishes loading.
        //
        // What must not be lost in exchange is connect legality, so both halves
        // are asserted here: the wire declares the same source and target types
        // any field-copying wire would, and `connect_is_legal` therefore still
        // refuses a producer that is not a sequence.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );

        let wire = ColorRunFrom(color);
        assert!(wire.target_path().is_empty(), "no field is copied");
        assert_eq!(wire.source_type(), TypeId::of::<FrameSequenceOut>());
        assert_eq!(wire.target_type(), TypeId::of::<SpriteMaterial>());

        let path = <ColorRunFrom as bevy::reflect::TypePath>::type_path();
        assert!(
            sway_graph::dispatch::connect_is_legal(app.world(), path, color, node),
            "a frame sequence may drive a sprite material's colour run"
        );
        assert!(
            !sway_graph::dispatch::connect_is_legal(app.world(), path, node, node),
            "but a sprite material is not a frame sequence producer"
        );
    }

    // -----------------------------------------------------------------------
    // Bounds
    // -----------------------------------------------------------------------

    #[test]
    fn the_displacement_bound_covers_both_ends_of_the_depth_range() {
        // The shader displaces by `(height - pivot) * range` for height in
        // [0, 1] along the mesh's own normals, so the geometry leaves the
        // surface in both directions and the bound is the larger end. Using
        // `range` alone over-inflates at the default pivot; using
        // `(1 - pivot) * range` alone under-inflates for a pivot above 0.5,
        // which is a cull of visible geometry.
        assert_eq!(max_displacement(2.0, 0.5), 1.0);
        assert_eq!(max_displacement(2.0, 0.0), 2.0);
        assert_eq!(max_displacement(2.0, 1.0), 2.0);
        assert_eq!(max_displacement(2.0, 0.25), 1.5);
        assert_eq!(
            max_displacement(-2.0, 0.5),
            1.0,
            "a negative range displaces just as far, the other way"
        );
    }

    /// The world-space extent an `Aabb` contributes along `direction` once the
    /// entity's scale is applied — the same quantity `Aabb::relative_radius`
    /// computes for the frustum test, written out here so the tests below
    /// assert against Bevy's actual culling rule rather than against the
    /// arithmetic they are testing.
    fn world_extent(aabb: &Aabb, direction: Vec3, scale: Vec3) -> f32 {
        (Vec3::from(aabb.half_extents) * scale * direction)
            .abs()
            .element_sum()
    }

    #[test]
    fn inflating_local_bounds_divides_the_world_displacement_by_the_scale() {
        // The unit mismatch this whole function exists for: the shader
        // displaces in *world* units along a normalized world normal, while
        // `Aabb` is *local* and is scaled by the entity's affine before the
        // frustum sees it. Adding `depth_range` straight to `half_extents`
        // passes at scale 1 and under-bounds by the scale factor everywhere
        // else — a quad scaled to 10 would lose 90% of its intended margin and
        // pop out of view while its relief is still on screen.
        let base = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let scale = Vec3::splat(4.0);

        let inflated = inflate_local_aabb(base, 0.5, scale);

        let before = world_extent(&base, Vec3::X, scale);
        let after = world_extent(&inflated, Vec3::X, scale);
        assert!(
            (after - before - 0.5).abs() < 1e-5,
            "the world margin must be the displacement itself, got {}",
            after - before
        );
    }

    #[test]
    fn a_non_uniform_scale_inflates_each_axis_by_its_own_divisor() {
        // A single scalar divisor cannot be right on all three axes: the
        // largest scale under-inflates the thin axes (culling visible relief),
        // the smallest over-inflates the wide ones. Per-axis is exact, and this
        // asserts the world margin on every axis independently.
        let base = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let scale = Vec3::new(0.5, 2.0, 8.0);

        let inflated = inflate_local_aabb(base, 0.25, scale);

        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let margin = world_extent(&inflated, axis, scale) - world_extent(&base, axis, scale);
            assert!(
                (margin - 0.25).abs() < 1e-5,
                "axis {axis:?} got a world margin of {margin}"
            );
        }
        assert_eq!(
            inflated.center, base.center,
            "the inflation is symmetric, so the centre must not move"
        );
    }

    #[test]
    fn a_degenerate_scale_produces_an_enormous_but_finite_bound() {
        // A zero scale flattens the entity's world image along that axis while
        // the displacement stays in world units, so no finite local half-extent
        // bounds it. The divisor is floored rather than allowed to reach zero:
        // an infinite or NaN half-extent makes every frustum comparison false
        // and culls the entity outright, which is the opposite of the safe
        // failure.
        let base = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));

        let inflated = inflate_local_aabb(base, 1.0, Vec3::new(0.0, 1.0, 1.0));

        assert!(Vec3::from(inflated.half_extents).is_finite());
        assert!(inflated.half_extents.x > 1e3, "and conservatively large");
    }

    #[test]
    fn a_non_finite_displacement_leaves_the_bound_finite() {
        // Reachable the moment a wire divides by zero upstream of
        // `depth_range`. Such a material already produces non-finite vertex
        // positions that the rasterizer drops; what must not happen is NaN
        // leaking into the frustum test for every *other* frame's bound.
        let base = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let inflated = inflate_local_aabb(base, bad, Vec3::ONE);
            assert!(
                Vec3::from(inflated.half_extents).is_finite(),
                "displacement {bad} escaped"
            );
        }
    }

    /// A mesh consumer, as a `SpriteMaterialFrom` wire would leave it.
    fn spawn_sprite_mesh(
        app: &mut App,
        material: Handle<SpriteMaterialAsset>,
        scale: f32,
    ) -> Entity {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Mesh::from(Cuboid::new(2.0, 2.0, 2.0)));
        let transform = Transform::from_scale(Vec3::splat(scale));
        app.world_mut()
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                GlobalTransform::from(transform),
            ))
            .id()
    }

    #[test]
    fn a_sprite_mesh_gets_an_inflated_aabb_rather_than_losing_frustum_culling() {
        // Spec scenario "Displaced geometry is not culled", and design, Risks:
        // an explicit inflated `Aabb`, deliberately *not* M1's
        // `NoFrustumCulling` — this material composes with arbitrary geometry,
        // where switching culling off is a cost the author never asked for.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial {
                depth_range: 2.0,
                ..default()
            },
            Some(color),
            Some(depth),
        );
        app.update();
        let material = published_handle(&app, node);
        let mesh = spawn_sprite_mesh(&mut app, material, 1.0);

        app.update();

        let aabb = app.world().get::<Aabb>(mesh).expect("bounds were inserted");
        // A 2×2×2 cuboid has half-extents of 1; the default pivot of 0.5 puts
        // the extreme displacement at half of `depth_range`.
        assert!((aabb.half_extents.x - 2.0).abs() < 1e-5, "{aabb:?}");
        assert!(
            app.world()
                .get::<bevy::camera::visibility::NoFrustumCulling>(mesh)
                .is_none(),
            "culling stays on; only the bounds move"
        );
    }

    #[test]
    fn the_bounds_system_stops_bevy_recomputing_the_aabb_from_the_mesh() {
        // `calculate_bounds` does not merely fill an `Aabb` in — its second
        // query *overwrites* one from the mesh whenever `Mesh3d` or the mesh
        // asset changes, which is exactly when the inflation matters most.
        // `NoAutoAabb` is what excludes this entity from both of its queries;
        // without the marker the inflated bound would be silently replaced by
        // the raw one on the next mesh edit.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial::default(),
            Some(color),
            Some(depth),
        );
        app.update();
        let material = published_handle(&app, node);
        let mesh = spawn_sprite_mesh(&mut app, material, 1.0);

        app.update();

        assert!(app.world().get::<NoAutoAabb>(mesh).is_some());
    }

    #[test]
    fn the_aabb_follows_the_entitys_scale_without_rewalking_the_mesh() {
        // Two properties at once, because they share a cause. The bound has to
        // be recomputed when the entity is scaled (the local margin depends on
        // it), and an animated entity is rescaled constantly — so the
        // undisplaced mesh bounds are cached rather than recomputed from
        // vertices each time. The cache is keyed by the mesh's asset id, and
        // this asserts the cached path still tracks the scale.
        let mut app = sprite_material_app();
        let color = spawn_sequence(&mut app, 4, 4);
        let depth = spawn_sequence(&mut app, 4, 4);
        let node = spawn_material(
            &mut app,
            SpriteMaterial {
                depth_range: 2.0,
                ..default()
            },
            Some(color),
            Some(depth),
        );
        app.update();
        let material = published_handle(&app, node);
        let mesh = spawn_sprite_mesh(&mut app, material, 1.0);
        app.update();
        assert!(app.world().get::<SpriteMeshBounds>(mesh).is_some());

        *app.world_mut()
            .get_mut::<GlobalTransform>(mesh)
            .expect("present") = GlobalTransform::from(Transform::from_scale(Vec3::splat(4.0)));
        app.update();

        let aabb = app.world().get::<Aabb>(mesh).expect("required");
        assert!(
            (aabb.half_extents.x - (1.0 + 1.0 / 4.0)).abs() < 1e-5,
            "the local margin must shrink as the entity grows, got {aabb:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Registration
    // -----------------------------------------------------------------------

    #[test]
    fn the_plugin_registers_every_authorable_component_it_owns() {
        // The equivalent of `sway-nodes`'
        // `the_plugin_registers_every_authorable_component`, and it belongs here
        // for the reason design D6 gives: these two nodes are registered from
        // `sway-runtime` rather than from `sway-nodes`, so `sway-nodes`' list is
        // no longer the whole answer to "what may a document name". A node that
        // exists but was never registered loads as an unknown key.
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            AssetPlugin::default(),
            SpriteMaterialPlugin,
        ));

        let registry = app.world().resource::<sway_graph::ComponentDocRegistry>();
        let mut names: Vec<&str> = registry.entries.iter().map(|entry| entry.name).collect();
        names.sort();
        assert_eq!(names, vec!["FrameSequence", "SpriteMaterial"]);
    }

    #[test]
    fn the_plugin_registers_every_wire_the_two_nodes_use() {
        // A wire that is not in the type registry cannot be dispatched: the
        // tick's `propagate_reflected` looks the type up by id and gives up, so
        // the connection would be visible on the canvas, saved to the document,
        // and silently carry nothing.
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            AssetPlugin::default(),
            SpriteMaterialPlugin,
        ));

        let registry = app.world().resource::<AppTypeRegistry>().read();
        for (name, type_id) in [
            ("SpriteMaterialFrom", TypeId::of::<SpriteMaterialFrom>()),
            ("ColorRunFrom", TypeId::of::<ColorRunFrom>()),
            ("DepthRunFrom", TypeId::of::<DepthRunFrom>()),
            ("FrameFrom", TypeId::of::<FrameFrom>()),
            ("TintFrom", TypeId::of::<TintFrom>()),
            ("OpacityFrom", TypeId::of::<OpacityFrom>()),
        ] {
            assert!(
                registry
                    .get(type_id)
                    .and_then(|registration| registration.data::<ReflectWire>())
                    .is_some(),
                "{name} is not dispatchable"
            );
        }
    }

    #[test]
    fn a_mesh_whose_material_is_incomplete_gets_no_bounds_and_therefore_no_geometry() {
        // The bounds come from the asset, so a material that published nothing
        // leaves nothing to bound — which is consistent, because that mesh is
        // not drawn either. The failure this catches is a system that unwraps
        // the material asset and panics on a run that has not loaded yet, which
        // is the normal state for the first few frames of every show.
        let mut app = sprite_material_app();
        let mesh = spawn_sprite_mesh(&mut app, Handle::default(), 1.0);

        app.update();

        assert!(app.world().get::<Aabb>(mesh).is_none());
    }
}
