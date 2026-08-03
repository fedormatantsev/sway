# Sway — Design

**Date:** 2026-07-25
**Status:** In implementation — M0, M1, M1b, M2a, M2b, M2c complete; M3 next
**Revision:** graph engine builds on Bevy's non-rendering subcrates (§2.2–§2.7, §3)
**Revision:** scene composition is expressed in the graph, Houdini/USD-shaped (§2.10)
**Revision (2026-08-02):** §5 and §7 reconciled against what was actually built.
**Revision (2026-08-03):** unified edges — one `Edge`, one arena, one compiled
order, replacing the three edge kinds and five node-declaration mechanisms
(§2.4, §2.5, §2.10, §2.11, §5, §7).
M2 shipped as M2a + M2b, an unplanned M2c added the editor's first real views,
and each completed milestone now carries the debt it did not discharge. The
architecture sections are unchanged; where implementation contradicted them the
correction lives in the milestone's findings report and is named in §7.

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

Variable arity is *declared*, not designed out. A node's inlet **count**
varies by instance; an inlet's **arity** does not — each element of a `Vec`
field is an independent single-source inlet, so `Merge`'s inputs are
`Vec<Product<Geometry>>` and `Sum`'s are `Vec<f32>` rather than a `ChildOf`
fan-in or an engine-side combining rule. The compiler reads a `Vec` field's
length off the instance once, at compile time, to lay out that many inlets and
their arena slots — that is the one number that is per-instance rather than
per-type. `Math` and `Switch` stay binary and compose regardless —
`Switch(s1, Switch(s2, a, b), c)` still covers the three-way case without a
count param — because nothing about declared arity forces every node with
multiplicity to use it.

So a registry entry is constant per type — same fields, same types, same
ordinals — except for that one per-instance count, read from the instance at
compile time and nowhere else. The port type registry subsumes capabilities
the same way it subsumes everything else: a `Product<Geometry>` inlet matches
a `Product<Geometry>` outlet by `TypeId`, through the same check that matches
`f32` to `f32`, so there is no separate mechanism for a structural connection
to type-check against. The editor's inspector remains a plain walk over a
registered type.

Port storage is a flat arena, not components, and holds **every slot value** —
scalars, vectors, colours, event streams, and the entity references that stand
in for structural connections. High-cardinality data itself — `Geometry`, a
material's `Handle` — is never in the arena, only a reference to the entity
that owns it (§2.10); that is what keeps a structural connection an ordinary
slot rather than a bypass around the arena. Ports are read and written in
compiled index order by a single system, and the editor reads the whole arena
to animate live values on edges (§2.8) — both are arena-shaped access patterns
that per-entity components would only make slower and more awkward. The arena
is a resource, taken out of the world for the duration of the tick so that a
node can hold `&mut World` and `&mut PortView` at once.

Three slot types, not two:

- a plain reflected value (`f32`, `Vec3`, `Waveform`) — always holds a current
  value.
- `Events<T>` — zero or more occurrences per tick, each with a sub-tick
  timestamp. This absorbed what used to be a separate `Event<T>` port kind: an
  ordinary value holding a typed `Vec<Occurrence<T>>`, not a zero-sized marker
  over a parallel arena.
- `Product<T>` — `Option<Entity>`, the source node's entity. Unconnected, it
  holds `None`, which is its authored value like any other slot; connected, it
  is what a `Feeds` slot or a `ChildOf` edge used to be, expressed as an
  ordinary typed value.

The value/event split is still required for the reason it always was: without
it there is no way to distinguish "CC is 0" from "no CC arrived", and note
timing collapses to tick granularity. Sub-tick timestamps let a note landing
between ticks start its envelope at the correct phase.

Observers are the wrong tool here despite the surface resemblance to
`Events<T>`. They fire immediately and recursively, which cannot be reconciled
with topologically ordered evaluation, and they carry no notion of buffering
several occurrences with sub-tick offsets to be drained at a known point.

**Edges are entities.** One `Edge` component carries a `from`/`to` pair of
`(field, index)` endpoints; `EdgeFrom`/`EdgeTo` relationship components keep
Bevy's reverse index, and despawning a node despawns its edges — which matters
at M7, where the failure mode of a hand-rolled edge list is a dangling
reference after a delete.

There is one edge kind, not three. What used to be three edge kinds — param,
`Feeds`, `ChildOf` — is now one inlet-type question: a plain-value or
`Events<T>` inlet is what a param edge fed; a `Product<T>` inlet is what a
`Feeds` slot fed; a `Product<Spatial>` inlet is what `ChildOf` meant, and an
edge into one still emits Bevy's `ChildOf` (§2.10). The rule that used to tell
an author which edge kind they wanted is now a rule about which inlet type a
node declares.

### 2.5 Compilation

```
project.ron → spawn node entities
            → validate: type match, direction, inlet-already-connected;
                         Product<Spatial> single-consumer and parenting acyclicity
            → one topological sort, over every edge except Product<Spatial>
            → flat Vec<Entity> + port arena layout
```

All failure happens at load. Tick is infallible.

**One pass and one sort, not two.** Every edge is now a `Product<T>`-, `Events<T>`-
or value-typed dependency between arena slots, so the old structure/dataflow
split collapses to a single rule: **everything except `Product<Spatial>` is a
dependency**, and enters the one topological sort. A `Product<Spatial>` edge
still emits Bevy's `ChildOf` for the scene hierarchy, but a parent does not
read anything from its child, so it is excluded from ordering — the one
survivor of the old structure-pass argument, not the whole pass. Failure modes
that used to belong to separate passes are now just edge types the validator
already knows: a parenting cycle or fan-out is a cycle or a duplicate-consumer
error on `Product<Spatial>` edges specifically; a slot filled twice or filled
with the wrong kind of thing is `InletAlreadyConnected` or a type mismatch on
any inlet, structural or not. Each is still reported in its own vocabulary;
"cycle detected" is unhelpful when the author connected two edges to one
parent socket.

**The union of what used to be two DAGs can contain a cycle where neither did
alone, and such a graph is now rejected.** A node feeding another that in turn
drives a param back on the first is a genuine circular dependency; the old
two-pass model let it compile, with one side silently reading stale data
because phase ordering resolved it. One sort turns that into a load-time
error, which is the better outcome.

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
| Cook invalidation | `bevy_ecs` — change ticks (compared explicitly, §2.11) |
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

#### One edge, three inlet types

There is one edge kind (§2.4's `Edge`); what a connection means is a question
about the **inlet's** declared type, not a choice among edge kinds:

| Inlet type | Compiles to | Fan-out | Enters the compiled order |
|---|---|---|---|
| `Product<Spatial>` | an `Edge` entity + arena slot; also emits Bevy's `ChildOf` | illegal — one parent | no |
| `Product<T>` (any other capability) | an `Edge` entity + arena slot, holding the source's entity | legal | yes |
| `Events<T>` / plain value | an `Edge` entity + arena slot, carrying the signal itself | legal | yes |

Every inlet touches the port arena, and every edge except one into a
`Product<Spatial>` inlet enters the topological sort (§2.5). `Product<Spatial>`
still composes transforms via Bevy's `ChildOf`; `Product<T>` for another
capability (`Geometry` chief among them) is Houdini's SOP wire, and an
operator reads its input's `Geometry` component off the referenced entity
rather than receiving a value through the arena.

One direction note, because it reads backwards from how a hierarchy pane draws
it: a node's own `Product<Spatial>` **outlet** holds its own entity, and it
feeds its parent's `Product<Spatial>` **inlet** (e.g. `Group.children`) — so
the edge's *source* is the child and its *target* is the parent, exactly as
it was under the old `ChildOf` edge kind. Only the edge kind carrying it
changed; the direction did not.

**The rule that tells an author which inlet type a node declares:** object-level
composition — place, group, instance, assign — declares a `Product<Spatial>`
inlet. Element-level operations — scatter, noise, displace — declare a
`Product<T>` inlet for another capability, `Geometry` chief among them.
Driving a value — colour, rotation, intensity — is a third, separate
question, answered by a plain-value or `Events<T>` inlet regardless of which
of the first two a node's other inlets use.

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

**`Mesh` is where a `Product<Geometry>` chain enters the `Product<Spatial>`
tree**, and it is the only place that happens other than `Asset`, which
imports a glTF subtree directly. Naming that boundary is most of what an
author needs to understand about the two chain kinds.

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

Structural inlets are consequently **named and typed**: `points`, `proto`,
`geo`, `material` are each a distinct `Product<T>` field. A material output
cannot fill a `points` inlet, because `Product<StandardMaterial>` does not
match `Product<Geometry>` by `TypeId` — the same check any other inlet's type
gets (§2.5). The arena slot holds only the source's entity; the target still
reads the source's component or handle through it, but the check that stops a
material from filling a geometry inlet is no longer a separate pass.

#### Two things this gets for free

**Intermediate results are inspectable.** Only entities marked renderable draw,
so cooked geometry on operator nodes sits in the world undrawn and available.
That is Houdini's per-node display flag, obtained by toggling a component, and
it is the single most useful debugging affordance in this class of tool.

**Bevy's change ticks are the cook invalidation.** A node cooks only when an
input's geometry changed, its params changed, or it is time-dependent. Change
ticks plus the flat compiled order supply the dirty propagation; there is no
cache to write. The comparison is explicit rather than a `Changed<T>` query
filter, for a reason §2.11 gives. Structure needs no cooking at all — a `Transform` node writes
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

### 2.11 Graph state reaching the ECS

Most of what "propagation" usually means does not exist here. Node params, node
state, geometry and transforms are already components on node entities (§2.10),
so nothing crosses a boundary between two representations. What happens instead
is a few narrow write paths inside the tick, after which Bevy's own machinery
takes over.

```
PreUpdate     MIDI IO thread → timestamped event buffers
FixedUpdate   (0..n times per frame)
                advance Time<Transport> from the phase estimator
                graph tick — one exclusive system, one compiled order;
                per node, in that order:
                  A/C. gather its inlets, tick (write outlets), cook if dirty
                  B.   apply effective params → own components
                  D.   fire observer triggers
Update        runtime systems: animation, particles, physics
PostUpdate    transform propagation, visibility
Extract       accumulated dirty set + ShaderParams → render world
Render        compute subgraph in `Product<Geometry>` order, then the draw
```

Because the tick is an exclusive system holding `&mut World`, **writes are
immediate**: a node later in topological order sees an earlier node's component
writes within the same tick. Routing through `Commands` would introduce a flush
boundary and a tick of lag, which is a concrete payoff of the §2.6 choice.

#### A/C — gather, tick, and cook, per node

One step now, not two. A `Product` edge is an ordinary arena dependency, the
same as a plain value or an `Events<T>` occurrence list, so the old split —
every node ticks, then every node cooks — collapses into one per-node
sequence: gather this node's connected inlets from their sources' outlet
slots (a plain value, an event list, or a `Product<T>`'s source entity, all
copied the same way); run its `tick`, which writes its own outlets and (§B)
its own components; and, immediately after, if the cook gate says dirty and
the node declares `COOKS`, run its `cook`, reading whichever entities its
`Product` inlets reference. Pure arena gathers are the only work that runs for
every node on every tick; a cook runs only when its node is dirty.

Because gather, tick and cook for one node all complete before the next
node's turn, a node whose tick depends on another node's cook from the same
tick is expressible: the compiled order guarantees the producer's cook has
already run before the consumer's tick reads it, the same way it guarantees
one node's outlet is written before another node's inlet copies it. §7's open
question about this is closed by that guarantee.

A CPU operator's cook reads its `Product` sources' `Geometry` through the
world, computes into a local, and inserts into itself. `Geometry`'s buffers
are `Arc`-backed, so passing an unchanged attribute through an operator is a
refcount bump rather than a copy. A GPU operator does none of this in the
tick — it only joins the dirty set, and §2.10 describes where it runs.

**The naive `Changed<Geometry>` filter is wrong here, and the failure is
silent.** The filter means "changed since this system last ran", and the
graph tick system runs every tick — so the flag is true for exactly one tick
after an upstream write, and a node that skips cooking on that particular
tick for any other reason misses the change permanently. Instead each node
stores, in its `State`, the change tick of every input it last cooked
against, and compares against `get_change_ticks::<Geometry>()` (or the
equivalent for whatever component its `Product` inlet references). That is
robust regardless of cadence and survives a node being added mid-session.

The dirty set for GPU cooks accumulates across every tick in a frame and is
drained at extraction, so a param changing on three consecutive ticks
produces one dispatch. A CPU operator downstream of a GPU one cannot read its
result during the tick at all — that is the mixed-residency ping-pong of
§2.10, and it costs either a stall or a frame of latency.

#### B — effective params into components

The actual graph→ECS write, and it is small: each node writes only its own
entity's components. A `Transform` node writes `Transform`; a material node
writes through its `Handle<M>` into `Assets<M>`.

**A connected port shadows the authored value; it does not overwrite it.**
`Params` holds what the author wrote, the arena holds what the edge is currently
sending, and the effective value is the arena's when the port is connected and
the params field's when it is not. Three things follow, all wanted: disconnect a
CC from a light and it returns to its authored value rather than freezing
wherever the CC last left it; saving the project cannot bake in whatever the LFO
happened to be at; and the inspector can show authored and live values at once.

**Write only on change.** Assigning `Transform` unconditionally sets Bevy's
change tick every tick, which re-runs transform propagation and re-uploads to
the GPU for a scene that is not moving, and makes `Changed<Transform>`
worthless for every downstream consumer. Components use `set_if_neq`. Assets
need more care: `Assets::get_mut` marks the asset changed by the act of calling
it, so a material write is `get`, compare, and only then `get_mut`.

One interaction to keep in mind at M5: continuous driving plus render
interpolation both target `Transform`, and they cannot both own it. The node
writes a `DrivenTransform` carrying previous and next; the per-frame
interpolator writes `Transform`.

#### D — events

A node calls `world.trigger(...)` and observers run immediately and
synchronously inside the tick, so their effects are visible to later nodes in
the same tick. An observer may spawn, despawn and mutate components freely, but
**must not touch the port arena**: it is not in the world during the tick
(§2.4), and re-entering the graph from an observer would break the ordering
guarantee the compiled order exists to provide.

#### Then nothing

After the tick no graph code runs. Transform propagation, visibility and render
extraction are Bevy's, reading components the graph happened to write. Between
ticks — and there may be several frames between them — the world keeps animating
on its own. That is §2.1 as a mechanism rather than a principle. The editor
likewise reads rather than receives: live port values come from the arena and
live node values from components, with nothing pushed to it.

#### Structural change is a separate, rarer path

Spawning and despawning never happen during a tick. On load, reload, or an
editor edit, the compiler runs and **reconciles by node ID**: nodes present in
both the old and new project keep their entity, their state, and their ECS
continuations; removed nodes despawn, cascading to children and edges; added
nodes spawn; a node whose *type* changed under the same ID is a remove plus an
add. Edges are rewired and the order recompiled.

This is worth stating precisely, because §2.10 claims the reconcile stage is
deleted. What is deleted is the *per-frame* reconcile a stream-and-commit design
would have needed. A reload-time reconcile remains, and it is what keeps hot
reload from resetting every LFO phase and killing every running animation on
each save — the difference between authoring with the app running and restarting
the app on every edit.

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
  performance bug that no output assertion would catch. The §2.11 failure mode
  deserves its own test — a node that skips a tick for an unrelated reason must
  still cook the upstream change, which is exactly what a `Changed<T>` filter
  would get wrong.
- **Reload** — reconcile a project against an edited copy and assert that
  surviving nodes kept their entity and state, that removed nodes took their
  edges with them, and that a node whose type changed was replaced rather than
  mutated.
- **Runtime** — replay recorded MIDI traces through a graph into a headless world;
  assert on ECS state and service calls rather than pixels.
- **Rendering** — no pixel-diff tests. Verified by eye.

## 5. Roadmap

Sizes are relative, not calendar. Ordering follows two rules: get one end-to-end
path working before deepening any layer, and pull genuinely unknown work early.

**Status at 2026-08-03.** M0, M1, M1b, M2a, M2b and M2c are complete and on
`main`; M3 is next. The unified-edges migration
(`reports/2026-08-03-unified-edges-findings.md`) is also complete — done ahead
of M3 because it is M4's opening work: one `Edge`, one arena, one compiled
order, replacing the three-edge-kind model and five node-declaration
mechanisms the RON format would otherwise have had to encode and then break.
A completed milestone's plan is superseded by its findings
report, which is linked below and is the authority on what was actually built.
Where one left debt it carries a *Carried forward* line saying so, and the
milestone that inherits it says so too — the point is that debt is visible in
the roadmap rather than only in a report nobody re-reads.

### M0 — Walking skeleton (S) — **complete**

MIDI note in → hardcoded Rust graph → a cube changes colour → fullscreen on the
HDMI display. No file format, no editor, no abstraction. Proved the MIDI IO
thread, the `FixedUpdate` tick position in the schedule, and fullscreen output on
an external display.

The cube and its `bridge.rs` were retired at M2b, having been replaced by a real
graph-authored scene. That is the intended fate of a walking skeleton.

### M1 — Render spike (M, high risk) — **complete, with one gap**

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

*Outcome* (`reports/2026-07-26-m1-render-spike-findings.md`): the point cloud is
a custom `SpecializedMeshPipeline` and the sprite layers took the `Material`
path; both hold frame rate indistinguishably from baseline at 50k points and 5
layers. `Scatter`'s compute pass dispatches once per dirty source, and a
`Readback` proves its output correct.

**The compute buffer never reached the draw.** That is a scope boundary, not a
format incompatibility — `point_cloud.rs` already binds two independently
strided vertex slots, and a third fed from scatter's raw position buffer closes
it with no second pass. The distinction matters and should not drift into the
stronger claim.

*Carried forward to M5:* closing compute→draw; the point cloud's unamortised
per-frame ~1.6 MB CPU clone plus ~1.6 MB re-upload; and the fact that these
demos are standalone, spawn their own cameras, and are therefore mutually
exclusive with the graph-built scene. None of them is a node yet.

### M1b — Integration spike (S) — go/no-go gate: **passed**

Headless Bevy rendering to a texture using an externally-created device,
composited by a Vello-backed masonry widget, in one process. Extended with a
pan/zoom canvas holding draggable boxes and bezier edges, to prove masonry can
carry a node editor at all.

*Outcome* (`reports/2026-07-28-m1b-integration-findings.md`): one `wgpu 29.0.4`
and one `winit 0.30.13` resolve across bevy 0.19 and `imaging_vello`, pinned
exactly in the workspace manifest and asserted by a compile-time test. Bevy's
output reaches the screen through our texture and our compositor. **The Syphon
fallback of §2.8 and the two-device CPU-copy fallback are both retired unused.**

The one non-obvious cost: masonry resets `paint_layer_mode` to `Inline` for
every widget on every redraw, so the `External` viewport layer only survives if
the host pumps anim frames continuously. That is masonry's own reference host
pattern, not a workaround, and it must not be simplified away.

### M2a — Graph engine core (L) — **complete**

M2 shipped in two halves. M2a is `sway-graph`'s signal half: the `NodeType`
contract with reflect-derived params, node and param-edge entities, type-erased
`Continuous`/`Event` ports over the arena, the dataflow compiler and its
topological sort, the `FixedUpdate` runner, and the eight signal nodes —
MidiNote, MidiCC, LFO, Envelope, Math, Remap, Switch, Select — behind a
golden-trace harness. Graphs are still constructed in Rust.

*Outcome* (`reports/2026-07-31-m2a-graph-engine-findings.md`): erased
`Box<dyn PartialReflect>` was adequate and forced no hand-written schema;
positional ordinal consts held across all eight types, once ordinal identity
was corrected to the `(name, ordinal)` pair; `Envelope` gained a second event
input, because note-off-driven release is graph input rather than hidden
controller logic.

*Carried forward:* nine reviewer-approved non-blocking gaps, all test coverage
or comments, listed in the report's "Deferred minor findings". The MIDI epoch
bridge is still throwaway and does not correct long-session mach-versus-fixed
drift — **M3 owns that**, since it is the same clock problem the transport is
about.

### M2b — Structure edges, geometry, the cook gate (L) — **complete**

M2's second half, and the milestone that made §2.10 real for the first time:
`ParentEdge` and `FeedsEdge` with the structure validation pass, the `Geometry`
component with its `Arc`-backed attribute tables, cook gating on change ticks,
the tick's second pass, and six nodes — `Grid`, `Displace`, `Mesh`, `Group`,
`StandardMaterial`, `Rgb`.

Cook gating belonged here rather than at M5 even though almost nothing cooks
yet: it determines what the tick loop looks like, and retrofitting it around an
existing runner is worse than building it in. The authored-versus-driven param
rule of §2.11 landed here for the same reason — it is a semantic, and every node
written after it assumes one answer or the other.

*Outcome* (`reports/2026-08-01-m2b-scene-composition-findings.md`): two orders
held with no cross-DAG constraint wanted; `Slots` plus `Produces` is the right
split once `Produces` gains a companion `produced_change_tick`; `Arc` sharing
measured 14.2× against a deep-copy counterfactual at the demo graph's own size,
and doubles as the mesh-upload gate. The gate is worth ~20× on the demo graph
(624 ns closed against 13.07 µs open).

**The gate is one bit per node, not one per reason.** A node dirtied by a param
edit cannot ask whether its *geometry* changed, so a node owning an expensive
resource needs a second, node-local gate inside its cook. §2.11 presents the
engine gate as the whole answer and it is not; the consequences are in §7.

*Exit met:* a Rust-built graph cooks a two-operator `Feeds` chain into a `Mesh`
under a `Group`, with live MIDI driving displacement, colour and rotation.

*Not met from M2's original exit:* **the graph does not drive M1's visuals.**
It drives its own scene. Point clouds and sprite layers become nodes at M5.

*Also not done:* the throwaway reflect-driven inspector M2 asked for, which
existed to find missing editor `TypeData` early. That risk is undischarged and
is pulled into M4 below.

### M2c — Scene and graph views (M) — **complete**

Unplanned, inserted between M2b and M3, and worth the detour: M3 is about a
phase estimate that is either right or wrong, and debugging it against a black
window and a log file is worse than debugging it against a picture of the graph.

Three panes wired to the live world — a scene/entity tree, the Bevy viewport,
and a graph canvas driven by a per-frame `capture(&World)` snapshot with live
activity on continuous edges. Read-only throughout. `EditorPos` became a
component in `sway-graph`, node identity became `NodeId` so a box keeps its
`WidgetId` across snapshots, and drag-to-connect was deleted rather than left
inventing edges that exist in no graph.

Design: `specs/2026-08-02-scene-and-graph-views-design.md`. **This is M7's first
slice, not an overlay** — the snapshot, the pane layout and the widget identity
model are what M7 extends.

*Carried forward:* event edges show no activity, because a frame-rate sampler
observes roughly half of a one-tick event and a randomly pulsing edge is worse
than a static one. The honest fix is a per-edge ring buffer written by the tick,
and it belongs at M7 with the rest of the editor's write paths. Dragged
positions do not persist (M4 serializes them, M7 writes them back), and node
display names are shortened from `type_name` until M4's registered short names
replace the shortening outright.

### M3 — Transport and beat lock (M) — **next**

MIDI clock ingestion at 24 ppqn, drift-corrected phase estimator,
start/stop/continue, tempo tracking. Transport-aware nodes: tempo-synced LFO,
beat-quantised trigger, bar/beat/16th time base.

**M2a's throwaway MIDI epoch bridge is M3's problem, and it should be replaced
rather than patched.** It samples the mach-versus-`Time<Fixed>` offset once at
first drain and never corrects drift, which is the same clock-alignment question
the phase estimator exists to answer — one clock discipline, not two.

M2c's panes are the debugging surface this milestone was sequenced to have. A
transport readout was deliberately left out of M2c, because inventing the
display before the thing it displays is backwards; M3 adds it, as a fourth
consumer of the same snapshot.

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

Four things this milestone inherits:

- **The unified-edges migration is this milestone's opening work, and it is
  already complete.** One `Edge` component, one `Inlets`/`Outlets` pair per
  node type, and one compiled order replace the three edge kinds and five
  node-declaration mechanisms this document described before
  2026-08-03 — a model M4's RON schema could not have been designed against
  without encoding all of it and then breaking it
  (`specs/2026-08-03-unified-edges-design.md` §10, "opens M4, before the RON
  schema is written"). Findings: `reports/2026-08-03-unified-edges-findings.md`.
  The sections this milestone revised are named in the Revision line at the
  top of this document.
- **The read-only inspector M2 asked for and did not build starts M4.** Its
  purpose was to find missing editor `TypeData` before an XL milestone depended
  on it, and that purpose is now sharper, not weaker: M4 decides how a params
  struct is read from and written back to a document, and an inspector is the
  same walk over the same registered type. M2c supplied the pane to put it in,
  so this is a small task rather than a milestone. Doing it after M4 commits to
  a serialisation shape gets the order backwards.
- **`EditorPos` is serialized here** (§2.10's node components are all authored
  data; a position is no different), which is what makes M2c's dragged positions
  survive a restart once M7 writes them back.
- **Event fan-in order across recompiles is undecided.** Ordering is
  deterministic for one compiled graph, established twice — by compiled rank and
  by a stable offset sort. Whether a recompile must preserve an earlier source
  order is a reload semantic, so it is M4's to answer, and answering it "no"
  silently means a hot reload can change which of two simultaneous notes wins.

*Exit:* a set can be authored by editing text with the app running.

### M5 — Visual runtime (L)

The real version of M1. M2b already made §2.10 real in miniature — `Geometry`,
the two edge kinds, `Grid`, `Displace`, `Mesh`, `Group`, `StandardMaterial`,
`Rgb` all exist and cook — so M5 is now **the rest of the node set plus GPU
residency**, not a from-scratch milestone: `Asset`, `Camera`, one node per light
type, `Scatter`, `CopyToPoints`, and the renderable marker. Runtime services
(`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`)
with owned invariants, glTF mesh instancing, curve-driven procedural animation,
physics if wanted. Where the fire-and-forget decoupling earns its keep: nodes
trigger, ECS systems continue.

**M1's visuals become nodes here, and the compute→draw gap closes here.** The
original M2 exit expected the graph to drive M1's point cloud and sprite layers;
M2b's scene node set drives its own mesh instead, so that promise moves to M5
intact. It carries M1's unamortised extraction with it — a per-frame CPU clone
and re-upload of the whole instance buffer, invisible at 50k points on an M4 and
not something to inherit unexamined at production cardinality.

**Two M2b invariants are safe by circumstance and must become safe by
construction before the node set grows:**

- The mesh upload gate fingerprints `P`'s `Arc` and point count only, so an
  operator that rewrites `N`, `uv` or indices while passing `P` through would
  produce a **silently stale mesh** — the one failure in that gate that does not
  fail toward wasted work. Nothing in today's operator set reaches it; "recompute
  normals" or "UV project" does. Fixing it means deciding what a cheap
  whole-`Geometry` identity is, which is exactly the decision GPU residency
  forces anyway, so the two should be made together.
- A material node never dirties its `Mesh` consumer, which is correct while a
  handle is created once at compile time and never recreated. Nothing enforces
  that. `Asset` and any node that reloads is where it stops being true.

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

### M7 — Editor (L, was XL)

**Smaller than it was, because M2c and M4 took its read half.** Pan/zoom, node
widgets keyed by `NodeId`, edge routing, hit-testing, the three-pane layout, the
live viewport, live edge values, and selection sync all exist; the read-only
inspector arrives at M4. What remains is the *write* half, which is where the
difficulty always was.

- Topology editing: drag-to-connect (deleted at M2c precisely so it would not
  ship as a lie), node creation from a palette, deletion, and param editing in
  the inspector.
- Writing dragged positions back to `EditorPos`, against M2c's seeding rule —
  a snapshot seeds a node's position once and never again, or the next frame
  snaps a dragged node home.
- Event-edge activity, which needs the per-edge ring buffer M2c refused to put
  in the hot tick for a read-only view. With the editor a first-class consumer
  and a write path already crossing into the tick, the trade changes.

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

- **Which geometry operators are GPU-resident** is still open, and M1 answered
  less than hoped. The shape is decided (§2.10: extract a dirty set, dispatch a
  render-graph subgraph in `Feeds` order, params through `ShaderParams`, arena
  stays on the CPU) and the criterion is decided (output size known before
  dispatch). M1 dispatched one compute operator from a dirty set and read its
  output back correctly, which is an **absence of a counterexample, not a
  confirmation** — one dispatch, one readback, one draw, all well inside
  conservative bounds, and the compute output never reached the draw. What
  remains open is unchanged: how far the criterion reaches, and whether mixed
  residency is tolerable or forces a rule that a `Feeds` chain is entirely one
  or the other. **Answer at M5**, where it now also decides what a cheap
  whole-`Geometry` identity is (§5, M5).

- **The cook gate is one bit per node, not one bit per reason.** M2b's finding,
  and it qualifies §2.11 directly: the engine gate decides *whether to call
  cook*, and a node owning an expensive resource still has to decide *whether to
  write it*. `Mesh` does this with a `GeometryFingerprint` over `P`'s `Arc`
  pointer and point count. What is open is whether that two-level structure is
  the design or a symptom — a per-reason gate, or a cheap whole-`Geometry`
  identity, would collapse it. Decide with GPU residency at M5, not before; the
  current arrangement is correct, just under-specified.

- ~~**A node whose tick depends on a cook from the same tick has no
  expression.**~~ — closed by the unified-edges design's §5 ("The tick").
  The two-phase split this question depended on — every node ticks, then
  every node cooks — is deleted: gather, tick and cook now happen per node,
  in the one compiled order (§2.11), and a `Product` edge is an ordinary
  dependency in that order like any other. A node can now declare a
  `Product<T>` inlet on another node's cooked output and have it correctly
  ordered before that node's tick runs — a hypothetical `Grid → PointCount →
  Displace.amount`, outputting `Grid`'s cooked point count as a signal, is
  expressible, because the edges order it rather than a global phase.

- **Fixed tick rate value** is unchosen, and M2b's measurements did not choose
  it. The mechanism is settled (`Time::<Fixed>::from_hz`). The data so far: the
  demo graph's `graph_tick` costs 624 ns with the gate closed and 13.07 µs with
  two CPU cooks open, or 0.16% of a 120 Hz budget at worst. That is one 48×48
  grid with no renderer, no live MIDI callback and no M5 residency traffic — not
  the graph the number should be chosen against. Two measurement rules earned
  the hard way and worth keeping: **time `graph_tick` directly**, because an
  `App::update()` fixture floor of ~40 µs dwarfs the signal, and **run with
  `--test-threads=1`**, because parallel test execution inflates timings by up
  to 40%. M2a's 2.226 µs/tick figure is not comparable to either and should not
  be cited.
- ~~**Reflect's ergonomics under a real node set**~~ — resolved for params,
  ports and slots by M2a and M2b. Nothing resisted `Reflect`: `Event<T>` and
  `Slot<T>` both derive it with a `PhantomData<fn() -> T>` field
  `#[reflect(ignore)]`d, and no hand-written schema was needed. Two rules came
  out of it — use `reflect_clone()` rather than `to_dynamic()` for any value
  that must later downcast to its concrete type, or reflected enums silently
  become dynamic proxies; and import `ReflectDefault` from the `bevy_reflect`
  prelude. The narrower part is still open: **editor `TypeData` is unexercised**,
  because the throwaway inspector M2 was meant to build was never built. That is
  the reason it now starts M4 (§5).
- ~~**State lives in two places**~~ — resolved by §2.2. Node state is components
  on the node entity, so state lives only in the world, and snapshot/restore
  becomes a question about the world rather than a per-node protocol. The open
  part is narrower: which components are performance state worth restoring and
  which are derived caches. Still revisit before M7, but as a labelling problem.
