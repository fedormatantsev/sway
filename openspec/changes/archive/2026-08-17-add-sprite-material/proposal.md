## Why

The MVP's target scene needs "several layers of animated spritesheets with depth
and alpha, blended by alpha and occluded by z-depth" (roadmap M8). Nothing in the
tree delivers that: `sprite_layer.rs` is M1 spike code — a billboard quad with one
full-frame texture, no atlas, no depth channel, no graph presence — and
`sprite_depth_spike.rs` is throwaway by design.

The M8 spike returned GO on the only genuinely unknown item (per-pixel depth
interpenetration), so the remaining work is design, not research. The design that
falls out of the spike, however, is not the one the roadmap sketched: the roadmap's
`SpriteLayer` / `SpriteAnim` component pair bundles geometry, material, atlas state
and animation into one component, which contradicts architecture §6 ("materials are
wired, not assigned") and duplicates float animation the graph already does.

## What Changes

- **New `SpriteMaterial` node** — a material node in the established
  `PbrMaterial` shape: authorable component, `SpriteMaterialOut` outlet holding the
  asset handle, and a `SpriteMaterialFrom` wire that hands it to a mesh. It takes a
  colour run and a depth run over wires, plus a `frame` number.
- **New `FrameSequence` node** — loads an ordered run of same-sized images from a
  folder, sorted by filename, and publishes them as one layered (`D2Array`) GPU
  texture. One node type serves both the colour run and the depth run, distinguished
  by an authored colour space. Frames become array *layers* rather than cells packed
  into a grid, so nothing can bleed between them and no cell arithmetic exists.
- **Depth is vertex displacement, not `frag_depth`.** The material's vertex stage
  samples the depth run and displaces along the mesh's own vertex normals. This
  is what makes the relief parallax when the quad is rotated against the camera —
  a `frag_depth` approach preserves screen position by construction and therefore
  cannot parallax. The displacement runs on whatever mesh is wired in, so a depth
  run is a general displacement map, not a sprite-only trick.
- **New `PlaneMesh` node** — a tessellated quad primitive, since the displacement
  needs vertices and `MeshAsset` can only load subdivision counts baked into a file.
  Subdivision counts are named `horizontal` / `vertical`, not `x` / `z`.
- **No `SpriteLayer` and no `SpriteAnim`**, contradicting the roadmap's node table.
  Geometry is a mesh node, the material is a material node, and animation is
  `MidiTime → Oscillator(Saw) → Remap → frame` on the canvas — the same shape
  `demo.sway.ron` already uses to move a cube.
- **BREAKING: `MeshAsset` no longer requires `MeshMaterial3d<StandardMaterial>`.**
  Material wires insert their own typed `MeshMaterial3d<M>` on connect instead.
  Without this an entity wired to a sprite material carries two material components
  and draws twice, once per `MaterialPlugin`. Consequence: a freshly created
  `MeshAsset` does not render until a material is wired, where today it renders with
  Bevy's fallback white.
- `frame` is a `FloatOut` inlet (roadmap D5 keeps scalars scalar). The read side
  clamps it into the wired sequence's range as a **safeguard only** — looping, ping-pong and
  hold-at-end are animation policy and stay in the node network, where they are
  visible and interchangeable. `Oscillator(Saw) → Remap` already expresses a loop
  with no new nodes.
- **`sway-runtime` gains a `sway-nodes` dependency**, mirroring `sway-midi`, so the
  material node can live beside its own shader and pipeline while still using
  `FloatOut` / `Vec3Out`.

Out of scope: `EnvironmentMap` (M8's other half), cross-fading between frames,
16-bit depth runs, streaming sequences longer than device memory allows, and retiring `sprite_layer.rs` / `sprite_depth_spike.rs`.

## Capabilities

### New Capabilities
- `nodes`: authorable scene and value components in `sway-nodes` — here `PlaneMesh`,
  and the change to how `MeshAsset` and material wires divide responsibility for
  `MeshMaterial3d<M>`.
- `runtime`: render-side behaviour in `sway-runtime` — the `SpriteMaterial` node,
  the `FrameSequence` node, their wires, the layer-selection and displacement
  semantics, and the depth-write pipeline configuration the M8 spike proved.

### Modified Capabilities
<!-- None. `document`, `editor` and `graph` requirements are unchanged: the new
     components round-trip and connect through machinery those specs already
     describe. -->

## Impact

- **`sway-nodes`** — new `plane_mesh.rs`; `mesh_asset.rs` loses one `#[require]`
  entry; `pbr_material.rs`'s `MaterialFrom` gains an insert-on-connect hook. The
  palette gains `PlaneMesh`, so `the_plugin_registers_every_authorable_component`
  changes.
- **`sway-runtime`** — new `sprite_material.rs`, `frame_sequence.rs` and
  `sprite_material.wgsl`; new
  `sway-nodes` dependency in `Cargo.toml`. `sprite_layer.rs` and
  `sprite_depth_spike.rs` are left alone as M1/M8-spike regression signals.
- **`sway-app`** — registers the new plugin; demo document gains sprite content;
  colour and depth frame folders land in `crates/sway-app/assets/`.
- **Rendering** — verified by eye per architecture §9, plus pure-function unit tests
  for layer selection, frame ordering, and pipeline-descriptor specialization.
