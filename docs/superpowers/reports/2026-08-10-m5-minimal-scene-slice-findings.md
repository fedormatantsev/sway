# M5 — the minimal scene slice: findings

**Date:** 2026-08-10
**Verdict:** GO — exit criterion met
**Plan:** [`2026-08-10-m5-minimal-scene-slice.md`](../plans/2026-08-10-m5-minimal-scene-slice.md)
**Spec:** [`2026-08-10-m5-minimal-scene-slice-design.md`](../specs/2026-08-10-m5-minimal-scene-slice-design.md)
**Roadmap:** M5 in [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)

## Question

Can the demo document author its own camera, light, material and PBR cubes,
with no Rust-side scene setup left anywhere?

## Answer

Yes. `crates/sway-app/assets/demo.sway.ron` now carries all ten entities the
scene needs — `camera`, `sun`, `mat`, `lfoA`, `lfoB`, `vec3A`, `vec3B`,
`cubeA`, `cubeB`, `group` — and `crates/sway-app/src/main.rs` no longer spawns
anything but the window and its plugins. `crates/sway-app/src/scene.rs`,
`demo_assets.rs` and `lib.rs` are gone. `cargo test --workspace`: **269
passed, 0 failed** (1 ignored doctest — `field_wire!`'s usage example,
added with the macro in Task 2, `#[ignore]`d since it needs macro-expansion
context rustdoc doesn't have; unrelated to Tasks 9/10). By eye: two
pale blue cubes bob on Y at different rates, lit from above-right, exactly as
the document specifies.

## What was built

Nine commits, one per task:

- `51d83fd` — Task 1: `apply` keeps `#[require]` companions a document
  doesn't name (see below — the plan's one real discovery).
- `d946cb1` — Task 2: `field_wire!` macro, `assert_writes_only_on_change`,
  `Vec3Value` + its three axis wires.
- `26da633` — Task 3: `TranslationFrom`/`RotationFrom`/`ScaleFrom` replace
  `TranslationYFrom`.
- `2887099` — Task 4: `Math`/`Remap` nodes; `switch_value` deleted.
- `9cd3b85` — Task 5: `MeshAsset` + the checked-in `cube.gltf`; `mesh.rs` and
  the `sway-geo` dependency gone from `sway-nodes`.
- `67af447` — Task 6: `PbrMaterial`/`MaterialOut`/`MaterialFrom`; `material.rs`
  absorbed.
- `5cee568` — Task 7: `SceneCamera` marker; `DirectionalLight`/`PointLight`
  authorable directly.
- `46a7f3a` — Task 8: the ten-entity demo document; `scene.rs`,
  `demo_assets.rs`, `lib.rs` deleted from `sway-app`; `load_project` gated on
  no `--demo`.
- `4034c34` — Task 9: `demo_renders.rs`, the GPU readback test.

Against the exit criterion — "the demo document authors its own camera,
light and PBR cube; no Rust-side scene setup remains anywhere" — every
component that used to be spawned in Rust (`setup_scene`, `DemoAssetsPlugin`,
`DemoCube`) is now a node in the document, and `main.rs`'s only scene-shaped
line left is the `--demo`-gated `load_project` call.

## The `apply` conflict (Task 1) — the plan's one real discovery

Writing the plan surfaced that `apply_components`'s document-removal pass
deliberately stripped any registered-authorable component the document
didn't name — including components an entity only acquired through Bevy's
`#[require]`. Roadmap D4 assumes the opposite: a document names one
component (`"Lfo"`, `"MeshAsset"`) and `#[require]` supplies the rest
(`FloatOut`, `Mesh3d`, `Transform`, `MeshMaterial3d`). Those two behaviours
are in direct conflict — on the very first reload, the removal pass would see
`FloatOut` present and unnamed and delete it, and the node would lose its
outlet.

The fix (Task 1) is narrow: walk `ComponentInfo::required_components()` for
every component the document *did* name on an entity, and exempt that
transitive set from removal. Nothing else changes — a component the document
never named through any chain still goes, which is what
`a_component_no_named_component_requires_is_still_removed` pins down.

**This is the thing M6 most needs to know.** M6's palette is built on exactly
this assumption — click once, get a component, let Bevy fill in the rest —
and without Task 1's fix that flow would silently break on the first
save/reload cycle, not on the first click. Any future authorable component
that leans on `#[require]` gets this exemption for free; nothing about it is
specific to M5's node set.

## Whether the spec's "verify before implementing" facts held

The spec listed three facts, two pre-confirmed and one open:

1. `Mesh3d`/`MeshMaterial3d<M>` are `Default` + `PartialEq` — pre-confirmed,
   held.
2. `Mesh3d` requires `Transform` but not `Visibility` — pre-confirmed, held
   (this is why `MeshAsset` requires `Visibility` explicitly, per
   `require_supplies_everything_the_renderer_needs` in Task 5).
3. **The open one:** whether `#[require]` accepts a generic type argument
   (`MeshMaterial3d<StandardMaterial>`), and whether a default (dangling)
   handle draws nothing rather than panicking. Both held:
   `#[require(Mesh3d, Visibility, Mesh3d, MeshMaterial3d<StandardMaterial>,
   EditorPos)]` compiles as written (`crates/sway-nodes/src/mesh_asset.rs`),
   and the by-eye run never showed a panic or a black/undefined-material
   frame at any point in the load sequence — a mesh with no material handle
   yet simply doesn't draw until `MaterialFrom` propagates.

## By-eye verification

Run: `cargo run -p sway-app -- --editor --windowed`. The app started cleanly
(`no CoreMIDI sources/destinations found` is expected on this machine — MIDI
is out of scope here), reached a steady ~22 fps within two seconds, and threw
no errors or warnings beyond one unrelated `bevy_time` timing notice on the
very first frame.

**What the human verifier confirmed by eye** (an automated screenshot could
not be obtained in this run — see note below):

- Two pale blue cubes, moving as expected — bobbing on Y at different rates
  per their two `Lfo`s — lit from above-right, matching the document's `sun`
  and `mat` (`base_color: (0.6, 0.7, 0.9)`).
- The graph canvas showed 8 of the document's 10 nodes and the wires between
  them (`mat`, `group`, `cubeA`, `cubeB`, `vec3A`, `vec3B`, `lfoA`, `lfoB`).
  `camera` and `sun` were visible in the tree view but **not** on the graph
  canvas, even after panning — see "What M6 and M7 inherit" below; this is
  root-caused, not a loose end.
- `cargo run -p sway-app -- --demo sprite-depth`: the spike's own scene,
  confirmed clean with **no** stray cube — the specific regression Task 8
  closes (the prior spike's findings report noted a `DemoCube` drifting
  through its screenshots because the project document loaded
  unconditionally; that document no longer loads under any `--demo`).

*Screenshot note:* this run's automated attempt to bring the app window
forward and capture it (`osascript`/`screencapture`, following the
sprite-depth spike's precedent) did not reliably raise the window above the
verifier's other applications in this environment, and was abandoned in
favour of direct human verification rather than shipping an unreliable or
misleading screenshot. The by-eye criteria above were confirmed by the human
running the app directly, not inferred from a captured image.

## What M6 and M7 inherit

- **The palette must insert `EditorPos` itself for foreign types.**
  `#[require(EditorPos)]` cannot be added to `DirectionalLight` or
  `PointLight` — they're Bevy's own types — so `crates/sway-nodes/src/lib.rs`
  registers them directly with no `EditorPos` requirement. A light authored
  without an explicit `EditorPos` (as any M6 palette-click would produce
  unless the palette adds one itself) lands on the canvas's fallback grid
  instead of a fixed position.
- **A new one, found in this by-eye pass:** the editor's graph canvas draws
  only entities that appear in `GraphOrder` — that is, only entities that are
  the source or target of a wire propagation, or that own a registered
  behaviour (`crates/sway-editor/src/snapshot.rs:228–279`,
  `capture_nodes`/`graph_entities`). `SceneCamera` and the lights have
  neither: nothing wires into them and nothing wires out, so they never
  appear in `GraphOrder.steps` and are structurally invisible to the graph
  canvas, regardless of whether they carry an `EditorPos`. They *do* show up
  correctly in the tree view, which reads the world directly rather than the
  graph order, and they render correctly — this is a canvas-population gap,
  not a loading or rendering defect. It predates M5 (confirmed:
  `git log 7a9c789..HEAD -- crates/sway-editor` is empty — no task in this
  plan touched `sway-editor`), but M5 is the first time it's user-visible,
  because M5 is the first time a camera or light is a real node in the
  document rather than something Rust spawned. M6 (palette/create) and M7
  (viewport/camera) should both budget for this: a camera or light created
  through the palette will be editable via the inspector and visible in the
  tree, but will not appear on the graph canvas until it's wired to
  something or the canvas's population rule changes to include leaf
  entities with an `EditorPos`.

## Surprises

- The `apply`/`#[require]` conflict (above) was the only genuine discovery —
  everything else in the plan matched its brief closely enough that
  implementer subagents transcribed it with only mechanical compile-fix
  deviations (an extra `use` import here, a borrow-checker-driven `if let`
  there — verified additive-only in every task's review).
- The graph-canvas visibility gap for leaf nodes (no wires, no behaviour)
  was not anticipated by the plan or its spec — both assumed the canvas
  shows "the graph," and camera/light are the first authorable components
  that are genuinely not part of any graph. It cost nothing to characterize
  once found (a single `grep` plus reading `snapshot.rs`), but it would have
  been easy to mistake for a registration bug without checking `GraphOrder`
  first.
- The readback test (Task 9) passed on the first attempt with no debugging
  needed — `2556/16384` cube pixels after 3 updates, comfortably past its
  1%-of-frame threshold, with the plan's documented worst case being 400
  updates on a cold cache.

## Not answered

- Whether M6's palette will actually insert `EditorPos` for foreign types as
  designed — no palette exists yet to check.
- Whether the graph-canvas population gap should be fixed by including
  leaf/`EditorPos`-only entities, or left as "the canvas is the graph, use
  the tree for everything else" — a product decision for M6/M7, not
  something this milestone's scope covers.
- Wires into `PbrMaterial`'s fields and into light intensity (M5-2, deferred
  by design) and everything else already listed as out of scope in the M5
  design spec (`EnvironmentMap`, `SpriteLayer`, `SpriteAnim`, mesh
  primitives, texture maps, camera FOV/clear colour).
