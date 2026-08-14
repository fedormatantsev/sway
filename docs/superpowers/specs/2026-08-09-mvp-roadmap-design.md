# Sway — MVP roadmap, redefined

**Date:** 2026-08-09
**Status:** Design approved; replaces M5–M7 as written in
[`2026-07-25-sway-design.md`](2026-07-25-sway-design.md)
**Architecture:** [`docs/architecture.md`](../../architecture.md) remains the
authority on current-state design. This document records the decisions that
change it and the milestones that follow from them.

## Why this exists

The roadmap through M4 pointed at a performance instrument: MIDI in, HDMI out,
exit criterion "a set is played with it". The work that follows is redefined
around a different exit criterion — **the target scene is built in the editor,
saved, and reopened, without touching RON.**

Four capabilities define it:

1. Create and edit nodes in the editor.
2. Manipulate the scene in the 3D preview with gizmos and camera navigation.
3. Connect nodes with wires.
4. Open and save the project on disk.

And one scene must be buildable with them:

- Several layers of animated spritesheets with depth and alpha channels,
  blended by alpha and occluded by z-depth.
- Several 3D mesh objects with PBR materials, transforms animated by wiring.
- An HDR map or cubemap driving the lighting of those objects.

MIDI transport and events stay in scope. The full MIDI/transport node set does
not.

## What the code actually is

Recorded because the roadmap it replaces described intent, not state.

**Live and working.** The graph engine (`Wire` trait, registries, Kahn order,
exclusive tick, rebuild diagnostics, topology watch). Document parse, apply
(reconcile by id) and emit — `to_document(world)` already round-trips. Headless
Bevy rendering into a shared texture composited under masonry. MIDI clock →
`TransportClock` → `Time<Transport>`, which the editor's transport bar reads.

**Registered node set.** One behaviour (`Lfo`), three wires (`AmplitudeFrom`,
`TranslationYFrom`, `ChildOf`), five authorable components (`Lfo`, `FloatOut`,
`Vec3Out`, `Transform`, `EditorPos`). Nothing else. `Vec3Out` is authorable but
no code produces one.

**Unregistered pure logic**, with passing tests, retained through the wire
migration: `math_value` / `remap_value` / `switch_value`, `beat_pulses`,
`envelope_tick` / `adsr_unscaled`, `note_message` / `cc_value`,
`standard_material`, `geometry_to_mesh`, `transform` / `rgb`.

**Read-only editor.** Scene tree, graph canvas (pan, zoom, select, drag boxes),
inspector, transport bar. No write path of any kind. `ViewportPlaceholder` is an
inert hole; nothing forwards pointer events into Bevy.

**Render is M1 spike code.** `sprite_layer.rs` is a billboard quad with one
texture and a hardcoded `(1,1,0,0)` atlas uniform — no depth channel. PBR meshes
reach the scene through a `DemoCube` marker in `sway-app`. Camera and light are
hardcoded in `setup_scene`. No environment map.

**Absent entirely.** Event infrastructure. No `TriggerOut`, no `TriggerIn`, no
`sway-events` crate.

## Decisions

### D1 — Procedural geometry leaves the MVP

The target scene uses asset meshes. `Grid`, `Displace`, `Scatter`,
`CopyToPoints`, geometry-intermediate ownership, and mesh-identity
fingerprinting all leave scope, and with them both of M5's standing open
questions. Point clouds and compute-scatter are not in the scene either.

`sway-geo`, `point_cloud.rs` and `scatter.rs` go **dormant** — left compiling
and reachable through `--demo`, not developed and not cleaned up.

The services layer (`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`,
`AnimationDirector`) has nothing left to serve and is not built. M6's show
hardening moves out of the MVP.

### D2 — Driven fields are read-only in the editor

**Superseded by M6-5** (`2026-08-10-m6-editor-write-half-design.md`): M6 does
not implement this. Every field, driven or not, is editable; the rationale
below was already conceded by this document's own "driven fields churn in the
file... harmless" — the file was never the problem, and the cost of building
detection machinery just to buy inspector/gizmo polish wasn't worth it. Kept
below for the historical record.

Once the editor writes, an authored value and a live value stop being the same
thing: gizmo-dragging a cube whose `translation.y` is LFO-driven, then saving,
would otherwise record whatever phase the LFO was at.

The resolution is the cheapest one that is honest. **The gizmo and the inspector
refuse to edit any field a wire drives**, and such fields render inert. Save
remains `to_document(world)` written to a file. There is no authored-value
shadow and no resident document model.

Accepted consequences, both deliberate:

- **Comments and ordering do not survive a save.** Not wanted.
- **Driven fields churn in the file.** `Transform` is emitted whole, but a wire
  targets a field path (`translation.y`), so a save bakes in the instantaneous
  driven value. Harmless — the first tick after load overwrites it — but `.ron`
  diffs will be noisy. No machinery is built against this.

### D3 — Sprite layers write per-pixel depth

Each frame carries a depth channel; the fragment shader emits
`@builtin(frag_depth)`, so a sprite layer interpenetrates 3D meshes and other
sprite layers per-pixel rather than sitting wholly in front of or behind them.

The pipeline configuration:

- Alpha-blended materials land in Bevy's `Transparent3d` phase, which already
  sorts back-to-front.
- `Material::specialize` receives `&mut RenderPipelineDescriptor`, so
  `depth_stencil.depth_write_enabled` is reachable. Turn it on.
- Meshes draw in the opaque pass first and establish depth. The farthest sprite
  layer draws, blends, and writes its per-pixel depth. Nearer layers depth-test
  against it and blend where they win.
- Fragments within a single quad never overlap, so there is no intra-layer
  ordering hazard.

Cost is losing early-Z, which is irrelevant for a handful of quads. Assets are
**two same-layout atlases**: RGBA colour and R depth.

This is the only genuinely unknown item in the plan, so its spike jumps the
queue. A negative result reshapes M8 and nothing else.

### D4 — The palette lists components; `#[require]` supplies companions

An LFO node is an entity carrying `Lfo`, `FloatOut` and `EditorPos`, but the
architecture says there is no node type and no node instance, so nothing knows
those three belong together.

`Lfo` gains `#[require(FloatOut, EditorPos)]`. The palette lists component types
straight from `ComponentDocRegistry`; clicking one spawns an entity and inserts
that component, and Bevy materialises the rest. No new concept — the ECS stays
the authoring surface, and the dependency is declared next to the type that has
it. Hand-authored RON that forgets `FloatOut` is fixed by the same change.

### D5 — Transform wires are Vec3, not per-axis

`TranslationFrom`, `RotationFrom` (euler, degrees) and `ScaleFrom` take
`Vec3Out`. There are no per-axis transform wires.

`Vec3Out` therefore needs a producer, which is a new **`Vec3 { x, y, z }` value
node** — a vector literal whose components are driveable, with unwired axes
holding their authored value. Not a `Compose` operator: it reads as a value in
the graph, the way TouchDesigner and Houdini both present it.

Colour follows the same rule for consistency: `base_color`, `emissive`, and
sprite `tint` are Vec3 wires. Genuinely scalar fields — `metallic`, `roughness`,
`opacity`, `frame`, `intensity` — stay `FloatOut` wires.

**Cost, accepted:** `TranslationYFrom` is deleted, and with it the demo
document's `"translation.y"` wire and `osc.rs`'s tests that assert on it. Making
a cube bob on Y goes from one edge to a node plus two edges. The cost lands on
the simplest case.

### D6 — Event payloads are generic

```rust
#[derive(Component)]
pub struct TriggerOut<P: EventPayload> { pub events: Vec<(f32, P)> }

pub trait EventWire: Relationship {
    type Payload: EventPayload;
    const NAME: &'static str;
}
```

The per-wire buffer lives on the consumer as `TriggerIn<W>`, inserted by a
component hook that `register_event_wire::<W>` installs on `W`. That registration
also monomorphises the clear and copy functions, so the tick never sees a
generic and the runtime cost matches concrete types.

Ordering per architecture §3, expressed as system sets: `EventPhase::Clear` →
`EventPhase::Copy` → `graph_tick` → `EventPhase::ClearOut`. Each
`register_event_wire::<W>` adds systems to the first two;
`register_event_payload::<P>` adds one to the last.

MVP payloads: `NoteEvent { gate_on: bool, note: NoteMsg }` and `Beat`. The first
matches `envelope_tick`'s existing `&[(f32, bool, NoteMsg)]` closely enough that
the tested ADSR logic re-attaches behind a trivial adapter.

Event wires round-trip through the document like value wires, so they need a
**parallel event-wire registry** alongside `registry_wires.rs`. Architecture §5
already says "the wire registries (value and event)", plural. Drag-to-connect
legality reads both.

This was chosen over two concrete outlet types, which would have been smaller.
The extra cost is type-plumbing in `sway-events`, and it bumps M9 from M to L.

## The node set

### Scene components

All are `Changed<T>` plain systems, per the behaviour table — none depend on a
wired inlet in the same tick, so none are behaviours.

| Component | Produces |
|---|---|
| `MeshAsset` | `Mesh3d` from an asset path |
| `PbrMaterial` | `MeshMaterial3d<StandardMaterial>` |
| `SceneCamera` | `Camera3d` + `RenderTarget` |
| `DirectionalLight`, `PointLight` | Bevy's own types, registered authorable directly |
| `EnvironmentMap` | `EnvironmentMapLight` (+ optional `Skybox`) |
| `SpriteLayer` | the colour+depth atlas material of D3 |
| `SpriteAnim` | advances `SpriteLayer.frame` from `Time<Transport>` |

`SpriteAnim` is deliberately a separate component. Frame advance depends only on
transport — external state — which the behaviour table puts in a plain system.
Add `SpriteAnim` and the layer self-animates; omit it and wire `frame` instead.
There is never a two-writer conflict.

`Transform` and `ChildOf` already exist.

### Value nodes

| Node | Kind | Inlets |
|---|---|---|
| `Vec3 { x, y, z }` → `Vec3Out` | behaviour | `Vec3XFrom`, `Vec3YFrom`, `Vec3ZFrom` |
| `Lfo` → `FloatOut` | behaviour, exists | `AmplitudeFrom` |
| `Math { op, a, b }` → `FloatOut` | behaviour | `MathAFrom`, `MathBFrom` |
| `Remap { in_min, in_max, out_min, out_max, clamp }` → `FloatOut` | behaviour | `RemapInputFrom` |
| `MidiCc { channel, cc }` → `FloatOut` | plain system | none |

No `Const` node. `Math.b` is an authored field a wire may override, so "LFO × 2"
is one `Math` with `b: 2.0` left unwired.

### Event nodes

| Node | Role |
|---|---|
| `MidiNote { channel, note_lo, note_hi }` | emitter → `TriggerOut<NoteEvent>` |
| `BeatTrigger { division }` | emitter → `TriggerOut<Beat>` |
| `Envelope { attack, decay, sustain, release }` | behaviour, `TriggerIn<NoteFrom>` → `FloatOut` |

### Wires

Roughly twenty-two, most of them ten near-identical lines. One
`field_wire!(Name, Source, Target, "path", |t| &mut t.field)` macro generates
them.

- **Vec3Out → target:** `TranslationFrom`, `RotationFrom`, `ScaleFrom`,
  `BaseColorFrom`, `EmissiveFrom`, `TintFrom`
- **FloatOut → target:** `Vec3XFrom`, `Vec3YFrom`, `Vec3ZFrom`, `AmplitudeFrom`,
  `MathAFrom`, `MathBFrom`, `RemapInputFrom`, `MetallicFrom`, `RoughnessFrom`,
  `FrameFrom`, `OpacityFrom`, and three distinct `intensity` wires (directional
  light, point light, environment map — different `Target` types, therefore
  different wire types)
- **Structural:** `ChildOf`
- **Event:** `NoteFrom`, `PulseFrom`

### Deleted

`TranslationYFrom` (D5), `geometry_to_mesh` and `scene.rs`'s `transform`/`rgb`
(dead with D1), `switch_value` (needs a bool outlet; nothing wants one).

Also cut, considered and rejected: `Switch` (no bool outlet exists), curve nodes
and `BeatPhase` (`Lfo` is already beat-locked and covers both).

## Milestones

Sizes are relative, not calendar.

This document is a roadmap spec, not a single implementation plan — the work
spans five loosely-coupled subsystems and will not fit one. **Each milestone
below gets its own plan** under `docs/superpowers/plans/`, written when it is
picked up rather than now. The decisions above (D1–D6) are settled across all
of them; the sequencing within a milestone is not.

### M8-spike — Prove the sprite depth pipeline (S)

Build one `SpriteLayerMaterial` with `depth_write_enabled` and a `frag_depth`
fragment shader, put a cube through it, and screenshot the intersection.

Jumps the queue: a negative result reshapes M8 and nothing else, and it is a
day's work.

*Exit:* a sprite quad visibly interpenetrates a mesh, per-pixel.

### M5 — Minimal scene slice (S/M)

Just enough scene for the editor to have something real to act on.

`MeshAsset`, `PbrMaterial`, `SceneCamera`, `DirectionalLight`, `PointLight`,
plus `Vec3`, `Math`, `Remap` and the Vec3 transform wires. `#[require]`
companions per D4.

`Math` and `Remap` are in despite "minimal": the pure logic and its tests
already exist, wrapping them is about sixty lines, and without them `Lfo` is the
only value source M6's palette has to list.

Deletes `DemoCube`, `setup_scene`, `mesh.rs` and `scene.rs`; absorbs
`material.rs`. Migrates the demo document and `osc.rs`'s tests off
`TranslationYFrom`.

*Exit:* the demo document authors its own camera, light and PBR cube. No
Rust-side scene setup remains anywhere.

### M6 — Editor write half (L)

- Palette from `ComponentDocRegistry`; create; delete, reparenting children
  first (Bevy despawn cascades).
- Reflect-driven inspector editing, with wire-driven fields inert per D2.
- Drag-to-connect and disconnect; legality from the wire registry.
- Open / Save / Save As via `to_document(world)`, with the self-triggered hot
  reload suppressed.
- Extracts `sway-document` from `sway-graph::project` — admitted refactoring:
  this milestone rewrites the save path regardless, and the move satisfies
  architecture §8.

*Exit:* a node is created, wired, edited, saved and reopened without leaving the
editor.

### M7 — Viewport interaction (L)

- Forward pointer and key events from the shell into Bevy when the pointer is
  over the viewport rect. Masonry consumes all of them today.
- An editor camera distinct from the scene camera, with orbit / pan / dolly and
  a toggle between them.
- Click-to-select via `MeshRayCast`, used directly as a `SystemParam`.
  `bevy_picking`'s own pointer input needs `bevy_winit`, which is disabled.
  Selection joins the tree↔canvas sync that already works.
- A translate/rotate/scale gizmo, analytic ray-vs-handle, writing `Transform`.
  Driven axes render inert.

M6 and M7 have no dependency on each other and may be swapped or run in
parallel.

*Exit:* the scene is composed by dragging, not by typing numbers.

### M8 — Visual target (M)

Sprite layers per D3: colour and depth atlas pair, atlas-cell animation from
transport or wall time, graph-authored `SpriteLayer` and `SpriteAnim`. Plus
`EnvironmentMap` — pre-baked KTX2 diffuse and specular cubemaps, Bevy's own
convention, no runtime prefiltering.

Rewrites the demo half of `sprite_layer.rs`.

*Exit:* the scene looks like the scene described at the top of this document.

### M9 — Events (L)

`sway-events` per D6. `MidiNote` and `BeatTrigger` emitters, `Envelope` as the
first consumer, `MidiCc`, event edges rendered in the canvas beside value edges.

Folds in the MIDI-nodes → `sway-midi` move — admitted refactoring: this
milestone edits those files regardless, and the move satisfies architecture §8.

*Exit:* a note drives an envelope drives a material, authored in the editor.

## Refactoring policy

Refactoring is admitted only when the code is being edited anyway, or when it
moves the architecture considerably toward its stated shape. Three qualify:

1. `sway-document` extraction (M6 rewrites the save path).
2. MIDI nodes → `sway-midi` (M9 edits those files).
3. Deleting the three orphan helpers in `sway-nodes`.

Everything else stays put. `sway-geo`, `point_cloud.rs` and `scatter.rs` go
dormant rather than being cleaned up.

## Out of the MVP

Show hardening (device hotplug, preflight, watchdog, black-frame fallback).
Procedural geometry operators and GPU-resident cook paths. The services layer.
Variadic inlets. Restore-authored-value on disconnect — superseded by D2.
Attribute expression and wrangle languages. Live graph patching, presets, video
decode, audio reactivity, multi-output, NDI/Spout, timeline sequencing.

## Open questions

- **False cycles.** Entity-level sort vertices can report a cycle when unrelated
  components on one entity flow in opposite directions. Cycles are already
  allowed (acyclic prefix, then append). Unchanged by this redefinition; see
  architecture §4.
- **`MeshRayCast` outside its plugin.** It is a `SystemParam` in
  `bevy_picking::mesh_picking::ray_cast`, but whether it needs resources that
  only `MeshPickingPlugin` initialises is unverified. Resolve at the top of M7;
  the fallback is hand-rolled ray-vs-AABB, which the gizmo needs anyway.
