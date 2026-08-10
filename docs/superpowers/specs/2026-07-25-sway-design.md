# Sway — Ongoing work

**Date:** 2026-07-25 (roadmap; architecture extracted 2026-08-09; roadmap
redefined 2026-08-09)
**Status:** In implementation — M5 complete, M6/M7 next
**Architecture:** [`docs/architecture.md`](../../architecture.md) is the
authority on current-state design.
**Rationale:** [`2026-08-09-mvp-roadmap-design.md`](2026-08-09-mvp-roadmap-design.md)
records the decisions behind the milestones below, the surveyed state of the
code, and the full node set. This document is the terse roadmap only.

Completed milestones (M0–M4, wires migration) and their findings live under
`docs/superpowers/reports/` and the historical plans/specs beside them. They are
not repeated here.

## The deliverable

**The target scene is built in the editor, saved, and reopened, without touching
RON.**

Four capabilities define it: create/edit nodes, manipulate the scene with
gizmos and camera navigation, connect nodes with wires, open/save on disk.

One scene must be buildable with them: several layers of animated spritesheets
with depth and alpha, blended by alpha and occluded by z-depth; several 3D mesh
objects with PBR materials whose transforms are animated by wiring; an HDR map
or cubemap driving their lighting.

MIDI transport and events are in scope. The full MIDI/transport node set is not.

## Roadmap

Sizes are relative, not calendar.

### M8-spike — Prove the sprite depth pipeline (S)

`Transparent3d` + `depth_write_enabled` via `Material::specialize` + a
`frag_depth` fragment shader, against a cube. Jumps the queue: it is the only
genuinely unknown item, and a negative result reshapes M8 alone.

*Exit:* a sprite quad visibly interpenetrates a mesh, per-pixel.

### M5 — Minimal scene slice (S/M)

Designed in [`2026-08-10-m5-minimal-scene-slice-design.md`](2026-08-10-m5-minimal-scene-slice-design.md).
Findings in [`2026-08-10-m5-minimal-scene-slice-findings.md`](../reports/2026-08-10-m5-minimal-scene-slice-findings.md).

`MeshAsset`, `PbrMaterial`, `SceneCamera`, `DirectionalLight`, `PointLight`,
plus the `Vec3` / `Math` / `Remap` value nodes and the Vec3 transform wires,
with `#[require]` companions. Deletes `DemoCube`, `setup_scene`, `mesh.rs`,
`scene.rs`; absorbs `material.rs`; migrates the demo off `TranslationYFrom`.

*Exit:* the demo document authors its own camera, light and PBR cube. No
Rust-side scene setup remains.

### M6 — Editor write half (L)

Palette from `ComponentDocRegistry`; create; delete (reparent children first);
reflect-driven inspector editing with wire-driven fields inert; drag-to-connect
and disconnect with legality from the wire registry; Open / Save / Save As.
Extracts `sway-document`.

*Exit:* a node is created, wired, edited, saved and reopened without leaving the
editor.

### M7 — Viewport interaction (L)

Pointer/key forwarding from the shell into Bevy over the viewport rect; an
editor camera distinct from the scene camera with orbit/pan/dolly; click-to-
select via `MeshRayCast`; a TRS gizmo writing `Transform`, with driven axes
inert.

Independent of M6 — may be swapped or run in parallel.

*Exit:* the scene is composed by dragging, not by typing numbers.

### M8 — Visual target (M)

Sprite layers with per-pixel depth and atlas animation (`SpriteLayer`,
`SpriteAnim`); `EnvironmentMap` from pre-baked KTX2 cubemaps.

*Exit:* the scene looks like the intended scene.

### M9 — Events (L)

`sway-events` with generic `TriggerOut<P>` payloads; `MidiNote` and
`BeatTrigger` emitters; `Envelope`; `MidiCc`; event edges in the canvas. Folds
in the MIDI-nodes → `sway-midi` move.

*Exit:* a note drives an envelope drives a material, authored in the editor.

## Refactoring policy

Admitted only when the code is edited anyway, or when the move pushes the
architecture considerably toward its stated shape. Three qualify:
`sway-document` extraction (M6), MIDI → `sway-midi` (M9), and deleting the
orphan helpers in `sway-nodes` (M5).

`sway-geo`, `point_cloud.rs` and `scatter.rs` go **dormant** — left compiling
and reachable through `--demo`, not developed and not cleaned up.

## Out of MVP

- Procedural geometry operators (`Grid`, `Displace`, `Scatter`,
  `CopyToPoints`), geometry intermediate ownership, mesh-identity
  fingerprinting — the target scene uses asset meshes.
- Point clouds and compute scatter.
- The services layer (`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`,
  `AnimationDirector`).
- Show hardening: device hotplug, preflight, output configuration, watchdog,
  black-frame fallback.
- Variadic inlets (`Merge` / `Sum`) — compose binary `Math` instead.
- Restore authored value on disconnect — superseded by D2 (driven fields are
  read-only in the editor).
- GPU-resident geometry operators / compute cook path.
- Live graph patching, presets/snapshots, video decode, audio reactivity (FFT),
  multi-output, NDI/Spout, timeline sequencing.

## Open questions

- **Entity-level sort vertices** can report a false cycle when unrelated
  components on one entity flow in opposite directions. Cycles are already
  allowed (acyclic prefix + append). Richer `(entity, component)` vertices
  remain optional future work — see architecture §4.
- **`MeshRayCast` outside `MeshPickingPlugin`** — unverified whether it needs
  resources only that plugin initialises. Resolve at the top of M7; the fallback
  is hand-rolled ray-vs-AABB, which the gizmo needs regardless.
