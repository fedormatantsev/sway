# Sway — M5, the minimal scene slice

**Date:** 2026-08-10
**Status:** Design approved; implementation plan to follow
**Milestone:** M5 in [`2026-08-09-mvp-roadmap-design.md`](2026-08-09-mvp-roadmap-design.md)
**Architecture:** [`docs/architecture.md`](../../architecture.md) is the authority
on current-state design. D1–D6 in the roadmap spec are settled and assumed here.

## The deliverable

The demo document authors its own camera, light, material and PBR cubes. No
Rust-side scene setup remains anywhere.

Enough scene for M6's editor and M7's viewport to have something real to act on:
a mesh that comes from a file, a material that is shared, a camera and a light
that are nodes, and the three value nodes that make wiring worth doing.

## Decisions taken for this milestone

### M5-1 — `MeshAsset` names a path; the repo gains a glTF cube

`MeshAsset { path: String }` loads through `AssetServer`, and M5 checks a small
glTF cube into `crates/sway-app/assets/`. Nothing procedural, no primitive
enum.

The alternative — a `MeshPrimitive { shape, size }` node — needs no checked-in
asset and always renders something when spawned from a palette, but it defers
every asset-loading concern (async handles, load failure, the sub-asset label
syntax, hot reload) to a later milestone that has not budgeted for them. The
target scene uses asset meshes; the loader is the thing worth proving now.

*Accepted cost:* a `MeshAsset` spawned from M6's palette with an empty path
renders nothing until a path is typed.

### M5-2 — M5 builds only the wires the roadmap lists

Nine wires plus the material wire of M5-3. Wires into `PbrMaterial`'s fields and
into light intensity are deferred, even though `field_wire!` makes each about
five lines: the milestone is named "minimal", their targets are authorable
without them, and adding one later is a one-line change.

### M5-3 — A material is its own entity, wired into the mesh

Architecture §6 says materials are wired, not assigned, so that sharing is
visible topology. The roadmap's node table reads the other way — `PbrMaterial`
producing `MeshMaterial3d` on the entity that carries it. The architecture wins.

A material node carries `PbrMaterial`; its `Changed<T>` system writes the
`StandardMaterial` asset and parks the handle in a produced `MaterialOut`
outlet. A `MaterialFrom` wire copies that handle into each consumer's
`MeshMaterial3d<StandardMaterial>`. Two cubes with one look is a visible
fan-out from one node, and editing the material edits both.

Sourcing the wire from `MaterialOut` rather than from `MeshMaterial3d` is what
keeps editor legality exact: every mesh entity carries a `MeshMaterial3d`, so a
wire sourced from that component would make every cube look like a legal
material producer.

*Accepted cost:* one entity and one wire beyond the roadmap's M5 list.

### M5-4 — One headless readback test guards the asset path

Architecture §9 says rendering is verified by eye, and that stands for how the
scene *looks*. It does not cover whether the glTF resolves at all: an asset root
that differs under `cargo test`, a wrong sub-asset label, or a handle that never
finishes loading all produce a world of exactly the right shape and an empty
screen. One test applies the real demo document to a headless app with a real
device and polls `app.update()` until the viewport texture stops being the clear
colour — the shape `sway-runtime/tests/sprite_depth_interpenetration.rs` already
established.

## The node set

Everything below lives in `sway-nodes`, which already depends on the bevy
facade. Each wire lives beside the component it targets, as `AmplitudeFrom` and
`Lfo` already do.

| Component | Doc name | `#[require]` | Produces |
|---|---|---|---|
| `MeshAsset { path: String }` | `MeshAsset` | `Transform`, `Visibility`, `Mesh3d`, `MeshMaterial3d<StandardMaterial>`, `EditorPos` | `Mesh3d(asset_server.load(&path))` on `Changed` |
| `PbrMaterial { base_color: Vec3, emissive: Vec3, metallic: f32, roughness: f32 }` | `PbrMaterial` | `MaterialOut`, `EditorPos` | writes the `StandardMaterial` asset; handle into `MaterialOut` |
| `SceneCamera` (marker) | `SceneCamera` | `Camera3d`, `EditorPos` | nothing further |
| `DirectionalLight`, `PointLight` | same | — (Bevy's own; each already requires `Transform` and `Visibility`) | themselves |
| `Vec3Value { x, y, z }` | `Vec3` | `Vec3Out`, `EditorPos` | behaviour → `Vec3Out` |
| `Math { op, a, b }` | `Math` | `FloatOut`, `EditorPos` | behaviour → `FloatOut` (existing `math_value`) |
| `Remap { input, in_min, in_max, out_min, out_max, clamp }` | `Remap` | `FloatOut`, `EditorPos` | behaviour → `FloatOut` (existing `remap_value`) |
| `Lfo` (exists) | `Lfo` | gains `FloatOut`, `EditorPos` (D4) | unchanged |

`MaterialOut(Handle<StandardMaterial>)` is a produced outlet and is **not**
registered authorable, so handles never round-trip through the document.
`Mesh3d`, `MeshMaterial3d` and `Visibility` are likewise unregistered and
therefore never emitted.

Notes on three of these:

- **`Remap.input`** is a field the roadmap's table omits. `RemapInputFrom` needs
  a target field, exactly as `Math.a` does.
- **`SceneCamera` is a bare marker.** `headless::retarget_cameras` already
  points every camera at the viewport texture each `Update`, so the roadmap's
  "`Camera3d` + `RenderTarget`" needs no code. Field of view and clear colour
  stay at Bevy's defaults; the marker exists so M7 can tell the scene camera
  from the editor camera.
- **Lights are Bevy's own types, registered authorable directly.** Both carry
  `#[reflect(Component, Default)]`, which is what `register_authorable`
  demands. `#[require(EditorPos)]` cannot be added to a foreign type, so a light
  authored without an explicit `EditorPos` lands on the canvas's fallback grid.
  M6's palette will need to insert `EditorPos` itself when spawning a foreign
  component; that is M6's problem, recorded here because M5 is what creates it.

The `Changed<T>` systems for `MeshAsset` and `PbrMaterial` are plain systems —
row 2 of the architecture's behaviour table — guarded with
`run_if(resource_exists::<..>)` so that `sway-nodes`' `MinimalPlugins` test apps
stay safe without an `AssetPlugin`.

## Wires

One `field_wire!(Name, Target-relationship, Source, Target, "path", |t| &mut t.field)`
macro generates the relationship, the relationship target, and a `Wire` impl
whose `propagate` is `map_unchanged(..).set_if_neq(..)`.

| Source | Wires | Target |
|---|---|---|
| `Vec3Out` | `TranslationFrom`, `ScaleFrom` | `Transform` |
| `Vec3Out` | `RotationFrom` — euler degrees → `Quat`, the one custom `propagate` | `Transform` |
| `FloatOut` | `Vec3XFrom`, `Vec3YFrom`, `Vec3ZFrom` | `Vec3Value` |
| `FloatOut` | `MathAFrom`, `MathBFrom` | `Math` |
| `FloatOut` | `RemapInputFrom` | `Remap` |
| `FloatOut` | `AmplitudeFrom` (exists) | `Lfo` |
| `MaterialOut` | `MaterialFrom` | `MeshMaterial3d<StandardMaterial>` |

`RotationFrom` computes the quaternion first and compares that, so an
unchanged euler triple does not dirty `Transform`.

## The demo document

```
lfoA ──amplitude──▶ lfoB
lfoA ──vec3.y────▶ vec3A ──translation──▶ cubeA ─┐
lfoB ──vec3.y────▶ vec3B ──translation──▶ cubeB ─┤──parent──▶ group
mat  ──material──▶ cubeA, cubeB
camera (SceneCamera + Transform)
sun    (DirectionalLight + Transform)
```

The cubes' x offsets move out of `Transform` and into `vec3A.x` / `vec3B.x`,
because `TranslationFrom` writes the whole vector each tick. That is D5's stated
cost, and putting it in the demo makes the `Vec3` node's reason for existing
visible rather than theoretical.

Both cubes share one material entity — the fan-out M5-3 buys.

`#[require]` also lets the document drop its explicit `FloatOut` entries.

## Deletions and the one thing the roadmap missed

Deleted: `sway-app/src/scene.rs` (`setup_scene`), `sway-app/src/demo_assets.rs`
(`DemoCube`), `sway-nodes/src/mesh.rs` (`geometry_to_mesh` — which drops
`sway-geo` from `sway-nodes`' dependencies), `scene.rs`'s `transform` and `rgb`,
`switch_value`, and `spatial.rs`'s `TranslationYFrom`. `material.rs`'s
`standard_material` is absorbed into the new `pbr_material.rs`.

**`load_project` becomes conditional.** It runs unconditionally today, which is
why the sprite-depth spike found a stray `DemoCube` drifting through its
screenshots. Once the document authors its own camera, that stray becomes a
second camera contending for one render target in every `--demo` run. M5 gates
`load_project` on `args.demo.is_none()`.

## Testing

- **Per node** — behaviour output for `Vec3Value`, `Math` and `Remap`; the
  existing one-tick chain test rewritten as `Lfo → Vec3 → Transform`; the
  fan-out test likewise.
- **Per wire** — one generic helper (propagate, `clear_trackers`, propagate
  again, assert `!is_changed`) called for each of the ten wires this milestone
  adds, and for `AmplitudeFrom`, which has no such test today. `ChildOf` is
  exempt: its `propagate` is empty and writes nothing. This is architecture §9's
  per-wire change-detection requirement without eleven hand-written tests.
- **Asset systems** — `PbrMaterial` creates one asset and mutates it in place on
  a later write; `MaterialFrom` lands the same handle on both cubes.
- **`sway-app/tests/demo_document.rs`** — rewritten to the new graph shape.
- **`sway-app/tests/demo_renders.rs`** (new) — M5-4's readback test.
- **By eye** — `cargo run -- --editor`, with a screenshot in the findings
  report.

## Verify before implementing

Three facts the plan must confirm rather than assume. The first two were checked
while writing this document and are recorded as confirmed; the third is open.

1. `Mesh3d` and `MeshMaterial3d<M>` are both `Default` and `PartialEq` in Bevy
   0.19 — **confirmed** (`bevy_mesh::components`, `bevy_pbr::mesh_material`), so
   `#[require]` needs no explicit initialiser and `set_if_neq` works on the
   handle.
2. `Mesh3d` requires only `Transform`, **not** `Visibility` — **confirmed**,
   which is why `demo_assets.rs` inserted `Visibility` by hand and why
   `MeshAsset` must require it.
3. Whether `#[require]` accepts a generic type argument
   (`MeshMaterial3d<StandardMaterial>`), and whether a default (dangling) mesh
   or material handle draws nothing rather than panicking. If the generic form
   is rejected, the `#[require(T = expr)]` initialiser form covers it.

## Out of scope for M5

`EnvironmentMap`, `SpriteLayer` and `SpriteAnim` (M8). Wires into material
fields and light intensity (M5-2). Camera field of view and clear colour.
Mesh primitives. Texture maps on `PbrMaterial`. Everything the roadmap already
puts past the MVP.
