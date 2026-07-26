# Sway — Design

**Date:** 2026-07-25
**Status:** Approved, pre-implementation
**Revision:** graph engine builds on Bevy's non-rendering subcrates (§2.2–§2.7, §3)
**Revision:** scene composition is expressed in the graph, Houdini/USD-shaped (§2.10)

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

The graph does two things, and only two. It **declares structure** — what exists
in the scene and how it composes (§2.10) — and it **fires** — "burst here",
"retarget that colour", "start clip 3". What it does not do is drive the world
frame by frame. Structure is cooked when something changes, not on every tick,
and a fired event belongs to ECS systems from the moment it lands: an animation
triggered by a node keeps running with no further involvement from the graph.

This is why the runtime stands alone, why the graph can tick at a rate unrelated
to the render loop, and why the graph does not need to be fast.

Corollary principle: the graph is the nervous system, the Bevy world is the body.
Low-cardinality global signals (an LFO, an envelope, a CC) live in the graph's
port arena. High-cardinality data (10k points, rigid bodies, particle lifetimes)
lives in the ECS as components, parameterised by the graph. Scene composition
does not breach this: geometry is a component on an entity, never a value on an
edge (§2.10). **Physics never becomes a node.**

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
binding. A scene node's entity is additionally the scene object it makes
(§2.10). There is no `NodeInstance` trait object: registration erases
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

That statement holds without qualification, and keeping it that way is a
constraint on the node set rather than a happy accident. **A type-selector param
is a smell; make it a node type.** There is no `Material` node with a kind
dropdown — there is one node per material type, `StandardMaterial` plus one per
custom shader material, each with ports that are simply its fields. Lights are
the same: `DirectionalLight`, `PointLight`, `SpotLight`, not one `Light` with a
kind param.

This costs nothing to write, because such nodes are generated —
`impl<M: Material + Reflect> NodeType for MaterialNode<M>` with one registration
call per material type. The editor palette gains an entry per type, which is
better than a generic node plus a dropdown, and changing a material's type
becomes replacing a node rather than flipping a param — honest, since either way
it invalidates every param edge attached to it.

Variable arity is designed out the same way. `Merge` needs no input ports at
all: its inputs are `ChildOf` edges, and fan-in is unbounded by nature. `Math`
and `Switch` are binary and compose — `Switch(s1, Switch(s2, a, b), c)` covers
the three-way case without a count param.

So a registry entry is a constant derived from the params type. The compiler
never evaluates a per-instance schema, the editor's inspector is a plain walk
over a registered type, and there is one fewer moving part in both.

Port storage is a flat arena, not components, and holds **only signal values** —
scalars, vectors, colours, event streams. Geometry and scene structure are not
in it (§2.10). Ports are read and written in compiled index order by a single
system, and the editor reads the whole arena to animate live values on edges
(§2.8) — both are arena-shaped access patterns that per-entity components would
only make slower and more awkward. The arena is a resource, taken out of the
world for the duration of the tick so that a node can hold `&mut World` and
`&mut PortView` at once.

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

**Param edges are entities**, carrying source and target relationship components
with their port indices. Bevy maintains the reverse index, and despawning a node
despawns its edges — which matters at M7, where the failure mode of a
hand-rolled edge list is a dangling reference after a delete.

They are one of three edge kinds. The other two — `ChildOf` and `Feeds` — carry
no value at all and are relationships between node entities directly rather than
edge entities of their own. §2.10 defines them.

### 2.5 Compilation

```
project.ron → spawn node entities
            → structure pass:  ChildOf / Feeds — acyclic, single-parent
            → dataflow pass:   param edges — validate types → topo sort
            → flat Vec<Entity> + port arena layout
```

All failure happens at load. Tick is infallible.

**Two passes, not one.** Structure edges are not data dependencies and must not
enter the topological sort — a `Transform` node's evaluation order has nothing
to do with which entity it parents. Their validation is separate and has its own
failure modes: a parenting cycle, a `ChildOf` fan-out (illegal, an entity has
one parent), a `Feeds` slot filled twice, a `Feeds` slot filled with the wrong
kind of thing (a material into a geometry slot). Each needs an error message in
its own vocabulary; "cycle detected" is unhelpful when the author connected two
edges to one parent socket.

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

**`FixedUpdate` decouples the tick rate from the frame rate, not the tick cost.**
It runs inside `Main`, on the main thread, in the same frame — a 30 ms cook is a
30 ms frame. And the coupling is slightly worse than neutral: a tick that
overruns its timestep leaves the accumulator behind, so the next frame runs
extra ticks to catch up. `Time<Fixed>::max_delta` stops that from spiralling,
but it damps by *dropping* ticks, so a heavy cook produces a long frame followed
by a jump in graph time — a hitch and a lurch, not just a busy screen. §7 states
the position taken on this.

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
  of the entire control layer is exact, not approximate. The guarantee is over
  the *tick sequence*: tests drive `app.update()` with a fixed delta and so are
  exact by construction. Live under sustained overload, `max_delta` drops ticks
  and output diverges from the trace — acceptable for an instrument, but not a
  promise the spec should make unqualified.

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
| Scene hierarchy, transform composition | `bevy_transform` — `ChildOf`, `GlobalTransform`, propagation |
| Operator input wiring (`Feeds`) | `bevy_ecs` — custom relationships |
| Cook invalidation | `bevy_ecs` — change detection (`Changed<Geometry>`) |
| Port type registry, editor metadata, schema | `bevy_reflect` — `TypeRegistry`, `TypeData`, field attributes |
| Type-erased port values | `bevy_reflect` — `Box<dyn PartialReflect>` |
| Params (de)serialisation | `bevy_reflect` serde |
| Tick rate, accumulator, catch-up | `bevy_time` — `Time<Fixed>` |
| Beat time, pause, tempo scaling | `bevy_time` — `Time<Transport>` |
| Project loading, file watching, hot reload | `bevy_asset` — `AssetLoader`, `AssetEvent` |
| Registration surface, schedule placement | `bevy_app` |
| Type validation, topological sort, error reporting | **ours** |
| `Geometry` attribute tables and the operators over them | **ours** |
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

### 2.10 Scene composition

The scene is built by the graph, not loaded beside it. Camera, lights, meshes,
groupings, materials and transforms are all authored as nodes, and there is no
base scene file that the graph layers over: content comes from Blender through
`Asset()` nodes at the leaves, composition comes from the graph. Ownership is
total, which is what makes teardown and reload answerable.

The model is Houdini's and USD's rather than a node-per-object scene editor. The
distinction is load-bearing: **operators act on streams, so cardinality lives in
the data, not the node count.** Thirty-two satellites are a `Scatter` and a
`CopyToPoints`, not thirty-two nodes.

**A node entity is a scene entity.** §2.2 already makes a node instance an
entity carrying `Params` and `State`; a scene node additionally carries
`Transform` and `Geometry`. There is no handle, no mapping table, no reconcile
step, and selecting a node in the editor selects the object it makes, because
they are the same entity.

#### Components

- **`Geometry`** — a named attribute table: `P`, `N`, `Cd`, `pscale`, plus
  arbitrary custom attributes. Planar, not interleaved, as in Houdini and USD —
  which is also the layout the GPU wants. One component holding a map rather
  than one component per attribute, because an author can create `@myattr` at
  runtime and component types cannot be registered then.
- **`Transform` / `GlobalTransform`** — Bevy is already the local↔world pair
  with propagation. Nothing to build.

An entity carries either, both, or neither. That is USD's prim-with-schemas
model: `Xform` and `Mesh` are independent capabilities of a prim, not a class
hierarchy.

#### Three edge kinds

| Kind | Compiles to | Carries | Fan-out |
|---|---|---|---|
| `ChildOf` | Bevy hierarchy | nothing | illegal — one parent |
| `Feeds` | a Bevy relationship, into a named typed slot | nothing | legal |
| param edge | an edge entity + arena slot | a signal value | legal |

Only param edges touch the port arena, and only they enter the topological sort.
`ChildOf` composes transforms; `Feeds` is Houdini's SOP wire, and an operator
reads its input's `Geometry` component rather than receiving a value.

One direction note, because it reads backwards in the compiler: dataflow runs
leaf→root while parenting runs root→leaf, so a `ChildOf` edge's *source* is the
child and its *target* is the parent.

**The rule that tells an author which edge they want:** object-level composition
— place, group, instance, assign — is structure. Element-level operations —
scatter, noise, displace — are data.

```
Grid ────────────── feeds(points) ──→ Scatter
Scatter ─────────── feeds(points) ──→ CopyToPoints
Asset("sat.glb") ── feeds(proto) ───→ CopyToPoints
CopyToPoints ────── feeds(geo) ─────→ Mesh("sats")
StandardMaterial ── feeds(material) → Mesh("sats")
Mesh("sats") ────── childOf ────────→ rig ── childOf ─→ root
Asset("hero.glb") ─ childOf ────────→ rig
DirectionalLight("key"), Camera ─ childOf → root

MidiNote ──> Envelope ─┬─param→ StandardMaterial("shiny").emissive
                       └─param→ hero.scale
MidiCC 74 ─> Smooth ────param→ DirectionalLight("key").illuminance
LFO(1/2 bar) ───────────param→ rig.rotate.y
```

`Grid`, `Scatter` and `CopyToPoints` carry `Geometry` and no `Transform`; they
are operators and sit outside the scene tree entirely. `Mesh` carries
`Transform`, `Mesh3d` and `MeshMaterial3d` and is in it. Which components an
entity has *is* the distinction, visible the ECS-native way. `CopyToPoints`
produces one buffer of instances — the scattered points never individuate into
entities.

**`Mesh` is where a `Feeds` chain enters the `ChildOf` tree**, and it is the
only place that happens other than `Asset`, which imports a glTF subtree
directly. Naming that boundary is most of what an author needs to understand
about the two chain kinds.

#### Materials are nodes, not assignments

A material node owns a `Handle<M>` and a `Mesh` node takes it as a typed input
slot. There is no node that assigns a material to something else, and therefore
no node that reaches into entities it does not own — the ownership rule of §2.2
holds without exception.

There is one node type per material type, not one `Material` node with a type
param, for the reason given in §2.4: it keeps every node's port schema derivable
from its params type alone.

The second effect matters more in practice. Material sharing becomes a visible
topology fact rather than hidden aliasing: one material node feeding three
`Mesh` nodes is obviously shared, and three material nodes are obviously not. The
failure this designs out is real and nasty — with assignment-style materials,
driving one object's emissive silently drives every object sharing the handle,
and the graph gives no indication. Here, wanting independent emissive means
drawing a second material node, which is exactly the thought the author should
be having.

`Feeds` slots are consequently **named and typed**: `points`, `proto`, `geo`,
`material`. A material output cannot fill a geometry slot, and that is checked
in the structure pass (§2.5) alongside cycles and fan-out. The edge still
carries nothing at runtime — the target reads the source's component or handle —
but it is not untyped.

#### Two things this gets for free

**Intermediate results are inspectable.** Only entities marked renderable draw,
so cooked geometry on operator nodes sits in the world undrawn and available.
That is Houdini's per-node display flag, obtained by toggling a component, and
it is the single most useful debugging affordance in this class of tool.

**`Changed<Geometry>` is the cook invalidation.** A node cooks only when an
input's geometry changed, its params changed, or it is time-dependent. Bevy's
change ticks plus the flat compiled order supply the dirty propagation; there is
no cache to write. Structure needs no cooking at all — a `Transform` node writes
its own component when a param changes and Bevy's propagation does the rest,
per-frame, where that work belongs.

#### Geometry residency — direction, not settled design

Geometry operators should run as compute shaders wherever the work allows it.
`Geometry`'s planar attribute layout was chosen partly for this: it is already
what the GPU wants. Note that the toolkit is compute shaders, vertex pulling
from storage buffers, and instancing — **there are no geometry shaders**, since
wgpu and WebGPU have no such stage and Metal never had one.

The structural consequence is larger than it first appears. Bevy's render world
is a separate world extracted once per frame, while the graph ticks in
`FixedUpdate` on the main world, so a cook cannot dispatch and read back
synchronously — it can only enqueue. **GPU cooking therefore leaves the graph
tick entirely:**

```
FixedUpdate   graph tick: apply params, mark dirty nodes
Extract       dirty set + ShaderParams → render world
Render        a render-graph subgraph dispatches compute in Feeds order;
              results stay in GPU buffers and are consumed by the draw
```

Mostly this is a gain. Dispatch coalesces per frame, so a param changing across
three ticks costs one dispatch, and cook cost genuinely leaves the frame's
critical path — the async escape hatch of §7 arriving by a different road.

**The port arena stays on the CPU.** It holds signal values that CPU-side nodes
consume; making it GPU-resident would turn every LFO write into a GPU write and
force readback for nodes reading their own inputs. The narrower split: a node
feeding a compute op writes its effective params into a `ShaderParams`
component (`#[derive(ShaderType)]`) on its own entity, which extraction uploads.
That is Bevy's existing material-uniform path, already paved. `Geometry` becomes
a handle to a GPU buffer rather than CPU arrays, and the cook invalidation
above is unaffected, since it keys on the component's change tick rather than
its contents.

The line for which operators can go to the GPU is **whether output size is known
before dispatch**. Element-wise work — noise, displace, transform, colour — and
`Scatter` at fixed count are clean. `CopyToPoints` is often no dispatch at all:
bind the point buffer as instance data and let the draw expand it. Variable
output size (delete-by-threshold, fracture) needs atomic counters and indirect
dispatch, and is later work. Anything rewriting topology or needing adjacency,
such as subdivide or fuse, stays on the CPU.

This qualifies one of the free wins above: with geometry resident on the GPU,
inspecting an intermediate node's output needs an explicit async readback rather
than a component read. Still worth having, but editor-requested and a frame
late, not free.

**The hazard to design for now** is mixed residency. A CPU operator wedged
between two GPU operators forces a readback and a stall, and in the graph it
looks identical to a chain that stays resident. Same shape as cook cost, so the
same position (§7): the tool reports rather than polices. Residency is shown on
the node — border, badge — so a ping-pong is something an author sees rather
than something they profile.

`sway-geo` consequently sits on the render side and depends on `bevy_render`.
§2.9's rule survives untouched: it constrains `sway-graph`, not the node crates.

#### What this deletes

The commit and reconcile stage, a `SceneNode` port type, bind points, name
resolution against an external scene file, and the whole sink-node set. A signal
connects directly to the parameter port of the node that builds the thing, so a
target cannot go stale.

#### What it gives up

Stream rewriting for object-level operations. In Houdini, `Transform →
Subdivide → Scatter` are all operators on one geometry stream. Here `Transform`
is a hierarchy node rather than a data operator, so transforming points and then
scattering on the result is a `Feeds` chain, not a `ChildOf` chain. The gain is
Bevy's transform propagation for free; the loss is that the two chains are
different chains and the author has to know which is which. Hence the rule
above.

## 3. Crate layout

```
sway-gpu        wgpu instance/device/queue creation — the single place the
                bevy↔vello version coupling lives
sway-graph      engine: port kinds, edge kinds, node type registry, compiler,
                port arena, cook gating, tick runner, project format
sway-nodes      built-in node types — signal nodes and scene nodes
sway-geo        Geometry attribute tables and the operators over them; sits on
                the render side, depends on bevy_render (§2.10)
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

`sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_reflect`, `bevy_time`,
`bevy_transform`, and `bevy_asset` — not on `bevy`, and specifically not on
`bevy_render`. `bevy_transform` joins the list because §2.10 makes the scene
hierarchy part of the graph; it is headless and pulls no renderer. Making the
engine generic over a context type was considered and rejected: a minimal
headless `App` is cheap to construct in tests, so the abstraction would buy
nothing real.

## 4. Testing strategy

- **Graph engine** — golden-trace tests. A recorded MIDI trace plus a fixed tick
  rate produces bit-identical output; assert against stored expectations.
- **Transport** — recorded clock traces including tempo changes and dropouts.
- **Compiler** — table-driven tests for type mismatches, cycles, missing nodes,
  unknown types, and the structure-pass failures of §2.5: parenting cycles,
  `ChildOf` fan-out, a `Feeds` slot filled twice. Every failure mode must produce
  a clear load-time error in the vocabulary of the edge kind that failed.
- **Cooking** — a cook is a pure function of its inputs, so assert on the
  resulting `Geometry` attributes directly. Also assert the *negative*: that an
  unrelated param change cooks nothing, since a broken invalidation gate is a
  performance bug that no output assertion would catch.
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

**Extended to cover compute.** One geometry operator — `Scatter` at fixed count
is the obvious candidate — dispatched from a dirty set carried through
extraction, writing a buffer the draw consumes. Same undocumented surface, same
rule that put this milestone out of order in the first place. If the
extract-and-dispatch shape of §2.10 turns out to be unworkable, that changes the
operator set, and learning it here costs a spike rather than a rewrite of M5.

*Exit:* a point cloud and a z-depth sprite layer render at frame rate with custom
vertex/fragment shaders, and one compute-cooked geometry operator dispatches from
a graph-shaped dirty set. The code is provisional — the goal is knowledge, not
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
entities, the three edge kinds with their two validation passes, type-erased
ports with `Continuous`/`Event` kinds, compiler, `FixedUpdate` runner. Initial
node set: MidiNote, MidiCC, LFO, Envelope, Math, Remap, Switch, Select.
Golden-trace test harness. Graphs still constructed in Rust.

Cook gating (`Changed<Geometry>` plus params) belongs here rather than at M5,
even though no node cooks anything yet: it determines what the tick loop looks
like, and retrofitting it around an existing runner is worse than building it
in. A trivial `Geometry`-producing node is enough to exercise it.

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

Node types are referenced in the file by a short registered name, not by reflect
`TypePath`. §2.4's generated node types make this necessary rather than merely
nicer: the path for a generic node reads
`sway_nodes::MaterialNode<bevy_pbr::StandardMaterial>`, which no one should have
to type or read in a hand-authored document, and which pins an internal module
layout into the file format.

Constraint: the format is both human- and machine-authored, so it must survive
round-tripping through the editor without destroying comments or ordering. Decide
this here, not at M7. **Reflect does not solve this** — `ReflectSerializer`
output is verbose and comment-destroying, and RON does not preserve comments on
round-trip either. The expected shape is reflect for *reading* and a
hand-controlled emitter for *writing*, editing the existing document in place
rather than regenerating it.

*Exit:* a set can be authored by editing text with the app running.

### M5 — Visual runtime (L)

The real version of M1, and the milestone that makes §2.10 real. The scene node
set — `Asset`, `Transform`, `Group`, `Camera`, `Mesh`, one node per light type,
one per material type — and the geometry operators — `Grid`, `Scatter`,
`CopyToPoints` — plus the `Geometry` component and the renderable marker. Runtime services (`PointCloudSet`, `SpriteLayers`,
`Emitters`, `CameraRig`, `AnimationDirector`) with owned invariants, glTF mesh
instancing, curve-driven procedural animation, physics if wanted. Where the
fire-and-forget decoupling earns its keep: nodes trigger, ECS systems continue.

Two things deliberately *not* here. An attribute expression node — Houdini's
wrangle, where most of its power concentrates — is a language or a compiled
kernel and is its own project; ship fixed operators first. And sinks driven at
tick rate will step visibly when rendering runs faster, so a continuously driven
`Transform` should write previous and next and let a per-frame system lerp by
`Time<Fixed>::overstep_fraction`. Standard fixed-timestep render interpolation,
cheap here, awkward to retrofit.

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

Two things specific to §2.10. **Deleting a scene node must reparent before
despawning** — Bevy's despawn cascades to `Children`, and a scene node's
children are its inputs, so deleting a `Group` would otherwise take out
everything feeding it. And the canvas should surface **per-node cook time and
the display flag**, which are the two affordances that make a Houdini-shaped
graph debuggable at all (§7, §2.10).

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

- **Cook cost belongs to the graph author, not the tool.** This is a decision,
  recorded here because §2.6 makes its consequence unavoidable: a cook runs on
  the main thread inside the frame, so an expensive one hitches. Houdini and
  TouchDesigner take the same position, and the alternative — a tick budget that
  silently defers work — trades a visible problem for an invisible one. The tool
  therefore *reports* cost rather than policing it: per-node cook time in the
  editor (M7), and a reflect marking on params that invalidate `Geometry` so the
  author can see that `Scatter.count` is not `Light.intensity`. The residual
  risk is covered by M6's watchdog rather than by the engine.

  The escape hatch, if this ever does bite: cook on `AsyncComputeTaskPool` and
  apply the result when it lands, which genuinely decouples cost from the frame
  at the price of geometry arriving a frame or more late. Named here so it is a
  known option rather than a 2am rediscovery.

- **Which geometry operators are GPU-resident** cannot be answered before M1
  produces one. The shape is decided (§2.10: extract a dirty set, dispatch a
  render-graph subgraph in `Feeds` order, params through `ShaderParams`, arena
  stays on the CPU) and the criterion is decided (output size known before
  dispatch). What is open is how far the criterion reaches in practice, and
  whether mixed residency proves tolerable or forces a rule that a `Feeds` chain
  must be entirely one or the other. Answer at M1, revisit before M5.

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
