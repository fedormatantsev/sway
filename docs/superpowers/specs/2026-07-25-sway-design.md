# Sway — Design

**Date:** 2026-07-25
**Status:** Approved, pre-implementation
**Revision:** graph engine builds on Bevy's non-rendering subcrates (§2.2–§2.7, §3)

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
about MIDI or pixels. It does know about Bevy: not the renderer, but the ECS,
reflection, time, and asset subcrates, which it uses as its own substrate rather
than reimplementing them (§2.9).

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
#[derive(Reflect, Component)]
struct LfoParams {
    #[reflect(@Range(0.0..=20.0))]
    hz: f32,
    shape: Waveform,
}

trait NodeType: 'static {
    type Params: Reflect + Component;     // authored values; schema is derived, not written
    type State: Component + Default;      // per-instance runtime state

    fn register(app: &mut App);           // once: reflect registration, components, systems, pipelines
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);
}
```

**A node type is plugin-shaped; a node instance is an entity.** `register` runs
once at app construction and may install components, systems, and whole render
pipelines. This is what lets a node ship its ECS systems and shaders alongside
its control logic.

Ten LFOs are ten entities, each carrying `LfoParams`, `LfoState`, and a port
binding. There is no `NodeInstance` trait object: registration erases
`NodeType::tick` to a bare `fn(&mut World, Entity, &mut PortView, &TickCtx)`
stored in the node type registry, and the tick loop dispatches through it. State
is components, so the editor can inspect it, and snapshot/restore becomes a
world-level concern rather than a per-node protocol (§7).

`setup` and `teardown` are gone. Component hooks on `State` cover instance
lifecycle: `on_add` spawns whatever the node owns in the world, `on_remove`
tears it down. A node deleted in the editor cleans up without the compiler
knowing anything about it.

`TickCtx` carries only what is specific to this tick — its duration and the
sub-tick window events are stamped against. Wall time comes from `Time<Real>`
and beat position from `Time<Transport>` (§2.7). Nodes derive time-varying
values from absolute time rather than accumulating per tick, so they stay
correct across pauses, tempo changes, and missed ticks.

### 2.3 The exposed runtime surface

Mechanically a node receives `&mut World` and can touch anything. By convention
it goes through registered service resources — for v1 roughly `PointCloudSet`,
`SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`. Each is a small
facade owning its own invariants.

The discipline matters: it is what keeps "a node can touch anything" from
becoming "any node can break anything", and it keeps nodes testable.

Where the interaction is genuinely fire-and-forget — "burst here", "start clip
3" — the facade call is an **observer trigger** rather than a method. This is
what observers are for, and it inverts the dependency: a node emits an intent
without linking the system that services it, so a node and the runtime feature
it drives can be developed and tested apart. Observers are used only in this
direction, node → world. They are deliberately **not** used for ports (§2.4).

### 2.4 Ports and edges

Values are **type-erased at runtime, validated at compile time**. A live set must
never die on a type mismatch, so connection legality is checked when the project
is loaded, never during tick.

**The port type registry is `bevy_reflect`'s `TypeRegistry`.** It already maps
`TypeId` to metadata; `TypeData` carries the per-type extras — display name,
`ReflectDefault`, how the editor should render it — and `#[reflect(@...)]` field
attributes carry the per-field ones, declared inline on the params struct. Port
values are `Box<dyn PartialReflect>`, which supplies runtime type identity,
cloning, and comparison without a bespoke value enum.

The consequence worth naming: **a node's schema is derived from its params
struct, not written alongside it.** There is no `schema()` to fall out of sync
with the type it describes.

Port storage is a flat arena, not components. Ports are read and written in
compiled index order by a single system, and the editor reads the whole arena to
animate live values on edges (§2.8) — both are arena-shaped access patterns that
per-entity components would only make slower and more awkward. The arena is a
resource, taken out of the world for the duration of the tick so that a node can
hold `&mut World` and `&mut PortView` at once.

Two port kinds:

- `Continuous<T>` — always holds a current value.
- `Event<T>` — zero or more occurrences per tick, each with a sub-tick timestamp.

The split is required. Without it there is no way to distinguish "CC is 0" from
"no CC arrived", and note timing collapses to tick granularity. Sub-tick
timestamps let a note landing between ticks start its envelope at the correct
phase.

Observers are the wrong tool here despite the surface resemblance to `Event<T>`.
They fire immediately and recursively, which cannot be reconciled with
topologically ordered evaluation, and they carry no notion of buffering several
occurrences with sub-tick offsets to be drained at a known point.

Edges are entities carrying source and target relationship components. Bevy
maintains the reverse index, and despawning a node despawns its edges — which
matters at M7, where the failure mode of a hand-rolled edge list is a dangling
reference after a delete.

### 2.5 Compilation

```
project.ron → spawn node + edge entities → validate types → topo sort
            → flat Vec<Entity> + port arena layout
```

All failure happens at load. Tick is infallible.

**Cycles are out of scope.** The compiler rejects them; the graph is a DAG. If
feedback becomes interesting later, a one-tick delay node reintroduces it — edges
are cut at delay nodes as a pre-pass feeding the same topological sort — without
changing the compiler's shape.

**The topological sort stays ours.** Bevy's `ScheduleGraph` already does a
cached topological sort with cycle detection and would appear to be free, but
using it would mean one system per node *instance*, against the grain of a
scheduler built to order systems per *type* — and its errors would name systems
rather than nodes, in direct conflict with §4's requirement that every load
failure produce a clear, node-attributed message. A topological sort is a few
dozen lines with error messages we control.

### 2.6 Tick model

The graph runs as a single **exclusive system in Bevy's `FixedUpdate`**, at a
fixed rate decoupled from render framerate. Serial evaluation, direct `&mut
World`, trivially ordered. `Time<Fixed>` owns the accumulator and the rate
(`Time::<Fixed>::from_hz`), and its clamped catch-up behaviour is exactly the
0..n-ticks-per-frame model below.

**Nodes are not per-type systems, and this is the one place the ECS is refused.**
The batched version of this design is seductive — an LFO node type as a system
over `Query<(&LfoParams, &mut LfoState)>`, no dispatch, good cache behaviour. It
does not survive ordering. Bevy schedules *systems*; the topological order is
over *instances*. A graph containing `LFO → Math → LFO` has no expression as
system ordering constraints. The escapes are one-tick latency on the edges that
run backwards against system order, or running the whole system set once per DAG
level. The first is still deterministic but no longer dataflow: a node reads its
input from the previous tick, and which edges are affected depends on system
registration order rather than on anything the author can see — the resulting
timing bugs would be invisible in the graph and reproducible only by accident.
The second pays a full schedule traversal per level to recover what a flat loop
already had. Serial dispatch over a compiled `Vec<Entity>` is correct, is fast
enough for a control graph of this cardinality, and keeps the semantics the
author expects when they draw an edge.

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

`Time<T>` is generic over a clock, so **the transport is a clock**:
`Time<Transport>`, whose elapsed time is measured in beats and whose advance per
tick is whatever the phase estimator says. A tempo-synced node takes
`Res<Time<Transport>>` and is otherwise an ordinary node; stop is a clock that
stops advancing. This is why `TickCtx` stays small — it carries what is specific
to the tick, and the clocks carry time.

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
socket, no schema manifest, no serialisation boundary. The editor walks
`TypeRegistry` directly for node types and their field metadata, and reads the
live port arena to animate values on edges. The inspector is a function of the
same type information the runtime uses, so there is no second description of a
node to keep in sync.

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

### 2.9 What Bevy owns

The dependency is on Bevy's **non-rendering subcrates**, named individually.
`sway-graph` must not pull `bevy_render`; the renderer belongs to `sway-runtime`
and nothing in the engine layer should be unbuildable headless.

| Concern | Owner |
|---|---|
| Node/edge storage, lifecycle, cascade delete | `bevy_ecs` — entities, hooks, relationships |
| Port type registry, editor metadata, schema | `bevy_reflect` — `TypeRegistry`, `TypeData`, field attributes |
| Type-erased port values | `bevy_reflect` — `Box<dyn PartialReflect>` |
| Params (de)serialisation | `bevy_reflect` serde |
| Tick rate, accumulator, catch-up | `bevy_time` — `Time<Fixed>` |
| Beat time, pause, tempo scaling | `bevy_time` — `Time<Transport>` |
| Project loading, file watching, hot reload | `bevy_asset` — `AssetLoader`, `AssetEvent` |
| Registration surface, schedule placement | `bevy_app` |
| Type validation, topological sort, error reporting | **ours** |
| Port arena and its compiled layout | **ours** |
| Serial tick dispatch over the compiled order | **ours** |
| Transport phase estimation from 24 ppqn | **ours** |

The line is consistent: Bevy owns storage, identity, metadata, and time; we own
ordering, typing, and the error messages a performer sees at load.

The earlier framing of this boundary — "`sway-graph` depends on `bevy_ecs` only"
— was already untrue in this document, since the node contract took `&mut App`
and the tick lived in `FixedUpdate`, both `bevy_app`. Drawing the line at
"everything except the renderer" is both honest and considerably more useful.

The cost is that a Bevy upgrade now touches the engine layer rather than only
the runtime. Given §2.8 already pins the Bevy version exactly to hold the
wgpu/winit alignment, this adds coordination but no new class of risk. The
testing argument in the original §3 survives intact: golden-trace tests build a
minimal `App` with no rendering, which is as cheap as building a bare `World`.

## 3. Crate layout

```
sway-gpu        wgpu instance/device/queue creation — the single place the
                bevy↔vello version coupling lives
sway-graph      engine: port kinds, node type registry, compiler, port arena,
                tick runner, project format
sway-nodes      built-in node types
sway-runtime    headless Bevy app rendering to a texture; services, pipelines
sway-midi       MIDI IO thread + transport clock estimator
sway-editor     masonry UI; links the runtime directly
sway-app        host: owns winit, creates the device, runs editor shell or
                show presenter
```

**`sway-schema` is gone.** It was to hold port types, node schema, editor
metadata, and the project format. `bevy_reflect` supplies the first three, and
what remains — port kinds, the node type registry, the document shape — is small
enough that a separate crate would exist only to preserve a boundary nothing
needs. The editor links `sway-graph` regardless.

`sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_reflect`, `bevy_time`, and
`bevy_asset` — not on `bevy`, and specifically not on `bevy_render`. Making the
engine generic over a context type was considered and rejected: a minimal
headless `App` is cheap to construct in tests, so the abstraction would buy
nothing real.

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

`sway-graph` core: `NodeType` trait with reflect-derived params, node and edge
entities, type-erased ports with `Continuous`/`Event` kinds, compiler,
`FixedUpdate` runner. Initial node set: MidiNote, MidiCC, LFO, Envelope, Math,
Remap, Switch, Select. Golden-trace test harness. Graphs still constructed in
Rust.

The reflect-derived schema is load-bearing for M7 and should be exercised here:
one node type's params should already drive a throwaway debug inspector, so that
missing `TypeData` is found now rather than at the start of an XL milestone.

*Exit:* a code-built graph drives M1's visuals from real MIDI; trace tests pass
deterministically.

### M3 — Transport and beat lock (M)

MIDI clock ingestion at 24 ppqn, drift-corrected phase estimator,
start/stop/continue, tempo tracking. Transport-aware nodes: tempo-synced LFO,
beat-quantised trigger, bar/beat/16th time base.

*Exit:* visuals stay locked through recorded traces containing tempo changes and
clock dropouts.

### M4 — Project format and hot reload (M)

Versioned RON project files loaded as a `bevy_asset` `Asset` with a custom
`AssetLoader`; `AssetEvent::Modified` triggers recompile. This is what makes
authoring possible long before the editor exists, and it is why the editor can
wait. File watching, debounce, and the write-then-rename behaviour of real text
editors come from `AssetServer` rather than a hand-rolled watcher.

Constraint: the format is both human- and machine-authored, so it must survive
round-tripping through the editor without destroying comments or ordering. Decide
this here, not at M7. **Reflect does not solve this** — `ReflectSerializer`
output is verbose and comment-destroying, and RON does not preserve comments on
round-trip either. The expected shape is reflect for *reading* and a
hand-controlled emitter for *writing*, editing the existing document in place
rather than regenerating it.

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
easy, high-value half, and easier still because it is a walk over `TypeRegistry`
and `TypeData` rather than a bespoke schema format — then the canvas: pan/zoom,
node widgets, edge routing, hit-testing, drag-to-connect. Live viewport and live
edge values throughout.

Topology editing spawns and despawns node and edge entities and requests a
recompile. Nothing here weakens §1's guarantee that the graph is compiled before
it runs: during a show there is no editor.

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

- **Fixed tick rate value** is unchosen. The mechanism is settled
  (`Time::<Fixed>::from_hz`); the number should be picked at M2 with
  measurements rather than by guess.
- **Reflect's ergonomics under a real node set** are unproven here. Params types
  must be `Reflect`, which constrains what a node author can put in them
  — trait objects, foreign types without a reflect impl, and closures all need
  work-arounds. M2 is where this is found out; the fallback is a hand-written
  schema for the few types that resist, not abandoning the registry.
- ~~**State lives in two places**~~ — resolved by §2.2. Node state is components
  on the node entity, so state lives only in the world, and snapshot/restore
  becomes a question about the world rather than a per-node protocol. The open
  part is narrower: which components are performance state worth restoring and
  which are derived caches. Still revisit before M7, but as a labelling problem.
