# Sway — Design

**Date:** 2026-07-25
**Status:** Approved, pre-implementation

## 1. What this is

An audiovisual instrument for live sets. It listens to MIDI from a hardware setup
(Elektron Octatrack and similar), and drives real-time 3D visuals out over HDMI.
DMX output comes later.

Current scope: **MIDI in, HDMI out.**

### Operating model

During a performance the tool runs unattended. MIDI is the only input; nobody
touches a keyboard. The editor is an authoring tool used before the show, not a
performance surface. This single fact removes a large class of requirements —
no live graph patching, no hot topology mutation, no dropped-frame guarantees on
edits — and the architecture is free to assume the graph is compiled before it
runs.

### Visual target for v1

A 3D scene with custom geometry and custom vertex/fragment shaders. Point clouds
and spritesheet layers with z-depth, animated 3D objects, procedural animation
from curves and optionally physics. The scene reacts to MIDI notes and CC, locked
to the transport.

### Audience

Built for one performer's own sets first, architected so it could be handed to
other VJs later. Project format and node API get real design attention;
onboarding, docs, and distribution are deferred.

## 2. Architecture

Three layers.

**Engine** — graph topology, port typing, compilation, scheduling. Knows nothing
about MIDI, Bevy, or pixels.

**Runtime** — the Bevy app: ECS world, render pipelines, physics, animation
systems, plus a deliberately *exposed service surface*. Runs per-frame whether or
not a graph exists.

**Nodes** — plugin-like units bridging the two.

### 2.1 The central decoupling

The graph *fires*; it does not *evaluate*. It says "burst here", "retarget that
colour", "start clip 3" — and ECS systems own the continuation. An animation
triggered by a node keeps running with no further involvement from the graph.

This is why the runtime stands alone, why the graph can tick at a rate unrelated
to the render loop, and why the graph does not need to be fast.

Corollary principle: the graph is the nervous system, the Bevy world is the body.
Low-cardinality global signals (an LFO, an envelope, a CC) live in the graph.
High-cardinality per-entity state (10k points, rigid bodies, particle lifetimes)
lives in the ECS, parameterised by the graph. **Physics never becomes a node.**

### 2.2 Node contract

```rust
trait NodeType {
    fn schema() -> PortSchema;            // ports, types, defaults, editor metadata
    fn register(app: &mut App);           // once: components, systems, pipelines
    fn instantiate(params: &Params) -> Box<dyn NodeInstance>;
}

trait NodeInstance {
    fn setup(&mut self, world: &mut World, ports: &mut PortView);
    fn tick(&mut self, world: &mut World, ports: &mut PortView, t: TickTime);
    fn teardown(&mut self, world: &mut World);
}
```

**A node type is plugin-shaped; a node instance is not.** `NodeType::register`
runs once at app construction and may install components, systems, and whole
render pipelines. `NodeInstance` is created per graph node — ten LFOs are ten
instances — and carries its own state and lifecycle. This split is what lets a
node ship its ECS systems and shaders alongside its control logic.

`TickTime` carries wall time, transport position in bars/beats, and tick
duration. Nodes derive time-varying values from absolute time rather than
accumulating per tick, so they stay correct across pauses, tempo changes, and
missed ticks.

### 2.3 The exposed runtime surface

Mechanically a node receives `&mut World` and can touch anything. By convention
it goes through registered service resources — for v1 roughly `PointCloudSet`,
`SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`. Each is a small
facade owning its own invariants.

The discipline matters: it is what keeps "a node can touch anything" from
becoming "any node can break anything", and it keeps nodes testable.

### 2.4 Ports and edges

Values are **type-erased at runtime, validated at compile time**. A live set must
never die on a type mismatch, so connection legality is checked when the project
is loaded, never during tick.

A port type registry maps `TypeId` to metadata: display name, default value, and
how the editor should render it.

Two port kinds:

- `Continuous<T>` — always holds a current value.
- `Event<T>` — zero or more occurrences per tick, each with a sub-tick timestamp.

The split is required. Without it there is no way to distinguish "CC is 0" from
"no CC arrived", and note timing collapses to tick granularity. Sub-tick
timestamps let a note landing between ticks start its envelope at the correct
phase.

### 2.5 Compilation

```
project.ron → validate types → topo sort → flat Vec<NodeIdx>
```

All failure happens at load. Tick is infallible.

**Cycles are out of scope.** The compiler rejects them; the graph is a DAG. If
feedback becomes interesting later, a one-tick delay node reintroduces it — edges
are cut at delay nodes as a pre-pass feeding the same topological sort — without
changing the compiler's shape.

### 2.6 Tick model

The graph runs as a single **exclusive system in Bevy's `FixedUpdate`**, at a
fixed rate decoupled from render framerate. Serial evaluation, direct `&mut
World`, trivially ordered.

Consequences, all coherent with the decoupling in 2.1:

- Zero to n ticks per rendered frame. Between ticks the world keeps animating on
  its own, which is exactly the intent.
- Because the tick rate is fixed and independent of rendering performance, a
  recorded MIDI trace replays to bit-identical graph output. Golden-trace testing
  of the entire control layer is exact, not approximate.

MIDI events are timestamped on arrival by the IO thread; the tick drains events
up to the tick boundary, preserving sub-tick offsets.

### 2.7 Transport

The Octatrack sends MIDI clock, and visuals are beat-locked as a first-class
concern. Raw 24 ppqn pulse timing is too jittery to use directly, so the
transport maintains a drift-corrected phase estimate, plus start/stop/continue
handling and tempo tracking. Nodes express time in bars, beats, and 16ths.

### 2.8 Editor and runtime integration

The editor embeds a **live runtime viewport** in one process, sharing a single
wgpu device.

**Structure.** Bevy renders to an offscreen texture in both modes. A thin
presenter decides where the texture goes: composited into a masonry widget when
authoring, blitted to a fullscreen surface on stage. One runtime code path, two
presenters.

**Event loop.** Bevy runs headless in both modes, driven by explicit
`app.update()` calls. The host owns winit and the wgpu device and supplies them
to Bevy via manual render creation rather than letting `RenderPlugin` create its
own. The runtime needs no keyboard or mouse input — it is MIDI-driven — so
forgoing Bevy's winit integration costs nothing. The extra fullscreen blit on
stage is well under a millisecond.

**What one process buys.** Shared memory between editor and engine: no control
socket, no schema manifest, no serialisation boundary. The editor enumerates node
types directly from the registry and reads live port buffers to animate values on
edges.

**The risk, taken deliberately.** Masonry draws through Vello; Bevy drives wgpu
directly. Compositing Bevy's output inside a masonry widget tree requires both to
resolve to the *exact same* wgpu version, and the same winit version to share an
event loop. Bevy and Vello pin these independently and do not move in lockstep.

Mitigations:

- Pin wgpu and winit exactly, once, in the workspace manifest.
- Make duplicate detection a build failure (`cargo tree -d` or `cargo-deny` in
  CI), so a duplicated wgpu is a red build rather than a baffling runtime type
  error.
- Confine all device creation to `sway-gpu`, so a divergence is one file's
  problem.
- Record the known-good `(bevy, vello, wgpu, winit)` tuple and upgrade only when
  the pair realigns.

This assumption is tested at M1b as a go/no-go gate. If it fails, the correct
response is the Syphon route (runtime publishes its frame as a shared GPU
texture, editor consumes it), not months of dependency patching.

**Editor UI is retained-mode**, using masonry. A graph is already a retained
structure with stable identity per node, port, and edge; selection, drag, collapse
and text-cursor state belong in the widget tree rather than being re-derived each
frame. The cost is that there is no node-editor crate to lean on — unlike egui,
where `egui_node_graph` exists as prior art — so pan/zoom transforms, bezier edge
rendering, curve hit-testing, and drag-to-connect are all hand-written. Arbitrary
2D drawing is well within what a Vello-backed widget does, so nothing is blocked.
Masonry is pre-1.0 with a moving API and thin examples; since the editor never
runs on stage, that churn is tolerable.

## 3. Crate layout

```
sway-gpu        wgpu instance/device/queue creation — the single place the
                bevy↔vello version coupling lives
sway-schema     port types, node schema, editor metadata, project format
sway-graph      engine; depends on sway-schema + bevy_ecs
sway-nodes      built-in node types
sway-runtime    headless Bevy app rendering to a texture; services, pipelines
sway-midi       MIDI IO thread + transport clock estimator
sway-editor     masonry UI; links the runtime directly
sway-app        host: owns winit, creates the device, runs editor shell or
                show presenter
```

`sway-graph` depends on `bevy_ecs` only, not full Bevy. Making it generic over a
context type was considered and rejected: a bare `World` is cheap to construct in
tests, so the abstraction would buy nothing real.

## 4. Testing strategy

- **Graph engine** — golden-trace tests. A recorded MIDI trace plus a fixed tick
  rate produces bit-identical output; assert against stored expectations.
- **Transport** — recorded clock traces including tempo changes and dropouts.
- **Compiler** — table-driven tests for type mismatches, cycles, missing nodes,
  unknown types. Every failure mode must produce a clear load-time error.
- **Runtime** — replay recorded MIDI traces through a graph into a headless world;
  assert on ECS state and service calls rather than pixels.
- **Rendering** — no pixel-diff tests. Verified by eye.

## 5. Roadmap

Sizes are relative, not calendar. Ordering follows two rules: get one end-to-end
path working before deepening any layer, and pull genuinely unknown work early.

### M0 — Walking skeleton (S)

MIDI note in → hardcoded Rust graph → a cube changes colour → fullscreen on the
HDMI display. No file format, no editor, no abstraction. Proves the MIDI IO
thread, the `FixedUpdate` tick position in the schedule, and fullscreen output on
an external display.

*Exit:* Octatrack plugged in, something on screen moves in time.

### M1 — Render spike (M, high risk)

Deliberately out of order. Bevy's custom-pipeline API is the least documented,
fastest-moving surface in the project, and both point clouds and z-depth
spritesheets need one. Build them driven by hardcoded parameters, before anything
architectural depends on them.

*Exit:* a point cloud and a z-depth sprite layer render at frame rate with custom
vertex/fragment shaders. The code is provisional — the goal is knowledge, not
architecture.

### M1b — Integration spike (S) — **go/no-go gate**

Headless Bevy rendering to a texture using an externally-created device,
composited by a Vello-backed masonry widget, in one process. Extended with a
pan/zoom canvas holding draggable boxes and bezier edges, to prove masonry can
carry a node editor at all.

*Exit:* one window, one device, Bevy output visible inside a masonry widget.
If bevy and vello cannot currently agree on wgpu and winit, stop and reconsider
against the Syphon route.

### M2 — Graph engine (L)

`sway-graph` core: node trait, port type registry, type-erased ports with
`Continuous`/`Event` kinds, compiler, `FixedUpdate` runner. Initial node set:
MidiNote, MidiCC, LFO, Envelope, Math, Remap, Switch, Select. Golden-trace test
harness. Graphs still constructed in Rust.

*Exit:* a code-built graph drives M1's visuals from real MIDI; trace tests pass
deterministically.

### M3 — Transport and beat lock (M)

MIDI clock ingestion at 24 ppqn, drift-corrected phase estimator,
start/stop/continue, tempo tracking. Transport-aware nodes: tempo-synced LFO,
beat-quantised trigger, bar/beat/16th time base.

*Exit:* visuals stay locked through recorded traces containing tempo changes and
clock dropouts.

### M4 — Project format and hot reload (M)

Versioned RON project files, `load → compile → instantiate`, reloading on file
change. This is what makes authoring possible long before the editor exists, and
it is why the editor can wait.

Constraint: the format is both human- and machine-authored, so it must survive
round-tripping through the editor without destroying comments or ordering. Decide
this here, not at M7.

*Exit:* a set can be authored by editing text with the app running.

### M5 — Visual runtime (L)

The real version of M1. Runtime services (`PointCloudSet`, `SpriteLayers`,
`Emitters`, `CameraRig`, `AnimationDirector`) with owned invariants, glTF mesh
instancing, curve-driven procedural animation, physics if wanted. Where the
fire-and-forget decoupling earns its keep: nodes trigger, ECS systems continue.

*Exit:* a set can be built that actually looks like the intended set.

### M6 — First show (M)

Hardening, not features. MIDI device hotplug and reconnect, a preflight check
validating the project and enumerating displays, output/display configuration, a
watchdog, and a black-frame fallback surviving any single subsystem failure.

*Exit:* a set is played with it.

### M7 — Editor (XL)

Masonry UI in the shape proven at M1b. Schema-driven inspector panel first — the
easy, high-value half — then the canvas: pan/zoom, node widgets, edge routing,
hit-testing, drag-to-connect. Live viewport and live edge values throughout.

*Exit:* authoring without touching RON.

### M8 — DMX (M)

Art-Net/sACN sink nodes, fixture profiles, patch. Architecturally just another
output domain hanging off the same graph — late because it is additive, not
because it is hard.

## 6. Deliberately deferred

Live graph patching. Preset and snapshot recall. Video decode. Audio reactivity
(FFT — likely the first wanted feature not on this list). Multi-output. NDI and
Spout. Timeline sequencing.

## 7. Known open questions

- **State lives in two places** — node-internal and ECS world. Free while the tool
  simply runs; becomes real work if preset recall or snapshot/restore is ever
  wanted. Revisit before M7.
- **Fixed tick rate value** is unchosen. Pick at M2 with measurements rather than
  by guess.
