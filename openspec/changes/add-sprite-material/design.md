## Context

See proposal.md — Why.

Three pieces of prior art constrain this design:

- **`sprite_layer.rs` (M1)** — a billboard quad whose placement comes from a
  uniform rather than `Transform`, with `NoFrustumCulling` to paper over the
  resulting AABB mismatch. Kept as an M1 regression signal; not extended.
- **`sprite_depth_spike.rs` + `sprite_depth_spike.wgsl` (M8 spike, verdict GO)** —
  proved that `Material::specialize` can flip `depth_stencil.depth_write_enabled`
  back on for an alpha-blended pass, and that a `@builtin(frag_depth)` sprite then
  interpenetrates an opaque mesh per-pixel. Throwaway by design.
- **`PbrMaterial` + `MeshAsset` + `MaterialFrom`** — the existing decomposition for
  "a material is a node, wired to a mesh", and the shape this change follows.

Roadmap decisions that bind: **D3** (sprite layers carry a depth channel and
interpenetrate meshes and each other), **D4** (`#[require]` supplies companions,
the palette lists components), **D5** (colours are `Vec3` wires, genuinely scalar
fields stay `FloatOut`).

## Goals / Non-Goals

**Goals:**

- Rotating a sprite quad against the camera shows parallax in the relief.
- The depth channel occludes and interpenetrates meshes and other sprite layers.
- Atlas animation is driven from the graph, using nodes that already exist.
- The material composes with arbitrary geometry, not just a quad.

**Non-Goals:**

- Correct blend *order* where two layers' depth ranges interleave. Depth testing
  will occlude correctly; `Transparent3d` sorts per-entity by centre distance, so
  the blend order between interpenetrating layers can still be wrong. Inherent to
  depth-writing transparency; not solved here.
- Silhouette-accurate relief at grazing angles beyond what the tessellation gives.
- Any pixel-diff test. Rendering is verified by eye (architecture §9).

## Decisions

### D1 — Depth displaces vertices, not `frag_depth`

The spike displaced fragments and wrote `@builtin(frag_depth)`. That is exact for
depth but **cannot produce parallax**: a fragment is rasterized from the flat quad,
so writing a displaced depth changes only what the depth buffer believes, never
where the pixel is drawn. The relief stays locked to the image while the plane
tilts underneath it.

Worse, the two ways to displace a fragment are both wrong for this goal:

- Along the **view ray** — screen position is preserved *by construction*, which is
  what makes the depth exact and simultaneously guarantees zero parallax.
- Along the **normal** — the displaced point projects somewhere other than the pixel
  being shaded, so the depth written is a lie whose error grows as
  `d · tan(angle between normal and view ray)`.

Displacing **vertices** along the mesh's own normals has neither problem: the
rasterizer projects the moved geometry honestly, parallax is real, and the depth
buffer is filled by ordinary rasterization.

*Alternative considered — parallax occlusion mapping.* Flat quad, fragment
ray-marches the heightfield in tangent space. Gives parallax without tessellation,
but costs 8–32 samples per fragment, cannot push relief past the quad's outline, and
has no coherent meaning where alpha is zero — POM assumes an opaque heightfield,
while every asset here is alpha-cut. Rejected on the alpha semantics more than the
cost.

**What this retires from the spike:** the reverse-Z reprojection, the
`clamp(clip.z / clip.w, 0.0, 1.0)` landmine, and the spike findings' open question
about out-of-range depth all disappear — a vertex pushed behind the camera is
ordinary geometry and the rasterizer clips it correctly. Early-Z is also regained.
**What survives:** the `specialize` flip of `depth_write_enabled`, which is still
required for layers to occlude each other, and is the one thing the spike proved.

### D2 — Tessellation density belongs to the mesh, not the material

Because the material displaces along the mesh's normals, the density that governs
relief fidelity is a property of the geometry that was wired in. `PlaneMesh`
carries `horizontal` / `vertical` subdivision counts; the material carries none.

Naming: `PlaneMeshBuilder`'s own fields are `subdivisions_x` / `subdivisions_z`,
named for a plane whose default normal is +Y. This node builds a quad facing +Z, so
those axis letters would name the wrong axes. `horizontal` / `vertical` describe the
quad as authored and as seen in the atlas.

Density is bounded by the *frequency of the depth sheet*, not by screen pixels: the
visible outline of an alpha sprite comes from the alpha channel, which is
per-fragment and already exact, so the grid only has to resolve the relief's shape.
Cost is flat across the useful range — five layers at 63×63 subdivisions is ~41k
triangles, at 255×255 ~655k — so this is a knob to turn by eye, not a budget to
compute. Default 63.

### D3 — No `SpriteLayer`, no `SpriteAnim`

The roadmap's node table lists a `SpriteLayer` component holding the atlas material
and a `SpriteAnim` component advancing its frame. This change contradicts both.

`SpriteLayer` would bundle geometry, material and atlas state into one component,
against architecture §6: *"Materials are wired (one type per material kind), not
assigned — sharing is visible topology."* `PbrMaterial` already demonstrates the
correct decomposition, and following it is what makes "any other mesh" work.

`SpriteAnim` would reimplement float animation the graph already performs.
`demo.sway.ron` moves a cube with `midiTime → lfoA → vec3A → translation`; a frame
counter is the same mechanism with a different endpoint (D4 gives the exact chain).
This also dissolves the roadmap's unanswered "transport or wall time" question —
whichever time source is wired in decides, and the choice is visible on the canvas
rather than buried in a component field.

It also puts *which* animation behaviour you get in the same place: `SpriteAnim`
would have had to grow a loop mode, a ping-pong mode and a hold mode as fields,
each duplicating a waveform the oscillator already has.

### D4 — The read side clamps; the graph owns animation policy

`frame` is `f32` (D5 keeps scalars scalar) and reaches the material either as an
authored field or through a `FrameFrom` wire from any `FloatOut`. Range enforcement
therefore cannot live on the writing side: a node feeding `37.5` into a 30-cell
sheet must land on the same cell as an authored `37.5`.

The material's sync system computes `clamp(floor(frame), 0, layers - 1)`, where
`layers` is the wired sequence's actual layer count rather than an authored number
(D7). Extracted as a pure, GPU-free function (`layer_index(frame, layers) -> u32`)
so it is directly unit-testable, the shape architecture §9 asks for. The shader
stays dumb: it receives an array layer index.

**Clamp, not modulo.** Modulo looks like the obvious choice for a frame counter, but
wrapping *is* looping, and looping is animation policy. Putting it on the read side
would silently impose one playback behaviour and make ping-pong or hold-at-end
unreachable, since the wrap would already have happened before any node could act on
it. Clamping is the minimum needed to guarantee the sheet is never sampled outside
its own cells — a safeguard, with no expressive content. All frame mangling belongs
in the node network.

The graph already has the vocabulary for it, with no new nodes:

```
MidiTime → Oscillator(Saw, period) → Remap(-1..1 → 0..frames) → frame
```

`Waveform::Saw` yields `2.0 * phase - 1.0` and `Remap` already takes explicit input
and output ranges, so a loop is three existing nodes. Swapping `Saw` for `Triangle`
gives ping-pong; `Remap`'s own `clamp` flag gives hold-at-end. Each is a visible
edit on the canvas rather than a hidden convention in the material — which is also
why `MathOp` needs no `Mod` variant.

*Consequence, accepted:* a frame number that runs monotonically past the end holds
on the last cell instead of looping. That is the correct failure for a safeguard —
visibly stuck, rather than quietly doing something the author did not ask for.

No cross-fade between cells. A float inlet invites `fract(frame)` blending, but
under D1 that is a geometry morph as well as a colour blend, and cross-fading
alpha-cut sprites double-exposes wherever the two frames' shapes differ. Snap is
both cheaper and the correct look for spritesheet content.

### D5 — Material wires own their `MeshMaterial3d<M>`

`MeshMaterial3d<M>` is typed per material kind, and `MeshAsset` currently declares
`#[require(..., MeshMaterial3d<StandardMaterial>)]`. Wiring a sprite material onto
such an entity leaves it carrying two material components; both `MaterialPlugin`s
extract it and the mesh draws twice.

`field_wire!` cannot resolve this — it does a reflected field copy onto a component
that must already exist, which is precisely why that `#[require]` is there.

So the `#[require]` is dropped, and each material wire's relationship hook inserts
its own `MeshMaterial3d<M>` on connect. This also fixes a latent inaccuracy: today
every mesh carries a material component before anything is wired, which makes every
mesh look like a legal material consumer.

*Cost, accepted:* a freshly created `MeshAsset` renders nothing until a material is
wired, where today it renders with Bevy's fallback white. M6's drag-to-connect is
what makes the two-material collision reachable in the first place, so the cost
lands with the feature that creates the hazard.

*Alternatives considered.* Having `SpriteMaterialFrom` *remove* the standard
material on connect preserves visible-on-click but is magic and does not restore on
disconnect. Making material wires mutually exclusive in the editor's connect
legality leaves hand-authored RON unprotected.

### D6 — `SpriteMaterial` lives in `sway-runtime`, `PlaneMesh` in `sway-nodes`

The material needs its own shader and pipeline (`sway-runtime` owns those,
architecture §8) *and* `FloatOut` / `Vec3Out` for its inlets (`sway-nodes` owns
those). `sway-nodes` and `sway-runtime` currently have no dependency in either
direction.

`sway-runtime` gains a `sway-nodes` dependency and registers `SpriteMaterial` from
its own plugin. This is exactly the precedent `sway-midi` set: it depends on
`sway-nodes`, declares `MidiTime` with `#[require(FloatOut, EditorPos)]`, and calls
`sway_graph::register_authorable` itself. Acyclic — `sway-nodes` has no runtime
dependency.

`PlaneMesh` needs no shader, so it stays in `sway-nodes` beside `MeshAsset`.

`FrameSequence` (D7) needs no pipeline either — it only assembles `Image` assets —
but it is meaningless apart from the material that consumes it, and its wires target
that material. It goes to `sway-runtime` alongside `SpriteMaterial`.

### D7 — A frame sequence is its own node, and its frames are array layers

Loading is not the material's job. A `FrameSequence` node loads an ordered run of
same-sized images and publishes one GPU texture; `SpriteMaterial` receives it over a
wire. One node type serves both the colour run and the depth run, so the two are
authored, cached and shared identically, and a sequence can feed more than one
material.

**Array layers, not a packed grid.** Bevy offers two routes. `TextureAtlasBuilder`
rect-packs differently-sized images into a single 2D texture — built for mixed
sprite dimensions, and wrong here: it reintroduces cell-rect arithmetic and, worse,
bilinear bleeding between adjacent cells. Under D1 that bleeding is not merely
colour ghosting, because the depth run is sampled in the *vertex* stage: a
neighbouring frame's height would be pulled into the geometry and edge vertices
would twitch toward the next frame's shape.

A `D2Array` texture removes the failure mode rather than mitigating it. Layers have
no neighbours to bleed from and are addressed by an integer index that never
interpolates, so linear filtering — which is wanted, since it interpolates the 8-bit
depth value to float before displacement and smooths the 256-step quantization — is
unconditionally safe. It also deletes the cell-rect arithmetic, and `columns` /
`rows` leave `SpriteMaterial` altogether.

*This supersedes an earlier decision to inset cell UVs by half a texel.* That was a
mitigation for a problem this structure does not have.

Assembly reuses Bevy: concatenate the loaded frames vertically into one buffer,
construct an `Image`, then `Image::reinterpret_stacked_2d_as_array(n)` for the
descriptor work and its validation.

**Frames come from a folder, sorted by filename.** A folder is the natural authoring
unit for a sequence and keeps the node to one path field; `AssetServer::load_folder`
does the work. Ordering is filesystem-dependent, so the loaded set is sorted by path
before assembly — that sort is what makes the sequence deterministic, and it is the
one part of this that must be tested rather than trusted.

*Cost, accepted:* folder listing is a property of the filesystem asset source and
does not survive asset packing or processing. A show build that packs its assets
would need a different enumeration. Out of scope here, and noted as a risk.

**Layer count is derived, not authored.** `SpriteMaterial` no longer carries a frame
count; the clamp of D4 is bounded by the layer count of the sequence actually wired
in. A sequence that fails to load fully cannot be sampled out of range.

*Alternatives considered.* A path pattern with an explicit count (`"smoke/{:03}.png"`,
30) is deterministic without a sort and survives asset packing, but makes the author
maintain a count that the folder already knows, and disagreements between the two
are silent. An explicit list of paths is the most flexible and the most tedious, and
a `Vec<String>` is poor in a reflect-driven inspector.

### D8 — Colour space is per-sequence, and sheets start at 8-bit

Because one node type serves both runs, colour space cannot be inferred from the
node. Depth is data — loading it through an sRGB view would warp the depth mapping —
while colour is colour. So `FrameSequence` carries an explicit colour-space field
and loads its frames via `load_with_settings`, setting `is_srgb` from it.

8-bit gives 256 steps across `depth_range`. Deliberately chosen as the starting
point: under D1 the value is interpolated to float before it displaces a vertex, so
quantization smooths rather than terraces, and the whole content pipeline stays
ordinary PNG. If banding shows, the upgrade path is `Rgba16Float` via KTX2 —
16-bit greyscale PNG is a trap (Bevy maps it to `R16Uint`, which is not filterable
at all) and `R16Unorm` / `Rgba16Unorm` need the non-default wgpu feature
`TEXTURE_FORMAT_16BIT_NORM`.

## Risks / Trade-offs

- **Stretched "skirt" triangles at depth discontinuities** → adjacent vertices
  displace far apart and the connecting triangle becomes a visible sheet. On
  alpha-cut sprites the discontinuity usually coincides with the alpha edge, so the
  skirt is discarded anyway. Mitigation is authorial: keep depth continuous where
  alpha is continuous. No shader machinery built for it.
- **AABB no longer bounds the mesh** → displacement pushes geometry outside the
  authored bounds and Bevy culls on the mesh AABB. An explicit `Aabb` inflated by
  `depth_range` is inserted, rather than inheriting M1's `NoFrustumCulling`.
- **Vertex-stage sampling must use `textureSampleLevel(..., 0.0)`** → `textureSample`
  needs implicit derivatives and is unavailable in a vertex shader. Easy to get
  wrong once; noted here so it is not rediscovered.
- **Blend order between interleaved layers** → see Non-Goals. Constrains how large
  `depth_range` can be relative to layer spacing; an authorial limit, not a bug.
- **`MeshAsset` renders nothing until wired** → see D5. Accepted.
- **Colour and depth sequences must agree in length** → nothing enforces it across
  two separately-loaded sequences. The material reports a diagnostic naming both when
  they disagree, and clamps to the shorter. They need *not* agree in resolution:
  both are sampled with normalized UVs, and per D1/D2 the depth run only feeds vertex
  displacement at tessellation density, so a 64×64 depth frame is Nyquist-matched to
  the default 63 subdivisions while the colour run stays full resolution. That
  decoupling is the main lever on sequence memory — see below.
- **Sequence length is bounded by the device, and by memory well before that** →
  `max_texture_array_layers` is 256 in wgpu's *defaults*, but `sway-gpu` requests
  `adapter.limits()` wholesale (`context.rs`), so the real ceiling is the adapter's —
  typically 2048 on desktop Metal/Vulkan/D3D12. Memory binds first: a 512² RGBA8
  colour frame is 1 MB, so a 256-frame pair with a 64² depth run is ~260 MB and a
  1024-frame pair is ~1 GB. Past roughly a thousand frames this is video, which the
  roadmap puts out of MVP; nothing here streams or pages. An oversized folder is a
  reported error rather than a silent truncation.
- **Folder enumeration does not survive asset packing** → `load_folder` is a
  filesystem-asset-source capability (D7). A show build that packs its assets needs
  a different enumeration; out of scope, and the reason the alternatives in D7 are
  written down rather than discarded.
- **Frames arrive asynchronously** → the array cannot be assembled until every frame
  has loaded, so a sequence has a not-ready state and a material wired to one
  renders nothing until it resolves. Assembly must also be idempotent under Bevy's
  `file_watcher`, which `sway-app` enables.

## Migration Plan

`sprite_layer.rs` and `sprite_depth_spike.rs` are untouched — they remain reachable
through `--demo` as M1 and M8-spike regression signals. Retiring them is deliberately
out of scope, so this change is purely additive on the render side and there is
nothing to roll back beyond removing the new plugin.

The one breaking edit is D5's `#[require]` removal. It affects `MeshAsset` entities
in existing documents: `demo.sway.ron`'s cubes already wire `MaterialFrom`, so the
hook supplies what the `#[require]` used to, and the document needs no edit.

## Open Questions

- Whether `depth_pivot` needs to be authorable per material or can be fixed at 0.5.
  Fixing it later is a field removal, which changes no requirement here.
