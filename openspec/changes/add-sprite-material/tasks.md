## 1. sway-nodes — geometry node and material-wire ownership

- [x] 1.1 Add `plane_mesh.rs` with a `PlaneMesh { size, horizontal, vertical }`
      component, `#[require(Transform, Visibility, Mesh3d, EditorPos)]`, and a
      `Changed<PlaneMesh>` system building the mesh via `PlaneMeshBuilder::new(Dir3::Z, …)`
      — mapping `horizontal`/`vertical` onto the builder's `subdivisions_x`/`subdivisions_z`
      (design D2: the builder's axis letters name a +Y plane and would mislead here).
- [x] 1.2 Unit-test `PlaneMesh`: independent subdivision counts change vertex density on
      the intended axis only, and `(0, 0)` yields four vertices.
- [x] 1.3 Remove `MeshMaterial3d<StandardMaterial>` from `MeshAsset`'s `#[require]` and
      update the doc comment that justifies it (design D5).
- [x] 1.4 Give `MaterialFrom` a relationship hook inserting `MeshMaterial3d<StandardMaterial>`
      on connect and removing it on disconnect, so the field copy still has a target.
- [x] 1.5 Unit-test the hook: connecting supplies the component, disconnecting removes it,
      and a mesh connected to two material kinds in turn carries exactly one at a time.
- [x] 1.6 Register `PlaneMesh` as authorable and update
      `the_plugin_registers_every_authorable_component`.
- [x] 1.7 `cargo test -p sway-nodes`

## 2. sway-runtime — the sprite material

- [x] 2.1 Add `sway-nodes` to `crates/sway-runtime/Cargo.toml` (design D6; mirrors
      `sway-midi`). Confirm the workspace still builds before writing anything against it.
- [x] 2.2 Add `frame_sequence.rs` with a `FrameSequence { folder, color_space }`
      component, `#[require(FrameSequenceOut, EditorPos)]`, and a
      `FrameSequenceOut { texture: Handle<Image>, layers: u32 }` outlet.
- [x] 2.3 Write the pure ordering + validation helpers: sort loaded frame paths
      ascending by filename, and check that every frame shares dimensions and format.
      No GPU, no ECS — this sort is what makes a sequence deterministic (design D7).
- [x] 2.4 Unit-test the ordering helper against a deliberately shuffled input,
      including that `10.png` sorts after `9.png` only if the content is zero-padded —
      document the padding requirement wherever it lands, since it will bite an author.
- [x] 2.5 `FrameSequence` load system: `AssetServer::load_folder` with
      `load_with_settings`-equivalent colour-space handling per D8; hold in a not-ready
      state until every frame has loaded; assemble by concatenating frames vertically
      and calling `Image::reinterpret_stacked_2d_as_array(n)`; report diagnostics for
      mismatched dimensions and for exceeding `device.limits().max_texture_array_layers`.
      Assembly must be idempotent under `file_watcher` reloads.
- [x] 2.6 Add `sprite_material.rs` with the `SpriteMaterial` component
      (`frame`, `tint`, `opacity`, `depth_range`, `depth_pivot` — no paths, no grid),
      `#[require(SpriteMaterialOut, EditorPos)]`, and a
      `SpriteMaterialOut(Handle<SpriteMaterialAsset>)` outlet.
- [x] 2.7 Write the pure `layer_index(frame, layers) -> u32` function: `floor` then
      `clamp` into `[0, layers)` — a safeguard, never a wrap (design D4).
- [x] 2.8 Unit-test `layer_index` against the spec's scenarios: `3.7`→3, `37.5`→29,
      `-1.0`→0 on a 30-layer sequence; and that `layers` of 0 or 1 does not produce an
      out-of-range index.
- [x] 2.9 Add `sprite_material.wgsl`: a standard Bevy mesh material (model matrix,
      `@location(1)` normal, `@location(2)` uv — no billboarding, no `placement` uniform).
      Both runs bind as `texture_2d_array`. Vertex stage samples the depth run's selected
      layer with `textureSampleLevel(…, layer, 0.0)` and displaces along the normal by
      `(sampled - pivot) * range`. Fragment stage samples colour from the same layer index,
      multiplies tint and opacity, discards below the alpha threshold. No `frag_depth`.
- [x] 2.10 Add the shader to `PREPROCESSOR_SHADERS` in `shader_validation.rs`, as
      `sprite_layer.wgsl` already is (it imports Bevy's own view bindings).
- [x] 2.11 Implement `Material::specialize` flipping `depth_stencil.depth_write_enabled`
      to `Some(true)`, split into a testable free function as the spike did, and unit-test
      the flip plus the no-depth-attachment case.
- [x] 2.12 Material sync system: build/update the material asset in place on
      `Changed<SpriteMaterial>`, following `sync_pbr_materials`' handle rules; take both
      textures and the layer count from the wired sequences; report a diagnostic when the
      two runs' lengths differ and bound the frame by the shorter. Differing *resolutions*
      are legal and must not warn.
- [x] 2.13 Insert an explicit `Aabb` inflated by `depth_range` on entities carrying the
      material, so displaced geometry is not culled — not `NoFrustumCulling`.
- [x] 2.14 Add `SpriteMaterialFrom` (with the insert/remove hook from 1.4's pattern),
      `ColorRunFrom`, `DepthRunFrom`, `FrameFrom`, `TintFrom`, `OpacityFrom`; register both
      nodes, their wires and the `MaterialPlugin` from the runtime's own plugin.
- [x] 2.15 `cargo test -p sway-runtime`

## 3. sway-app — content and demo

- [x] 3.1 Author or generate two frame folders under `crates/sway-app/assets/`: a colour
      run and a depth run of equal length, zero-padded filenames, continuous depth where
      alpha is continuous (design, Risks). Author the depth run at roughly the mesh's
      tessellation density rather than the colour run's resolution — it costs far less
      memory and resolves no less relief.
- [x] 3.2 Extend `demo.sway.ron` with a sprite layer: two `FrameSequence` nodes wired
      into a `SpriteMaterial`, itself wired to a `PlaneMesh` quad, with `frame` driven by
      `MidiTime → Oscillator(Saw, period) → Remap(-1..1 → 0..layers)` — the loop is
      built from existing nodes, with no bespoke animation component (design D3, D4).
- [x] 3.3 Place the sprite layer so it interpenetrates an existing demo cube, and add a
      second layer overlapping the first, so both interpenetration cases in the spec are
      reachable by eye.
- [x] 3.4 Register the runtime's sprite plugin in `build_app`.
- [x] 3.5 `cargo test --workspace`

## 4. Verify by eye (architecture §9 — no pixel-diff tests)

- [ ] 4.1 Run windowed and confirm the sequence animates from transport, that swapping
      the oscillator's `Saw` for `Triangle` turns the loop into ping-pong with no material
      edit, and that scrubbing the frame value past the layer count holds on the last
      layer rather than sampling out of range.
- [ ] 4.2 Rotate the quad against the camera with the M7 gizmo and confirm parallax —
      near relief shifting further across the image than far relief. This is the exit
      criterion the whole design turns on (design D1); if it does not appear, stop.
- [ ] 4.3 Confirm sprite-vs-mesh interpenetration, and sprite-vs-sprite interpenetration
      between the two layers — the case the M8 spike explicitly did not test.
- [ ] 4.4 Sweep subdivision counts (0, 31, 63, 255) and record where the relief stops
      improving visibly, to justify the default.
- [ ] 4.5 Confirm no frame-to-frame bleeding: no colour ghosting and no vertex twitching
      toward the next frame's shape. D7 claims array layers make this structurally
      impossible — this check is what confirms the claim rather than assuming it.
- [ ] 4.6 Check for 8-bit banding in the relief and in the interpenetration seam. Record
      the result; the KTX2 `Rgba16Float` upgrade path (design D8) is out of scope here
      but its trigger should be documented.
- [ ] 4.7 Record the memory cost of the demo's two sequences and the resolution the depth
      run was authored at, so the next author has a real number to size against.
- [ ] 4.8 Write findings to `docs/superpowers/reports/`, including whether the skirt
      artefact at depth discontinuities was visible in practice.
