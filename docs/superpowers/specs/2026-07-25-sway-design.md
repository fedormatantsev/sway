# Sway — Ongoing work

**Date:** 2026-07-25 (roadmap; architecture extracted 2026-08-09)
**Status:** In implementation — M5 next
**Architecture:** [`docs/architecture.md`](../../architecture.md) is the
authority on current-state design. This document tracks remaining milestones
and open work only.

Completed milestones (M0–M4, wires migration) and their findings live under
`docs/superpowers/reports/` and the historical plans/specs beside them. They
are not repeated here.

## Roadmap

Sizes are relative, not calendar. Ordering: one end-to-end path before deepening
any layer; pull genuinely unknown work early.

### M5 — Visual runtime (L)

Re-attach the node set the wire migration deleted (pure logic and traces still
exist), then make the scene look like the intended set.

**Node re-attach.** Components, value wires, behaviours, and event emitters for
the non-MIDI set in `sway-nodes`, and the MIDI/transport set in `sway-midi`
(note, CC, envelope consumers, sync LFO, beat trigger, etc.). Each type must
round-trip through `sway-document` as it lands. Event path follows
architecture §3 (`sway-events`, per-wire buffers, pre-tick clear/copy).

**Geometry flow + scene set (CPU).** `Asset`, `Camera`, one component per light
type, `Scatter`, `CopyToPoints`, renderable marker, `Grid` / `Displace` /
`Mesh` as operators. Decide intermediate ownership and cook gating under
`Changed<T>` only — no GPU graph ops in MVP (architecture §6). Close M1's
compute→draw gap only insofar as CPU/graph-authored visuals need it; do not
inherit M1's unamortised per-frame instance-buffer clone unexamined.

**Services.** `PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`,
`AnimationDirector` with owned invariants; glTF instancing; curve-driven
procedural animation; physics if wanted. Fire-and-forget via observers.

**Carry carefully.** Whatever replaces M2b's mesh upload gate must decide a
cheap whole-`Geometry` identity so rewriting `N`/`uv`/indices while passing `P`
through cannot leave a silently stale mesh.

**Deliberately not here.** Attribute expression/wrangle languages. GPU-resident
operators.

**Also land with M5 if not already extracted:** `sway-events` and
`sway-document` as crates per architecture §8; MIDI nodes moved into
`sway-midi`.

*Exit:* a set can be built that actually looks like the intended set.

### M6 — First show (M)

Hardening, not features. MIDI device hotplug and reconnect, preflight
(project validation + display enumeration), output/display configuration,
watchdog, black-frame fallback surviving any single subsystem failure.

*Exit:* a set is played with it.

### M7 — Editor (L)

Write half of the editor. Read half already exists (scene/graph/viewport panes,
live values, read-only inspector).

- Topology editing: drag-to-connect, entity creation from a palette, deletion,
  value editing. Legality from the wire registries (value and event). Render
  `ProjectDiagnostics` beside `GraphDiagnostics`.
- **In-place document writer** in `sway-document`: replace one line per changed
  component/wire so comments and ordering survive; write `EditorPos` back.
- Event-edge activity (per-edge ring buffer) once event wires exist.
- Deleting an entity must reparent children first (Bevy despawn cascades).
- Surface per-node cook time and display flag for geometry debugging.

Show builds still compile topology watches out; stage remains MIDI-only.

*Exit:* authoring without touching RON.

## Out of MVP

- Variadic inlets (`Merge` / `Sum`) — compose binary `Math` / `Switch` instead.
- Restore authored value on disconnect — editor/document policy later.
- GPU-resident geometry operators / compute cook path.
- Live graph patching, presets/snapshots, video decode, audio reactivity (FFT),
  multi-output, NDI/Spout, timeline sequencing.

## Open questions

- **Entity-level sort vertices** can report a false cycle when unrelated
  components on one entity flow in opposite directions. Cycles are already
  allowed (acyclic prefix + append). Richer `(entity, component)` vertices
  remain optional future work — see architecture §4.

M5 may still fork on geometry intermediate ownership and mesh-identity
fingerprinting; those are milestone design decisions, not standing architecture
gaps.
