# Sway — Design

**Date:** 2026-07-25
**Status:** In implementation — M0, M1, M1b, M2a, M2b, M2c, M3 complete; the
wire model has replaced the graph engine; M4 complete, M5 next
**Revision:** graph engine builds on Bevy's non-rendering subcrates (§2.2–§2.7, §3)
**Revision:** scene composition is expressed in the graph, Houdini/USD-shaped (§2.10)
**Revision (2026-08-02):** §5 and §7 reconciled against what was actually built.
**Revision (2026-08-03):** unified edges — one `Edge`, one arena, one compiled
order, replacing the three edge kinds and five node-declaration mechanisms
(§2.4, §2.5, §2.10, §2.11, §5, §7).
**Revision (2026-08-06): wires.** There are no nodes, no edge entities, no port
arena and no cook gate. A connection is a Bevy `Relationship` component whose
type carries what it does; entities and components *are* the graph. This
rewrites §2.2, §2.4, §2.5, §2.10, §2.11 and touches §2.1, §2.3, §2.6, §2.8,
§2.9, §3, §4, §5, §7. Design: `specs/2026-08-05-wires-design.md`; the
implementation landed across `0a1a8a0..d97e0c5` and is the authority where this
document and the code disagree.
M2 shipped as M2a + M2b, an unplanned M2c added the editor's first real views,
and each completed milestone now carries the debt it did not discharge. Where
implementation contradicted the architecture the correction lives in the
milestone's findings report and is named in §7.

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
other VJs later. Project format and the component/wire API get real design
attention;
onboarding, docs, and distribution are deferred.

## 2. Architecture

Three layers.

**Engine** — connection types, evaluation order, and the walk that runs it.
Knows nothing about MIDI or pixels. It does know about Bevy — not the renderer,
but the ECS, reflection, time, and asset subcrates, which it uses as its own
substrate rather than reimplementing them (§2.9). Since the wire revision it is
*less* than that substrate rather than a layer on top of it: typing, direction,
fan-out and lifetime are the ECS's, and the engine adds only ordering.

**Runtime** — the Bevy app: ECS world, render pipelines, physics, animation
systems, plus a deliberately *exposed service surface*. Runs per-frame whether or
not a graph exists.

**Wires and behaviours** — the two things registered with the engine, and the
whole of what bridges the two layers (§2.2).

### 2.1 The central decoupling

The graph does two things, and only two. It **declares structure** — what exists
in the scene and how it composes (§2.10) — and it **fires** — "burst here",
"retarget that colour", "start clip 3". What it does not do is drive the world
frame by frame. Structure is cooked when something changes, not on every tick,
and a fired event belongs to ECS systems from the moment it lands: an animation
triggered from the graph keeps running with no further involvement from it.

This is why the runtime stands alone, why the graph can tick at a rate unrelated
to the render loop, and why the graph does not need to be fast.

Corollary principle: the graph is the nervous system, the Bevy world is the body.
Low-cardinality global signals (an LFO, an envelope, a CC) travel between
components on entities. High-cardinality data (10k points, rigid bodies,
particle lifetimes) lives in the ECS as components, parameterised by the graph.
Since the wire revision the distinction is one of cardinality alone, not of
storage: both are components, and a signal is simply a small one. Scene
composition does not breach this either — geometry is a component on an entity,
never a value carried by a connection (§2.10). **Physics is never wired.**

### 2.2 The wire contract

There is no node type and no node instance. **An entity is a graph vertex
because it carries components, and a connection is a component too.**

```rust
pub trait Wire: Relationship {
    type Source: Component;                          // read on the producer
    type Target: Component<Mutability = Mutable>;    // written on the consumer
    const NAME: &'static str;                        // the editor's and the file's key

    fn propagate(src: &Self::Source, dst: Mut<Self::Target>);
}

#[derive(Component)]
#[relationship(relationship_target = DrivesTranslationY)]
pub struct TranslationYFrom(#[entities] pub Entity);

impl Wire for TranslationYFrom {
    type Source = FloatOut;
    type Target = Transform;
    const NAME: &'static str = "translation.y";

    fn propagate(src: &FloatOut, dst: Mut<Transform>) {
        dst.map_unchanged(|t| &mut t.translation.y).set_if_neq(src.0);
    }
}
```

The relationship component lives on the **consumer** and names the producer;
Bevy's `RelationshipTarget` on the producer collects its consumers. **Outlets
are components** — an entity has a `f32` outlet because it has `FloatOut`, and
two outlets of one type are two newtypes. **Inlets are wire types** — an entity
has a `translation.y` inlet because it has a `Transform` and that wire's
`Target` is `Transform`.

**Behaviours** are the second and last thing registered. Most computation is not
a connection, and most of it does not belong to the graph at all:

| What the output depends on | Where it runs |
|---|---|
| Only external state — `Time`, MIDI, input | An ordinary Bevy system, before the tick |
| Nothing; it only consumes — mesh upload, material rebuild | An ordinary Bevy system on `Changed<T>` |
| **A wired inlet, in the same tick** | A **behaviour**, `fn(&mut World, Entity, &TickCtx)`, placed in the order |

Only the third case needs the graph, and it needs it for something no ordinary
system can supply: a position determined by data flow rather than by a fixed
slot in the schedule. An LFO whose amplitude is driven by another LFO must
compute between the two propagations. Registration is per component type, so a
type that *can* be driven is a behaviour even for instances that happen not to
be.

**Hierarchy costs one impl**, because `ChildOf` is already a `Relationship`:
`Source` and `Target` are both `Transform`, and `propagate` is empty because a
structural connection's existence *is* its state. Nothing in `sway-graph`
inserts `ChildOf`; authoring does, and Bevy's hooks maintain `Children`. There
is no `Spatial` marker, no parenting pass, and no compile step that emits it.

Instance lifecycle is the ECS's. Despawning a consumer takes its wires with it,
because the wire component is on the consumer. Despawning a producer is the one
case the ECS does not clean up — the consumer's wire component is left naming a
dead entity, and propagation skips it on the spot (§2.5). Component hooks remain
available for anything a component type owns in the world; the engine knows
nothing about them.

`TickCtx` carries only what is specific to this tick — its duration, its start,
and its index. Wall time comes from `Time<Real>` and beat position from
`Time<Transport>` (§2.7). Behaviours derive time-varying values from absolute
time rather than accumulating per tick, so they stay correct across pauses,
tempo changes, and missed ticks.

### 2.3 The exposed runtime surface

Mechanically a behaviour receives `&mut World` and can touch anything. By
convention it goes through registered service resources — for v1 roughly
`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`.
Each is a small facade owning its own invariants.

The discipline matters: it is what keeps "a behaviour can touch anything" from
becoming "anything can break anything", and it keeps behaviours testable. A
wire's `propagate` is narrower still — it sees one source component and one
target component, and cannot reach the world at all.

Where the interaction is genuinely fire-and-forget — "burst here", "start clip
3" — the facade call is an **observer trigger** rather than a method. This is
what observers are for, and it inverts the dependency: a behaviour emits an
intent without linking the system that services it, so it and the runtime
feature it drives can be developed and tested apart. Observers are used only in
this direction, graph → world. They are deliberately **not** used to carry
connections (§2.4).

### 2.4 What the ECS enforces, and what is left over

There is no port arena, no type-erasure, no `TypeId` comparison, no direction
check, no inlet-already-connected check and no fan-out rule anywhere in the
engine. Every one of them is a property of Bevy's `Relationship`:

| Invariant | Enforced by |
|---|---|
| An inlet has at most one source | One component per type per entity |
| Value types match | `Wire::Source` / `Wire::Target` — Rust, at compile time |
| Direction | Which side holds the relationship component |
| Fan-out from one outlet to many inlets | The `RelationshipTarget` collection |
| Rewiring replaces the old source | `Relationship::on_insert` evicts the previous one |
| A despawned consumer takes its wires with it | The wire component lives on the consumer |
| A self-connection is rejected | Bevy removes self-referential relationships |

Each of these is pinned by a characterization test against the pinned Bevy
(`crates/sway-graph/tests/relationship_semantics.rs`), because the engine now
*depends* on them rather than implementing them.

The requirement that made the old design type-erase — a live set must never die
on a type mismatch — is met more strongly than before: an illegal connection is
not a load error, it is a program that does not compile. What remains a runtime
question is only whether the *entities* still hold the components a wire names,
and that is answered by skipping (§2.5) and reported as a diagnostic.

**A type-selector param is still a smell; make it a component type.** The
argument survives the revision intact, and now costs even less to honour. There
is no `Material` component with a kind dropdown — there is one component and one
wire per material type, generated by a blanket impl with one registration call
each. Lights are the same: `DirectionalLight`, `PointLight`, `SpotLight`, which
are already Bevy's own component types. Changing a material's type is replacing
a component, which honestly invalidates the wires attached to it.

Two things the old model expressed and this one does not yet:

- **Events.** The value/event split is still required for the reason it always
  was: without it there is no way to distinguish "CC is 0" from "no CC arrived",
  and note timing collapses to tick granularity. Under wires an event inlet is
  a wire whose `Target` is a per-tick buffer component, plus a clearing policy —
  who drains it, and when. That policy is unbuilt and is named in §7.
  Observers remain the wrong tool despite the surface resemblance: they fire
  immediately and recursively, which cannot be reconciled with topologically
  ordered evaluation, and they carry no notion of buffering several occurrences
  with sub-tick offsets to be drained at a known point.
- **Variadic inlets.** A `Vec` of sources has no single-relationship
  representation, so `Merge` and `Sum` have none either. `Math` and `Switch`
  stay binary and compose regardless — `Switch(s1, Switch(s2, a, b), c)` covers
  the three-way case — so nothing is blocked today. Also §7.

Both were expressible in the arena model and are not expressible now; that is a
real cost of the revision, paid deliberately for the seven rows in the table
above.

### 2.5 Ordering and the rebuild

Nothing is compiled. One derived artifact exists, and it is a flat list:

```rust
#[derive(Resource, Default)]
pub struct GraphOrder { pub steps: Vec<Step> }

pub enum Step {
    Propagate { run: PropagateFn, src: Entity, dst: Entity, wire: &'static str },
    Run       { run: BehaviourFn, entity: Entity },
}
```

A rebuild collects every instance of every registered wire type into links,
Kahn-sorts the **entities** they connect, and emits, per entity in that order,
its inbound propagations followed by its behaviours. That per-entity ordering is
the whole reason a chain resolves within one tick.

**A step carries its own function pointer.** The list is heterogeneous and
`Wire::propagate` is generic over its associated types, so it is not
object-safe; monomorphising `propagate_of::<W>` once at registration is what
makes one `Vec` possible. There is no wire id, no registry index and no
indirection on the tick path — the registries exist for the rebuild, the editor
and the project format, and the tick never reads them. A step stays plain `Copy`
data rather than a boxed closure so the order remains inspectable: tests assert
on it and the editor can show it.

**The rebuild is authoring-time work.** `GraphOrder` is rebuilt when a
`TopologyDirty` flag is set, and the flag starts set. Per-wire-type watch
systems notice insertion and removal and set it, and they are registered into a
system set gated on an `Authoring` resource — **absent from a show build**, so a
show pays one bool read per tick. Authoring is therefore plain ECS insertion;
there is no `connect` API to route changes through and no discipline the show
path has to honour. If whatever loads a project forgets to set the flag the step
list stays empty and the graph is visibly inert, which is a loud failure rather
than a subtle one.

**A cycle never stops the render.** The sort emits the acyclic part in
topological order and appends cycle members in entity order, where they read the
previous tick's value. This is a deliberate reversal of the old position that
the compiler rejects cycles: with no load-time gate left between a text edit and
a running show, refusing to run is the worse failure. Cycles, along with a
producer that lacks the wire's `Source` and a consumer that lacks its `Target`,
land in a `GraphDiagnostics` resource for the editor to render — computed at
rebuild, so a live show pays nothing for them.

Order is deterministic: ties break by ascending entity index. That costs a small
piece of vigilance, because `Entity`'s `Ord` in the pinned Bevy is *descending*
in raw index — its niche encoding stores the complement — so the heap and the
cycle sort both compensate, and a test pins the encoding so a Bevy upgrade fails
loudly rather than silently reordering.

**Vertices are entities, not (entity, component) pairs.** Two unrelated
components on one entity flowing in opposite directions therefore read as a
cycle even though nothing circles. This is accepted: the case is rare and
splitting the entity in two resolves it. §7 carries it.

**The topological sort stays ours.** Bevy's `ScheduleGraph` already does a
cached topological sort with cycle detection and would appear to be free, but
using it would mean one system per graph *instance*, against the grain of a
scheduler built to order systems per *type* — and its errors would name systems
rather than entities, in direct conflict with §4's requirement that a failure
produce a clear, author-attributable message. Kahn's algorithm is a few dozen
lines with diagnostics we control.

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

**Behaviours are not per-type systems, and this is the one place the ECS is
refused.** The batched version is seductive — an LFO as a system over
`Query<(&Lfo, &mut FloatOut)>`, no dispatch, good cache behaviour. It does not
survive ordering. Bevy schedules *systems*; the order here is over *instances*.
A graph containing `LFO → Math → LFO` has no expression as system ordering
constraints. The escapes are one-tick latency on the connections that run
backwards against system order, or running the whole system set once per DAG
level. The first is still deterministic but no longer dataflow: a value arrives
a tick late, and which connections are affected depends on system registration
order rather than on anything the author can see — the resulting timing bugs
would be invisible in the graph and reproducible only by accident. The second
pays a full schedule traversal per level to recover what a flat loop already
had. A serial walk of the step list is correct, is fast enough for a control
graph of this cardinality, and keeps the semantics the author expects when they
draw a wire.

This is also exactly why behaviours exist and are rare (§2.2): a component whose
output does not depend on a wired inlet has no reason to be in the order at all,
and stays an ordinary Bevy system where the ECS is at its best.

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
tick is whatever the phase estimator says. A tempo-synced behaviour reads
`Res<Time<Transport>>` and is otherwise ordinary; stop is a clock that
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
socket, no schema manifest, no serialisation boundary. The editor reads the
world directly — a box per entity, inlets from the registered wire types whose
`Target` that entity has, outlets from the `Source` components it has, a line
per wire instance, and live values read straight off the components. The
inspector is a walk over the same registered types the runtime uses, so there is
no second description of anything to keep in sync.

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
structure with stable identity per entity and connection; selection, drag, collapse
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
| Connection storage, direction, single-source, fan-out, rewire | `bevy_ecs` — `Relationship` / `RelationshipTarget` |
| Entity lifecycle and cascade delete of a consumer's wires | `bevy_ecs` — entities, hooks, relationships |
| Scene hierarchy, transform composition | `bevy_transform` — `ChildOf`, `GlobalTransform`, propagation |
| Value typing of a connection | **rustc** — `Wire::Source` / `Wire::Target` |
| Dirty propagation | `bevy_ecs` — `Changed<T>`, given §2.11's no-equal-write rule |
| Editor metadata, and the project format's payloads | `bevy_reflect` — `TypeRegistry`, `TypeData`, field attributes |
| Component (de)serialisation | `bevy_reflect` serde |
| Tick rate, accumulator, catch-up | `bevy_time` — `Time<Fixed>` |
| Beat time, pause, tempo scaling | `bevy_time` — `Time<Transport>` |
| Project loading, file watching, hot reload | `bevy_asset` — `AssetLoader`, `AssetEvent` |
| Registration surface, schedule placement | `bevy_app` |
| Topological order and its determinism | **ours** |
| Rebuild diagnostics, and the project document's own errors | **ours** |
| The step list and the walk over it | **ours** |
| `Geometry` attribute tables and the operators over them | **ours** |
| Transport phase estimation from 24 ppqn | **ours** |

The line is consistent, and the wire revision moved it decisively toward Bevy:
Bevy owns storage, identity, connection semantics, metadata and time; rustc owns
typing; we own ordering, the document, and the messages an author sees.

The earlier framing of this boundary — "`sway-graph` depends on `bevy_ecs` only"
— was already untrue in this document, since registration took `&mut App` and
the tick lived in `FixedUpdate`, both `bevy_app`. Drawing the line at
"everything except the renderer" is both honest and considerably more useful.

The cost is that a Bevy upgrade now touches the engine layer rather than only
the runtime. Given §2.8 already pins the Bevy version exactly to hold the
wgpu/winit alignment, this adds coordination but no new class of risk. The
testing argument in the original §3 survives intact: golden-trace tests build a
minimal `App` with no rendering, which is as cheap as building a bare `World`.

### 2.10 Scene composition

The scene is built by the graph, not loaded beside it. Camera, lights, meshes,
groupings, materials and transforms are all authored in the graph, and there is
no base scene file it layers over: content comes from Blender through `Asset`
entities at the leaves, composition comes from the wiring. Ownership is
total, which is what makes teardown and reload answerable.

The model is Houdini's and USD's rather than a node-per-object scene editor. The
distinction is load-bearing: **operators act on streams, so cardinality lives in
the data, not the operator count.** Thirty-two satellites are a `Scatter` and a
`CopyToPoints`, not thirty-two entities.

**A graph entity is a scene entity.** §2.2 makes every vertex an ordinary
entity; a scene entity is one that additionally carries `Transform` and
`Geometry`. There is no handle, no mapping table, no reconcile step, and
selecting a box in the editor selects the object it makes, because they are the
same entity.

**Status after the wire revision.** The model below is the intent and is
unchanged by it, but its mechanism is not built. `Geometry`, its `Arc`-backed
attribute tables and the operators over them survive in `sway-geo` as pure,
tested functions; what is gone is the machinery that carried geometry from one
entity to another — `Product<T>`, `Feeds`, the cook gate and the two-pass tick.
Geometry and asset flow is named work, and §5's M5 owns it. The rest of this
section reads as the target, with the two subsections below marking what
changed.

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

#### One mechanism, three kinds of connection

There is one mechanism — a wire (§2.2) — and what a connection means is a
question of which wire type it is:

| Connection | Wire | Enters the order |
|---|---|---|
| Parenting | `ChildOf`, `Source = Target = Transform`, empty `propagate` | yes, propagating nothing |
| Geometry flow | a wire whose `Source` is `Geometry`, plus a behaviour that cooks — **unbuilt, M5** | yes |
| Driving a value | a value wire: an outlet component → a field of the consumer's component | yes |

Parenting composes transforms through Bevy's own `ChildOf` and `Children`, and
it sits in the order like anything else — harmless, because its `propagate` is
empty and a parent reads nothing from a child. The old rule that parenting had
to be *excluded* from the sort left with the two-DAG model that needed it.

Geometry flow is Houdini's SOP wire and is the piece the wire revision left
unbuilt. Its shape follows from `propagate`'s signature: propagation sees only
the source and target components, never the consumer's own parameters, so an
operator cannot be a wire alone. It is a wire that delivers the upstream
`Geometry` plus a behaviour on the operator's own component that cooks with its
parameters in hand. M5 decides it; §7 records it as open.

One direction note, because it reads backwards from every other wire: `ChildOf`
lives on the **child** and names the parent, so a parenting connection's
*source* is the parent and its *target* is the child, while every value wire
lives on the consumer. That is Bevy's choice rather than ours, and it is worth
stating once instead of rediscovering.

**The rule that tells an author which wire they want:** object-level composition
— place, group, instance — is `ChildOf`. Element-level operations — scatter,
noise, displace — are geometry wires. Driving a value — colour, rotation,
intensity — is a value wire, independently of which of the first two the same
entity also uses.

```
Grid ────────────── geo ─────→ Scatter
Scatter ─────────── geo ─────→ CopyToPoints
Asset("sat.glb") ── proto ───→ CopyToPoints
CopyToPoints ────── geo ─────→ Mesh("sats")
StandardMaterial ── material → Mesh("sats")
Mesh("sats") ────── ChildOf ─→ rig ── ChildOf ─→ root
Asset("hero.glb") ─ ChildOf ─→ rig
DirectionalLight("key"), Camera ─ ChildOf → root

MidiNote ──> Envelope ─┬─→ StandardMaterial("shiny").emissive
                       └─→ hero.scale
MidiCC 74 ─> Smooth ────→ DirectionalLight("key").illuminance
LFO(1/2 bar) ───────────→ rig.rotation.y
```

`Grid`, `Scatter` and `CopyToPoints` carry `Geometry` and no `Transform`; they
are operators and sit outside the scene tree entirely. `Mesh` carries
`Transform`, `Mesh3d` and `MeshMaterial3d` and is in it. Which components an
entity has *is* the distinction, visible the ECS-native way — and under wires
that is no longer only how it reads but how it is enforced, since a wire may
only land on an entity that has its `Target`. `CopyToPoints` produces one buffer
of instances — the scattered points never individuate into entities.

**`Mesh` is where a geometry chain enters the scene tree**, and it is the only
place that happens other than `Asset`, which imports a glTF subtree directly.
Naming that boundary is most of what an author needs to understand about the two
chain kinds.

#### Materials are wired, not assigned

A material entity owns a `Handle<M>` and a `Mesh` entity takes it through a
wire. Nothing assigns a material to something else, and therefore nothing
reaches into an entity it does not own — the ownership rule of §2.2 holds
without exception.

There is one component and one wire type per material type, not one `Material`
with a type param, for the reason given in §2.4.

The second effect matters more in practice. Material sharing becomes a visible
topology fact rather than hidden aliasing: one material entity feeding three
meshes is obviously shared, and three material entities are obviously not. The
failure this designs out is real and nasty — with assignment-style materials,
driving one object's emissive silently drives every object sharing the handle,
and the graph gives no indication. Here, wanting independent emissive means
drawing a second material entity, which is exactly the thought the author should
be having.

Structural connections are consequently **named and typed** — `points`, `proto`,
`geo`, `material` are each a distinct wire type — and the check that stops a
material from filling a geometry inlet is no longer a pass at all. A wire whose
`Source` is `Handle<StandardMaterial>` cannot be created against an entity that
has no such component, and it cannot be *written* against a `Geometry` target,
because `MaterialFrom` and `GeometryFrom` are different types.

#### Two things this gets for free

**Intermediate results are inspectable.** Only entities marked renderable draw,
so cooked geometry on operator entities sits in the world undrawn and available.
That is Houdini's per-node display flag, obtained by toggling a component, and
it is the single most useful debugging affordance in this class of tool.

**`Changed<T>` is the cook invalidation.** An operator recomputes when its input
geometry or its own parameters changed. There is no gate resource, no
`last_product_ticks`, no `COOKS` flag and no cache to write — a plain
`Changed<Geometry>` system outside the graph is the whole mechanism, bought by
the one rule §2.11 states: a wire must never write an equal value. The old
explicit change-tick comparison existed because the graph tick ran every tick
and would eat the flag; behaviours no longer own cooking, so the ordinary filter
is correct again. Structure needs no cooking at all — a wire writes `Transform`
when its source moves and Bevy's propagation does the rest, per frame, where
that work belongs.

#### Geometry residency — direction, not settled design

Geometry operators should run as compute shaders wherever the work allows it.
`Geometry`'s planar attribute layout was chosen partly for this: it is already
what the GPU wants. Note that the toolkit is compute shaders, vertex pulling
from storage buffers, and instancing — **there are no geometry shaders**, since
wgpu and WebGPU have no such stage and Metal never had one.

The structural consequence is larger than it first appears. Bevy's render world
is a separate world extracted once per frame, while the graph ticks in
`FixedUpdate` on the main world, so a cook cannot dispatch and read back
synchronously — it can only enqueue. **GPU cooking therefore happens entirely
outside the graph**, which the wire model already requires of it: the graph is
main-world only, and touches no asset and no GPU resource.

```
FixedUpdate   graph tick: propagate values, run behaviours — nothing else
Update        Changed<T> systems queue what the propagation dirtied
Extract       dirty set + ShaderParams → render world
Render        a render-graph subgraph dispatches compute in wire order;
              results stay in GPU buffers and are consumed by the draw
```

Mostly this is a gain. Dispatch coalesces per frame, so a value changing across
three ticks costs one dispatch, and cook cost genuinely leaves the frame's
critical path — the async escape hatch of §7 arriving by a different road.

**Signal values stay on the CPU.** They are small components that CPU-side
behaviours consume; making them GPU-resident would turn every LFO write into a
GPU write and force readback for anything reading its own inputs. The narrower
split: an entity feeding a compute op writes its effective values into a
`ShaderParams` component (`#[derive(ShaderType)]`) on itself, which extraction
uploads. That is Bevy's existing material-uniform path, already paved.
`Geometry` becomes a handle to a GPU buffer rather than CPU arrays, and the
invalidation above is unaffected, since it keys on the component's change tick
rather than its contents.

The line for which operators can go to the GPU is **whether output size is known
before dispatch**. Element-wise work — noise, displace, transform, colour — and
`Scatter` at fixed count are clean. `CopyToPoints` is often no dispatch at all:
bind the point buffer as instance data and let the draw expand it. Variable
output size (delete-by-threshold, fracture) needs atomic counters and indirect
dispatch, and is later work. Anything rewriting topology or needing adjacency,
such as subdivide or fuse, stays on the CPU.

This qualifies one of the free wins above: with geometry resident on the GPU,
inspecting an intermediate result needs an explicit async readback rather than a
component read. Still worth having, but editor-requested and a frame late, not
free.

**The hazard to design for now** is mixed residency. A CPU operator wedged
between two GPU operators forces a readback and a stall, and in the graph it
looks identical to a chain that stays resident. Same shape as cook cost, so the
same position (§7): the tool reports rather than polices. Residency is shown on
the box — border, badge — so a ping-pong is something an author sees rather than
something they profile.

`sway-geo` consequently sits on the render side and depends on `bevy_render`.
§2.9's rule survives untouched: it constrains `sway-graph`, not the crates that
define components and behaviours.

#### What this deletes

The commit and reconcile stage, a `SceneNode` port type, bind points, name
resolution against an external scene file, and the whole sink-node set. A signal
wires directly into the component of the entity that builds the thing, so a
target cannot go stale.

#### What it gives up

Stream rewriting for object-level operations. In Houdini, `Transform →
Subdivide → Scatter` are all operators on one geometry stream. Here `Transform`
is a scene component rather than a data operator, so transforming points and
then scattering on the result is a geometry chain, not a parenting chain. The
gain is Bevy's transform propagation for free; the loss is that the two chains
are different chains and the author has to know which is which. Hence the rule
above.

### 2.11 Graph state reaching the ECS

Almost nothing of what "propagation" usually means exists here, and the wire
revision removed what was left. Values, geometry and transforms are components
on ordinary entities, so nothing crosses a boundary between two
representations — a wire writes the real component, and Bevy's machinery takes
over from there.

```
PreUpdate     MIDI IO thread -> timestamped buffers
              (authoring builds only) watch systems mark the topology dirty
FixedUpdate   (0..n times per frame)
                advance Time<Transport> from the phase estimator
                rebuild the order, if the topology is dirty
                graph tick - one exclusive system, one flat step list:
                  Propagate { src, dst }  write one field of one component
                  Run { entity }          a behaviour, with &mut World
Update        runtime systems, incl. every Changed<T> reaction the tick caused
PostUpdate    transform propagation, visibility
Extract       dirty set + ShaderParams -> render world
Render        compute subgraph, then the draw
```

Because the tick is an exclusive system holding `&mut World`, **writes are
immediate**: a later step sees an earlier one's component writes within the same
tick. Routing through `Commands` would introduce a flush boundary and a tick of
lag, which is a concrete payoff of the §2.6 choice.

#### The one discipline: never write an equal value

`get_mut` marks `Changed<Target>` unconditionally, so a wire that writes every
tick would defeat change detection for everything downstream — and since the
cook gate is gone, `Changed<T>` is now the *whole* dirty story. **`propagate` is
therefore responsible for not writing an equal value.** Bevy's own API suffices:
`Mut::map_unchanged` narrows to a field without marking anything, and
`set_if_neq` then marks only on a real change, so the engine adds no helper of
its own.

This is the single rule a wire author can silently break, and breaking it is
invisible — the values stay correct while everything downstream re-runs every
tick. It has its own test, and any new wire type needs one.

Assets need more care than components for the same reason: `Assets::get_mut`
marks the asset changed by the act of calling it, so a material write is `get`,
compare, and only then `get_mut`. That write does not happen in the graph at
all (the graph touches no assets); it is a `Changed<T>` system downstream of it.

#### Unconnected values

A wire writes into the real component, which is also where the authored value
lives. A connected wire overwrites it each tick; on disconnect the field keeps
whatever arrived last, and the author edits onward from there. There is no
shadow copy, no prefill pass and no connected-slot mask — the graph stores no
per-connection state whatsoever.

The cost is explicit and is a reversal of the previous position: authoring a
value, connecting, then disconnecting does not restore what was typed.
Restoring it is undo-shaped editor policy, not engine state. What the old rule
bought — the file can never bake in whatever the LFO happened to be at — is
bought differently now: the project document is the authored value, and a
connected field is not written back to it (§5, M4).

One interaction to keep in mind at M5: continuous driving plus render
interpolation both target `Transform`, and they cannot both own it. The wire
writes a `DrivenTransform` carrying previous and next; the per-frame
interpolator writes `Transform`.

#### Events

A behaviour holds `&mut World` and may `trigger` an observer, which runs
immediately and synchronously, so its effects are visible to later steps in the
same tick. An observer may spawn, despawn and mutate components freely. What it
must not do is insert or remove a wire component: that changes the topology
mid-walk, against a step list built before the tick started. In an authoring
build the watch systems would notice and rebuild next frame; in a show build
nothing would, and the graph would silently keep running the old order.

#### Then nothing

After the tick no graph code runs. Transform propagation, visibility and render
extraction are Bevy's, reading components the graph happened to write. Between
ticks — and there may be several frames between them — the world keeps animating
on its own. That is §2.1 as a mechanism rather than a principle. The editor
likewise reads rather than receives: values come from components, structure from
relationship components, evaluation order from `GraphOrder`, problems from
`GraphDiagnostics`, with nothing pushed to it.

#### Structural change is a separate, rarer path

Spawning, despawning and rewiring never happen during a tick. On load, reload or
an editor edit, entities are spawned or despawned and wire components inserted
or removed as ordinary ECS operations; the topology flag is set; and the next
`FixedUpdate` rebuilds the order before ticking.

Reload **reconciles by document id** (§5, M4): an entity present in both the old
and the new document keeps its `Entity`, its editor identity and any components
a runtime system attached to it; removed entities despawn, taking their wires
and their children; added ones spawn. This is what keeps hot reload from
resetting the world on every save — the difference between authoring with the
app running and restarting it on every edit. The wire model makes it cheaper
than it was: a behaviour derives from absolute time rather than accumulating, so
there is little per-instance state left for a reconcile to preserve.

## 3. Crate layout

```
sway-gpu        wgpu instance/device/queue creation — the single place the
                bevy↔vello version coupling lives
sway-graph      engine: the Wire trait, the wire and behaviour registries, the
                topological order and its rebuild, the tick walk, rebuild
                diagnostics, the transport clock, project format (M4)
sway-nodes      the built-in components, wires and behaviours
sway-geo        Geometry attribute tables and the operators over them; sits on
                the render side, depends on bevy_render (§2.10)
sway-runtime    headless Bevy app rendering to a texture; services, pipelines
sway-midi       MIDI IO thread + transport clock estimator
sway-editor     masonry UI; links the runtime directly
sway-app        host: owns winit, creates the device, runs editor shell or
                show presenter
```

**`sway-schema` is gone.** It was to hold port types, node schema, editor
metadata, and the project format. Rust's type system supplies connection typing,
`bevy_reflect` supplies the editor metadata and the document's payloads, and
what remains — two registries and the document shape — is small enough that a
separate crate would exist only to preserve a boundary nothing needs. The editor
links `sway-graph` regardless.

`sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`,
`bevy_time` and `bevy_transform`, with `bevy_asset` joining at M4 — not on
`bevy`, and specifically not on `bevy_render`. `bevy_transform` is on the list
because `ChildOf`'s wire impl and the hierarchy are the engine's business;
it is headless and pulls no renderer. Its manifest is the only place this
constraint is enforced, and it says so. Making the engine generic over a context
type was considered and rejected: a minimal headless `App` is cheap to construct
in tests, so the abstraction would buy nothing real.

## 4. Testing strategy

- **Graph engine** — golden-trace tests. A recorded MIDI trace plus a fixed tick
  rate produces bit-identical output; assert against stored expectations.
- **Transport** — recorded clock traces including tempo changes and dropouts.
- **Bevy's relationship semantics** — characterization tests, because §2.4 now
  *depends* on seven behaviours rather than implementing them. Non-cascading
  despawn is the one that matters most: if `LINKED_SPAWN` were not gated as
  assumed, despawning an LFO would despawn everything it drives.
- **Order** — the sort's determinism and its tie-breaking, cycle members
  appended rather than rejected, and, above all, that a chain resolves in
  **one tick**. A one-tick assertion is what distinguishes a correct order from
  a schedule that merely converges over several frames, and it is the claim the
  whole design turns on.
- **Change detection** — every wire type needs a test that a second tick
  carrying the same value leaves `Changed<Target>` false. §2.11's rule is the
  one an author can silently break, and a broken one is a performance bug no
  output assertion catches.
- **Cooking** — when geometry flow returns at M5: a cook is a pure function of
  its inputs, so assert on the resulting `Geometry` attributes directly, and
  assert the negative — an unrelated change recomputes nothing.
- **Project document** — round-trip (world → document → world) for every
  authorable component and wire; malformed input reports rather than panics; and
  a reload against an edited copy asserts that surviving entities kept their
  `Entity` and their runtime-attached components, that removed ones took their
  wires and children with them, and that a syntax error leaves the running world
  untouched.
- **Runtime** — replay recorded MIDI traces through a graph into a headless world;
  assert on ECS state and service calls rather than pixels.
- **Rendering** — no pixel-diff tests. Verified by eye.

## 5. Roadmap

Sizes are relative, not calendar. Ordering follows two rules: get one end-to-end
path working before deepening any layer, and pull genuinely unknown work early.

**Status at 2026-08-09.** M0, M1, M1b, M2a, M2b, M2c, M3 and M4 are complete,
and the **wire model** has since replaced the engine those milestones built;
M5 is next. A completed milestone's plan is superseded by its findings report, which
is linked below and is the authority on what was actually built. Where one left
debt it carries a *Carried forward* line saying so, and the milestone that
inherits it says so too — the point is that debt is visible in the roadmap
rather than only in a report nobody re-reads.

Two entries below describe work that was subsequently deleted. **M2a and M2b are
left in place rather than rewritten**, because what they proved — that a
topological order over instances is the right shape, that `Arc`-backed geometry
sharing pays, that authored-versus-driven is a semantic every node assumes —
survived the engine that carried it. What they built did not.

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

### M2a — Graph engine core (L) — **complete, engine since replaced**

*Superseded by the wire model (below): every mechanism named in this entry has
been deleted. What survived is the eight nodes' pure logic and their golden
traces, and the finding that a topological order over instances is the right
shape.*

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

### M2b — Structure edges, geometry, the cook gate (L) — **complete, engine since replaced**

*Superseded by the wire model (below): the edge kinds, the structure pass and the
cook gate are gone. `Geometry` and its `Arc`-backed tables survive in `sway-geo`
as pure code, and the sharing measurement below still stands.*

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
existed to find missing editor `TypeData` early. That risk is still undischarged
and is M4's opening work below — two milestones later than intended.

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
and it belongs at M7 with the rest of the editor's write paths — now behind
`Events<T>` having any representation at all (§7). Dragged positions do not
persist (M4 serializes them, M7 writes them back), and display names are
shortened from `type_name` until M4's registered short names replace the
shortening outright.

The panes survived the wire migration by being rewritten against it: the
snapshot reads entities, registered wire types and `GraphOrder` rather than
nodes, edges and a compiled graph. The layout, the identity model and the
read-only discipline are unchanged, which is the useful evidence — the views
were coupled to the world, not to the engine.

### M3 — Transport and beat lock (M) — **complete**

MIDI clock ingestion at 24 ppqn, drift-corrected phase estimator,
start/stop/continue, tempo tracking. Transport-aware nodes: tempo-synced LFO,
beat-quantised trigger, bar/beat/16th time base.

*Outcome* (`reports/2026-08-04-m3-transport-findings.md`): a 48-pulse
least-squares estimator follows the recorded 120→90 BPM change to within ±1 BPM
in 1.167 s, freewheels a one-second dropout by 1.9996 beats, and re-locks
without a tested position discontinuity. `Time<Transport>` keeps its monotone
clock while Start/Song Position move the musical origin; the editor reads the
same clock in a fixed 24-pixel transport strip; and the demo graph uses
`SyncLfo` and bar phase rather than wall time.

*Carried forward:* `Events<Beat>` still has no consumer, and Start/Song Position
arriving mid-tick quantize musical zero to the tick boundary (under 9 ms at
120 Hz). MIDI sources without individual hardware timestamps can also lock
stably to frame-rate-derived BPM when multiple clock pulses collapse in one
frame (for example, 30 fps → 75 BPM for a 120-BPM clock). The transport layer
came through the wire migration untouched.

### Wires — the engine rebuilt (L) — **complete**

Unplanned, and the largest deletion in the project so far. The engine M2a and
M2b built worked, and generalised hierarchy, event propagation and data flow
through one mechanism — which cost a port arena, a slot-addressing scheme, a
compile pass with per-kind special cases, and a cook gate with its own dirty
bookkeeping. `compile.rs` was 1132 lines, `tick.rs` 614, `registry.rs` 581. The
wire model deletes all of it: a connection is a Bevy `Relationship` whose type
carries what it does, and entities and components *are* the graph.

Done before M4 for the same reason the unified-edges migration was: **the
project format cannot be designed twice.** A RON schema written against nodes,
edges, ports and ordinals would have had to encode every one of them and then
break.

Design: `specs/2026-08-05-wires-design.md`. Implementation: `0a1a8a0..d97e0c5`,
thirteen commits, tests green throughout — the node engine was deleted last, so
every step ran against a working suite.

*Outcome:* `sway-graph` is a `Wire` trait, two registries, a Kahn sort, a step
list and a walk. Bevy's relationship semantics were pinned by characterization
test before anything was built on them, and all five held, including the
non-cascading despawn the design flagged as its one unverified assumption. The
vertical slice — `Lfo A → Lfo B.amplitude → Transform.translation.y`, with
fan-out and `ChildOf` — resolves in a single tick, which is the design's central
claim under test. The editor reads the new model, keyed by entity.

*Deliberately deleted with it, and now owed:* every node type outside the slice.
`NodeType` impls are gone; **their pure logic and its tests are not** — the
envelope curves, `BeatTrigger`'s boundary math, the LFO phase advance, MIDI
parsing, `Geometry` and its operators all still stand, tested, waiting to be
re-attached as behaviours. `sway-nodes/tests/traces.rs` still passes.

*Carried forward:* `Events<T>` and variadic inlets have no representation
(§2.4); geometry and asset flow is unbuilt (§2.10); a disconnected value no
longer returns to what was authored (§2.11); and vertices are entities rather
than (entity, component) pairs, so two unrelated components on one entity can
read as a false cycle (§2.5). All four are in §7. The node set and geometry flow
are M5's; the rest are open questions rather than scheduled work.

### M4 — Project format and hot reload (M) — **complete**

Design: `specs/2026-08-06-project-format-design.md`, which is the authority on
everything summarised here.

Versioned RON project files loaded as a `bevy_asset` `Asset` with a custom
`AssetLoader`; `AssetEvent::Modified` triggers a reload. This is what makes
authoring possible long before the editor exists, and it is why the editor can
wait. File watching, debounce, and the write-then-rename behaviour of real text
editors come from `AssetServer` rather than a hand-rolled watcher. `bevy_asset`
joins `sway-graph`'s dependency list here, as its manifest anticipates.

Under wires a project file is entities, components and wires — which is exactly
what Bevy's `DynamicScene` already serializes, entity remapping included. **It
is still not used**, for the reason this milestone existed before the wire
model: `DynamicScene` is keyed by full reflect `TypePath`, is verbose, and a
round-trip through the editor rewrites the file wholesale, destroying comments
and ordering. The format is both human- and machine-authored, and that
constraint outranks the code saved.

The shape, decided here rather than at M7:

- **Our document, reflect payloads.** A stable string id per entity, which
  doubles as its `Name`; components keyed by short registered name; wires keyed
  by the wire type's `NAME`. Reading is `bevy_reflect` per component payload;
  writing is a hand-controlled emitter.
- **One component per line, one wire per line** — a format constraint, not a
  formatting habit. It is what lets M7's writer replace a single line in place
  rather than re-emitting the file.
- **Two registrations, both additive.** `register_authorable::<C>(app, "Lfo")`
  records the short name against its `TypeRegistration` plus reflect insert and
  read; wire registration gains insert and read through `Relationship::from` and
  `Relationship::get`, and panics at startup on a duplicate `NAME` — loud, and
  before a show rather than during one.
- **Reload reconciles by document id.** Entities carry their id; one present in
  both documents keeps its `Entity`, its editor identity and whatever a runtime
  system attached to it. Removed ones despawn, added ones spawn, wires are
  rewired, and the topology flag does the rest (§2.11). Renaming an entity is a
  delete plus an add, which is honest — nothing else identifies it.
- **Failure is split.** A RON syntax error rejects the whole reload and keeps
  the running world: a bad keystroke mid-set must not empty the scene. A
  semantic error — unknown component, unresolvable wire target, a payload that
  will not deserialize — applies everything else and reports the item into a
  `ProjectDiagnostics` resource beside `GraphDiagnostics`. Rendering either
  resource is M7's; M4 only fills them.

Two things this milestone inherits:

- **The read-only inspector M2 asked for and did not build lands here.** Its
  purpose was to find missing editor `TypeData` before an XL milestone depended
  on it, and that purpose is sharper now: the inspector is a reflect walk over
  an entity's registered components, which is the same walk the document's
  reader and emitter perform. M2c supplied the pane to put it in.
- **`EditorPos` is serialized here**, which is what makes M2c's dragged
  positions survive a restart once M7 writes them back.

Two things deliberately *not* here. The **in-place, comment-preserving writer**
is M7's, with the one-line-per-item rule above recorded as what it will need;
M4 builds a whole-document emitter instead and proves the format complete with a
world → document → world round-trip. And the **event fan-in ordering question**
M4 previously inherited is moot: `Events<T>` has no representation under wires
(§2.4), so it returns to §7 as part of that question rather than as a reload
semantic.

*Outcome:* the format's three load-bearing assumptions (design §10) were
pinned by characterization test before any format code existed, and two of
the three came back false. `ron::Value` cannot drive
`TypedReflectDeserializer` through an enum field, so a document's component
payloads are stored as raw text (`Box<ron::value::RawValue>`) and driven
through `ron::de::Deserializer::from_str` at the point of use, rather than
via a pre-parsed `ron::Value`. And `ReflectComponent::apply` does not leave
a component's unnamed fields alone: `TypedReflectDeserializer` fills every
unnamed field from `ReflectDefault` *at deserialize time*, so the "partial"
value `apply` receives is already a full, default-filled struct — `apply`
and `insert` behave identically. The applier uses `insert` unconditionally
and accepts the resulting loss: a reload resets any field the document does
not name back to its `Default`, even one a wire is currently driving. A
`ron` 0.12.2 quirk was also found and worked around — `RawValue` capture
absorbs an adjacent pretty-printer separator on reparse, breaking
byte-equal round-tripping, fixed by trimming each payload immediately after
parse.

The inspector (§6) is the reflect walk it was designed to be, and it found
one gap directly: `Transform`'s `rotation` (a `Quat`) has no dedicated
formatter and falls back to `{value:?}` debug output — the signal, per the
inspector's own doc comment, that the type wants editor `TypeData`. Every
other field the demo's five authorable components actually use (`f32`,
`Vec2`, `Vec3`, and the `Waveform` enum) renders cleanly. Carried forward to
M7: an editable, non-read-only `TypeData` widget for `Quat`, and rendering
`ProjectDiagnostics` (and `GraphDiagnostics`) beside each other — M4 fills
the resource and surfaces syntax failures through Bevy's asset-load log, but
per-item errors (`UnknownComponent`, `BadPayload`, `UnknownWire`,
`UnresolvedTarget`) have no widget yet, so a typo in a component name is
silent in the UI.

The whole-document round-trip (world → document → world, spec §5) holds:
`document_to_world_to_document_is_stable` and `the_emitted_text_reparses`
both pass against the engine's wire fixtures, and
`demo_document_round_trips_through_the_world` pins the same completeness
check against the demo's real component/wire set (`Lfo`, `Transform`,
`EditorPos`, `DemoCube`, `ChildOf`, and the four demo wires). The one thing
the format still cannot express is an asset handle — `Handle<Mesh>` is asset
flow, which is M5's — so the demo's cubes are authored as a `DemoCube`
marker component and a plain Bevy `Added<T>` system attaches the mesh and
material outside the document, exactly as designed as the milestone's one
deliberate seam.

One environment-specific surprise, unrelated to the format itself: Bevy's
default `AssetPlugin` resolves `assets/` relative to `CARGO_MANIFEST_DIR`
(the crate's own directory), not the workspace root or the process's
working directory. `assets/demo.sway.ron` was first placed at the
workspace root on the assumption that `cargo run`'s working directory
governed asset resolution; it does not, and file watching silently found
nothing there. Moved to `crates/sway-app/assets/demo.sway.ron`, where Bevy
actually looks. Worth remembering for M5 and M7, which will add more
asset-backed content.

Manually verified with the app running: an edited beat interval changes bob
speed within a frame or two of saving; rewiring a cube's `translation.y` to
the other LFO takes effect live; adding and then deleting an entity in the
document reconciles by id rather than restarting the scene — the other two
entities keep bobbing, unaffected, across both edits; a syntax error (a
missing closing paren) leaves the running scene untouched and reports the
failure rather than crashing or clearing it; fixing the syntax resumes
updates from the file.

*Exit:* a set can be authored by editing text with the app running — met.

### M5 — Visual runtime (L, larger than it was)

The real version of M1, and now also the milestone that **puts the node set
back**. The wire model deleted every node type outside its slice while keeping
their logic (see the Wires entry), so M5 opens by re-attaching that logic as
components, wires and behaviours: MIDI note and CC, `Envelope`, `Math`, `Remap`,
`Switch`, `Select`, `BeatTrigger`. That is re-wiring tested code, not rewriting
it, and each type must round-trip through M4's document as it lands.

Then the part that was always M5: **geometry flow plus the rest of the scene
set** — `Asset`, `Camera`, one component per light type, `Scatter`,
`CopyToPoints`, the renderable marker, and `Grid`/`Displace`/`Mesh` returned as
operators. Runtime services (`PointCloudSet`, `SpriteLayers`, `Emitters`,
`CameraRig`, `AnimationDirector`) with owned invariants, glTF mesh instancing,
curve-driven procedural animation, physics if wanted. Where the fire-and-forget
decoupling earns its keep: a behaviour triggers, ECS systems continue.

**Geometry flow is a design decision, not just work.** §2.10 names the shape — a
wire delivering the upstream `Geometry` plus a behaviour that cooks with its own
parameters in hand — but not who owns the intermediate, how a chain's cook is
gated now that `Changed<T>` is the only mechanism, or how `Handle<Mesh>` reaches
`Mesh3d` without the graph touching an asset. Decide it here, with GPU
residency, since the two constrain each other.

**M1's visuals become part of the graph here, and the compute→draw gap closes
here.** M1's demos are standalone, spawn their own cameras, and carry an
unamortised extraction — a per-frame CPU clone and re-upload of the whole
instance buffer, invisible at 50k points on an M4 and not something to inherit
unexamined at production cardinality.

**One M2b invariant is worth carrying forward even though its code is gone:**
the mesh upload gate fingerprinted `P`'s `Arc` and point count only, so an
operator that rewrites `N`, `uv` or indices while passing `P` through would
produce a **silently stale mesh** — the one failure in that gate that does not
fail toward wasted work. Whatever replaces it must decide what a cheap
whole-`Geometry` identity is, which is exactly the decision GPU residency forces
anyway.

Two things deliberately *not* here. An attribute expression operator — Houdini's
wrangle, where most of its power concentrates — is a language or a compiled
kernel and is its own project; ship fixed operators first. And values driven at
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

**Smaller than it was, because M2c and M4 took its read half.** Pan/zoom, boxes
keyed by entity, edge routing, hit-testing, the three-pane layout, the live
viewport, live values, and selection sync all exist; the read-only inspector
arrives at M4. What remains is the *write* half, which is where the difficulty
always was.

- Topology editing: drag-to-connect (deleted at M2c precisely so it would not
  ship as a lie), entity creation from a palette, deletion, and value editing in
  the inspector. The palette and the legality of a drag both come from the wire
  registry: a wire may be drawn from an entity carrying its `Source` to one
  carrying its `Target`, and the registry already answers both. Also the
  diagnostics pane M4 left empty: render `ProjectDiagnostics` beside
  `GraphDiagnostics` so a mid-edit typo is visible rather than silent.
- **The in-place document writer.** M4 decided the format and left this: locate
  the one line for the component or wire that changed and replace it, so
  comments and ordering survive. Plus `EditorPos` written back against M2c's
  seeding rule — a snapshot seeds a position once and never again, or the next
  frame snaps a dragged box home.
- Event-edge activity, which needs the per-edge ring buffer M2c refused to put
  in the hot tick for a read-only view. Blocked on `Events<T>` having a
  representation at all (§7).

Topology editing inserts and removes wire components and spawns and despawns
entities; the watch systems mark the topology dirty and the next tick rebuilds.
Nothing here weakens §1's guarantee that the graph is fixed before it runs:
during a show there is no editor, and the watch systems are not even compiled
into the schedule.

Two things specific to §2.10. **Deleting an entity must reparent its children
first** — Bevy's despawn cascades to `Children`, so deleting a group would
otherwise take out everything under it. And the canvas should surface **per-node
cook time and the display flag**, which are the two affordances that make a
Houdini-shaped graph debuggable at all (§7, §2.10).

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

Four of these are new with the wire model, and they are first because they are
capabilities the previous engine had and this one does not.

- **`Events<T>` has no representation.** §2.4 states why the value/event split
  is still required — "CC is 0" versus "no CC arrived", and note timing that
  does not collapse to tick granularity — and the wire model currently cannot
  express it. The shape is probably a wire whose `Target` is a per-tick buffer
  component, but the hard part is the **clearing policy**: who drains the buffer
  and when, such that every consumer in the order sees the same occurrences and
  nothing is seen twice. Nothing needs it before MIDI note and CC return, so
  **answer at M5**, with them.

- **Variadic inlets have no representation.** A `Vec` of sources is not a
  single relationship, so `Merge` and `Sum` have no form. `Math` and `Switch`
  stay binary and compose, so nothing is blocked; the question is whether a
  fan-in wire type (many consumers of one target, i.e. the relationship reversed)
  is worth introducing or whether composition is simply the answer. Cheap to
  defer, so deferred.

- **A disconnected value does not return to what was authored** (§2.11). The
  previous engine kept the authored value in a params struct and shadowed it
  with the connected one; the wire model has one component field and no shadow.
  The intended answer is that the *document* holds the authored value and the
  editor restores from it on disconnect — undo-shaped policy rather than engine
  state — but that is asserted, not built, and M7 is where it becomes real.

- **Vertices are entities, not (entity, component) pairs** (§2.5). Two unrelated
  components on one entity flowing in opposite directions read as a false cycle.
  The escape hatch is splitting the entity, which is cheap. If it recurs often
  enough to be annoying, the fix is a richer link representation, and it is
  bounded work — only the rebuild changes, not the tick.

- **Cook cost belongs to the graph author, not the tool.** This is a decision,
  recorded here because §2.6 makes its consequence unavoidable: expensive work
  triggered by the tick still runs on the main thread inside the frame, so it
  hitches. Houdini and TouchDesigner take the same position, and the alternative
  — a budget that silently defers work — trades a visible problem for an
  invisible one. The tool therefore *reports* cost rather than policing it:
  per-node cook time in the editor (M7), and a marking on the values that
  invalidate `Geometry` so the author can see that `Scatter.count` is not
  `Light.intensity`. The residual risk is covered by M6's watchdog rather than
  by the engine. The wire model moved cooking out of the graph and into
  `Changed<T>` systems, which changes where the time is spent but not who owns
  it.

  The escape hatch, if this ever does bite: cook on `AsyncComputeTaskPool` and
  apply the result when it lands, which genuinely decouples cost from the frame
  at the price of geometry arriving a frame or more late. Named here so it is a
  known option rather than a 2am rediscovery.

- **How geometry flows, and which operators are GPU-resident**, are now one
  question and both are open. §2.10 gives the CPU shape (a wire delivering the
  upstream `Geometry` plus a behaviour that cooks) and the GPU shape (extract a
  dirty set, dispatch a render-graph subgraph in wire order, `ShaderParams` for
  values, signals stay on the CPU), and the criterion for the second is decided
  (output size known before dispatch). M1 dispatched one compute operator from a
  dirty set and read its output back correctly, which is an **absence of a
  counterexample, not a confirmation** — one dispatch, one readback, one draw,
  all well inside conservative bounds, and the compute output never reached the
  draw. What is open: how far the criterion reaches, whether mixed residency is
  tolerable or forces a rule that a chain is entirely one or the other, and what
  a cheap whole-`Geometry` identity is. **Answer at M5.**

- ~~**The cook gate is one bit per node, not one bit per reason.**~~ — void, not
  answered. The engine gate it described was deleted with the node engine;
  `Changed<T>` on a real component is now the whole mechanism (§2.11). The
  underlying question survives inside the geometry-residency item above: a
  consumer owning an expensive resource still has to decide whether an upstream
  change actually requires rewriting it, and `Mesh`'s old `GeometryFingerprint`
  over `P`'s `Arc` pointer and point count is the shape of the answer that will
  be needed again.

- ~~**A node whose tick depends on a cook from the same tick has no
  expression.**~~ — closed, twice. The unified-edges design merged the two
  phases; the wire model then deleted the concepts the question was posed in.
  There is one ordered list of steps, and anything that must run after a value
  arrives and before it is read downstream is a behaviour placed in it (§2.2).

- **Fixed tick rate value** is **120 Hz**, and the wire model changed the cost
  structure underneath it without measuring the result. `graph_tick` is now a
  flat walk of `Vec<Step>` with no arena, no gather and no cook, which should be
  strictly cheaper than what M2b measured, but **no direct measurement has been
  taken**. The earlier figures (M2b's 624 ns gate-closed, 13.07 µs with two CPU
  cooks; M2a's 2.226 µs/tick) describe an engine that no longer exists and
  should not be cited. Keep the measurement rules when it is finally done:
  **time `graph_tick` directly**, because an `App::update()` fixture floor of
  ~40 µs dwarfs the signal, and **run with `--test-threads=1`**, because
  parallel test execution inflates timings by up to 40%. The rate's real margin
  is decided under M5's load, not before.

- ~~**Reflect's ergonomics under a real node set**~~ — resolved, and then made
  much less load-bearing. Reflection is no longer on the tick path at all; it
  survives for `EditorPos`, the transport types, the inspector and M4's document
  payloads. Two rules from M2a still apply wherever it is used: prefer
  `reflect_clone()` over `to_dynamic()` for any value that must later downcast
  to its concrete type, or reflected enums silently become dynamic proxies; and
  import `ReflectDefault` from the `bevy_reflect` prelude. The narrower part is
  still open for a third milestone running: **editor `TypeData` is unexercised**,
  because the inspector M2 asked for was never built. M4 finally builds it (§5).

- ~~**State lives in two places**~~ — resolved, and the wire model went further:
  there is no engine-side state left at all. No arena, no per-node runtime
  record, no gate bookkeeping — only components in the world and one derived
  step list. Snapshot and restore is therefore a question about the world. The
  open part is narrower still: which components are performance state worth
  restoring and which are derived caches. Revisit before M7, as a labelling
  problem.
