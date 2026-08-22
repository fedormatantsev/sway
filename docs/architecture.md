# Sway — Architecture

Current-state architecture and key design decisions. Ongoing work lives in
`openspec/changes/` (OpenSpec changes).

## 1. What this is

An audiovisual instrument for live sets. It listens to MIDI from a hardware
setup (Elektron Octatrack and similar) and drives real-time 3D visuals out over
HDMI.

Current scope: **MIDI in, HDMI out.**

### Operating model

During a performance the tool runs unattended. MIDI is the only input; nobody
touches a keyboard. The editor is an authoring tool used before the show, not a
performance surface. That removes a large class of requirements — no live graph
patching, no hot topology mutation, no dropped-frame guarantees on edits — and
the architecture may assume the graph is ordered before it runs on stage.

### Visual target for v1

A 3D scene with custom geometry and custom vertex/fragment shaders. Point clouds
and spritesheet layers with z-depth, animated 3D objects, procedural animation
from curves and optionally physics. The scene reacts to MIDI notes and CC,
locked to the transport.

**The MVP target is narrower**, and the roadmap is scoped to it: spritesheet
layers writing **per-pixel** depth from the sheet, asset meshes with PBR
materials whose transforms are wire-animated, and an HDR/cubemap driving their
lighting. Point clouds, procedural geometry, curve nodes and physics are all
past the MVP (see §10 for the full out-of-MVP list).

### Audience

Built for one performer's own sets first, architected so it could be handed to
other VJs later. Project format and the component/wire API get real design
attention; onboarding, docs, and distribution are deferred.

### Layers

**Engine (`sway-graph`)** owns what makes the control graph a graph: the
`Graph` resource, generational `NodeId`, node-kind registration, Kahn rebuild
into a flat step list, and the exclusive tick walk over that list. It does not
own MIDI, pixels, event buffers, the on-disk document, a UI toolkit, or a
channel. It depends on Bevy's non-rendering subcrates — not `bevy_render`.
Connection storage, single-source, fan-out, and rewire are the edge list and
`Graph::connect`. The authoring surface is the graph's own operations
(§2). Show and editor builds share the same engine; there are no
topology-watch systems to omit.

**Runtime (`sway-runtime` + host)** is the Bevy app that exists every frame:
world, render pipelines, animation systems, and a small set of service facades
(`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`,
and similar) with owned invariants. Projectors derive entities and assets from
the graph after the tick. Rendering is headless to a texture; `sway-app`
owns winit and the wgpu device and presents either into the editor or fullscreen
HDMI. The runtime runs whether or not a graph is loaded.

**Nodes, edges, and events** bridge engine and runtime:

- **Value edges** — `(NodeId, path) → (NodeId, path, slot)` on the `Graph`;
  propagate copies outlet into inlet and must not write an equal value.
- **Node evaluation** — only when output depends on an inlet in the same tick
  and must sit in dataflow order. Otherwise ordinary Bevy systems.
- **Events (`sway-events`)** — separate crate; occurrences in a per-tick
  arena, addressed by handles that travel ordinary edges. See §3.

**Document (`sway-document`)** is out of `sway-graph`. It reads and writes the
`Graph` resource — no parallel snapshot model inside the engine.

**Supporting crates:** `sway-base-nodes` (built-in value/signal node kinds,
including `CurveSampler`, `Timer`, and the generic `Trigger` payload),
`sway-midi-core` (MIDI IO, messages, and pulse-clock math), `sway-midi` (Bevy
MIDI plugin, transport and control-change snapshots, `MidiTime` / `MidiCc`,
channel-filtered `MidiNotes`, and `OnMidiNote`), `sway-geo` (geometry
tables and CPU operators), `sway-editor` (masonry UI on the live graph),
`sway-gpu` (single device-creation pin for the bevy↔vello coupling).

## 2. Decoupling and the graph contract

### Central decoupling

The graph is the **authored model**. It declares nodes and edges, evaluates
them, and tracks which nodes changed. The Bevy world is **derived**: projectors
spawn entities, allocate asset handles, and attach materials after the tick.
Nothing in the world writes graph values; authoring reaches the world only
through the graph.

Low-cardinality signals live as node fields. High-cardinality data (points,
rigid bodies, particle lifetimes) lives in the ECS, parameterised by the graph.
Geometry is never a value carried by a connection. **Physics is never wired.**
Fire-and-forget interactions ("burst here", "start clip 3") still belong to
observer triggers (§3), not to value edges.

### Nodes and edges

A `Graph` is a `Resource`: a `Vec<Node>` plus an edge list, addressed by
generational `NodeId`. A deleted id never resolves to a later node. There is
no entity-as-vertex and no relationship component per connection.

A **node** is one reflected type with exactly three nested parts. An absent
part is `()`:

| Part | Role | Editor | Document |
|---|---|---|---|
| Inlets | Authored and/or driven fields | Fields + inlet sockets | Yes |
| State | Internal memory | Hidden | No |
| Outlets | Values other nodes can read | Outlet sockets only | No |

Alongside the parts a node carries **annotations**: a map of typed values,
keyed by name, that the graph never interprets, never acts on, and never
requires. They are where a surface parks its own state — the editor keeps
canvas placement under `"pos"` as a `Vec2`. Writing one does not mark the node
changed, because nothing downstream depends on it. The graph holds no other
display state: no selection, no viewport, no canvas.

An **edge** is `(src NodeId, outlet path) → (dst NodeId, inlet path)` plus a
`slot` sort key. The path names a **declared field** of the inlets or outlets
part. The resolver prepends `inlets.` / `outlets.`, so stored paths stay short
(`"translation"`, not `"inlets.translation"`). A compound inlet may be connected as
a whole *or* by naming one component, so one axis can be driven while the
others keep their authored values; scene placement is three inlets
(`translation`, `rotation`, `scale`) so a `Vec3` can drive a cube without a
nested path.

Connect-time legality is decided from reflected types:

```
outlet type S → inlet type D is legal iff
  D == S            direct
  D == Option<S>    optional inlet
  D == Vec<S>       variadic inlet; several edges may land here
```

`slot` is a sort key, not an array index. At rebuild, edges landing on one
`Vec<T>` inlet are collected, sorted by slot with `NodeId` breaking ties, and
the `Vec` is sized to the edge count and filled in that order. A non-variadic
inlet sits at slot 0 and takes the same code path.

`Graph::connect` enforces the invariants `Relationship` used to provide
silently: no self-edges, a single-connection inlet is replaced rather than
duplicated, and deleting a node drops every edge that names it. Fan-out is
free — it is the edge list.

A **valueless** (marker) edge carries no copy but stays in the sort, so
producer chains such as `FrameSequence → SpriteMaterial` still order. Asset
and scene connections use protocol markers, not `Handle<T>` on the edge.
Handles live in node state, which is never serialized.

### Evaluation

A node kind implements `evaluate(&mut self, world: &World)`. `&mut self`
reaches inlets, state and outlets. `&World` is how an ordinary node such as
`MidiTime` reads an external resource without `sway-graph` naming a MIDI type
and without a pre-tick injection phase.

The tick is an exclusive system driven through `World::resource_scope`, so
`&mut Graph` and `&World` are held at once. Inside the scope the `World`
genuinely does not contain the `Graph`: a node cannot re-enter the tick or
read another node's outlet behind the edge list's back.

Everything outside the graph writes it through the graph's own operations —
`insert`, `create`, `remove`, `set_field`, `connect`, `disconnect`,
`set_slot` — and there is no second vocabulary in the engine restating them as
data. A field write carries `&dyn PartialReflect` and reports
`FieldWrite { Written, Unchanged, Rejected }`, so the engine enumerates no set
of value types a write may carry: converting whatever an editing control
produced into the field's declared type is the authoring surface's job.

A surface that cannot reach the graph at the moment a gesture happens may
record that gesture in a form of its own and apply it later — a masonry widget
cannot borrow the world mid-dispatch, so `sway-editor` has an `EditorEdit`
payload and a `PreUpdate` applier of its own. That form belongs to the surface,
not to the engine. Anything that *can* reach `&mut Graph` — document load, the
viewport's gizmo and picker — calls the methods directly. Picking returns an
`Entity` only to look up a `NodeId` for the editor's selection.

## 3. Events

Events are not value wires and are not owned by `sway-graph`. **`sway-events`**
owns an **occurrence arena** and the one plugin that empties it. A value wire
carries a *level* — the tick copies a field and a node reads whatever stands
there; an event wire carries the things that *happen*, over exactly the same
edges.

- The occurrences live in an **`EventArena`**, a non-send `World` resource
  holding **this tick's batches**. What travels the wire is an
  **`EventHandle<P>`**: two integers and a payload type tag, *naming* one
  batch. Two operations reach a batch and nothing else does — `publish` takes a
  whole batch and returns its handle, `read` takes a handle and returns that
  batch. A handle is a name, not a capability, so a consumer holding one has no
  operation that could add to what it received: the read and write paths are
  separated structurally rather than by discipline.
- **A handle is an ordinary reflected field value**, so a trigger connection is
  an ordinary edge: the same copy step, the same exact-type legality rule,
  `Option<EventHandle<P>>` for an optional inlet and `Vec<EventHandle<P>>` for
  a variadic merge in ordering-key order. `sway-graph` names no handle,
  occurrence, payload type or arena, and needs no knowledge of any of them.
- **A producer publishes during its own `evaluate`** — it hands its whole batch
  to the arena, gets a handle back, and writes that handle to its own outlet —
  and **holds no state**: no buffer, no occurrences, no handle between ticks.
  With nothing to publish it writes the **empty handle**, and publishing an
  empty batch yields that same empty handle, so an unconditional producer
  cannot report a change on a tick where nothing happened.
- **A consumer reads by handle.** Fan-out costs nothing and copies nothing:
  every consumer of an outlet holds the same handle and reads the same
  refcounted batch. Reading does not consume. A node that forwards or merges
  publishes a batch of its own.
- **The arena is emptied before every tick** — drop every batch, bump the
  generation — which is the whole lifecycle, O(batches) rather than O(nodes),
  with no per-kind index of which fields are handles.
- **A stale handle reads empty.** A handle carries the generation it was
  published in, so one that outlived its tick reads as *no occurrences* —
  never as whatever now occupies its slot, and never as a failed evaluation.
  Two behaviours fall out of that rather than needing machinery: a producer
  that stops publishing leaves nothing observable behind, and **a trigger
  connection inside a cycle carries nothing**, because the handle its partner
  published last tick is stale.
- **Publishing dirties.** A handle names one tick's batch, so a publishing node
  writes a different outlet value every tick and is reported changed, as is
  every node its handle reaches. Only the empty handle replacing the empty
  handle reports nothing.
- **A handle is session state.** It is never authorable; a document stores the
  empty handle for a handle inlet, so a save names no batch and no generation
  and is byte-stable across ticks.
- Registration is one call per payload type (`register_event_handle::<P>`); the
  arena itself needs no registration of any kind.

**Order relative to the graph tick** (`FixedUpdate`):

1. `EventClearSet` — empty the arena. Deliberately *not* gated on asset
   loading, so the arena stays bounded whatever else is running.
2. Graph tick runs — producers publish and consumers read, in the order the
   graph already puts a producer before its consumers, so a chain of trigger
   connections carries occurrences end to end within one tick.

**`MidiNotes` is the first producer**: it publishes every note-on and note-off
of the tick on its authored channel as one batch — channel, note, velocity,
on/off and the sub-tick offset the MIDI drain already records — leaving the
empty handle on a silent tick. It still does not pick a pitch. **`OnMidiNote`**
matches an authored scientific-pitch name (`C4`, `D#1`) against that stream
and publishes generic `Trigger` occurrences on `pressed` / `released`.
`Trigger` lives in `sway-base-nodes`; `sway-midi` depends on that crate one way
so the converter can name the payload. A host that adds `MidiPlugin` does not
register `Trigger` on MIDI's behalf.

## 4. Ordering, rebuild, and tick

### Order

One derived artifact exists — a flat list of steps over `NodeId`:

```rust
pub enum GraphStep {
    TruncateList { node: NodeId, path: ParsedPath, len: usize },
    Propagate(PropagateStep),
    Evaluate { node: NodeId },
}
```

A rebuild walks the edge list, Kahn-sorts the **nodes** they connect, and
emits, per node in that order, list truncations, inbound propagations, then
evaluation. Ties break by ascending `NodeId`. The unit of ordering is the
node: `evaluate` reads every inlet and writes every outlet, so every outlet of
a node depends on every one of its inlets. There is no finer vertex that could
report a cycle the node does not actually have.

### Rebuild

The order rebuilds when the graph is dirty — commands that change topology set
that flag. Authoring is the command set; there is no ECS `connect` API and no
per-wire `on_add` hook.

A cycle never stops the render. The sort emits the acyclic part in topological
order and appends cycle members in `NodeId` order, where they read the previous
tick's value. A valueless edge emits no propagate step but still contributes a
sort constraint.

The topological sort stays ours: Bevy's `ScheduleGraph` orders systems per type,
not instances, and its errors would name systems rather than nodes.

### Tick

The graph runs as a single **exclusive system in `FixedUpdate`**, rate decoupled
from render framerate via `Time<Fixed>`. Serial evaluation through
`World::resource_scope`. Writes are immediate within the tick.

For a `Propagate` step the tick takes `&outlets` of one node and `&mut inlets`
of another via `slice::get_disjoint_mut`, copies the field, and dirties the
destination only when reflected equality says the value changed. For an
`Evaluate` step it calls `evaluate(&mut self, &World)` on the node. An equal
write reports nothing.

`FixedUpdate` decouples tick rate from frame rate, not tick cost — a heavy cook
still hitch the frame; `max_delta` may drop ticks under sustained overload.
**Evaluation cost belongs to the graph author**, not the tool. The tool reports
cost rather than silently deferring work.

Instance order cannot be expressed as system-order constraints without
one-tick latency or a schedule traversal per DAG level. A serial walk of the
step list is the chosen semantics.

Consequences:

- Zero to n ticks per rendered frame; between ticks the world keeps animating.
- A recorded MIDI trace plus a fixed delta yields bit-identical tick sequences
  for golden-trace tests. Live overload that drops ticks may diverge — acceptable
  for an instrument, not an unqualified promise. World reads from `evaluate`
  must stay confined to resources the trace controls.

### Transport ownership

`sway-graph` is MIDI-agnostic. **`sway-midi-core`** owns MIDI IO, typed
messages, and the Bevy-free `PulseClock`. **`sway-midi`** owns the Bevy plugin,
its inbox and tick buffers, the `Transport` snapshot resource, and `MidiTime`
as an ordinary node that reads that resource through `&World` during
evaluation. Each fixed tick drains timestamped messages into `PulseClock` and
samples a fresh `Transport` before the graph tick; there is no injection phase
and no MIDI type named by `sway-graph`.

## 5. Editor integration and what Bevy owns

### Editor and runtime

The editor embeds a live runtime viewport in one process, sharing one wgpu
device. Bevy renders to an offscreen texture; a thin presenter composites into a
masonry widget when authoring or blits fullscreen on stage. Bevy runs headless;
the host owns winit and the device (`sway-gpu`) and drives `app.update()`.

The editor reads the graph directly — nodes, edges, registered node kinds,
live inlet values, evaluation order — with no control socket and no second
schema. UI is retained-mode masonry; pan/zoom, bezier edges, and
hit-testing are hand-rolled.

**Risk taken deliberately:** masonry/Vello and Bevy must resolve the same wgpu
and winit. Mitigations: exact workspace pins, duplicate detection as a build
failure, device creation confined to `sway-gpu`, upgrade only when the known-good
tuple realigns. If that gate fails, the fallback is Syphon-style frame sharing —
not months of dependency patching.

### Ownership table

| Concern | Owner |
|---|---|
| Value connection storage, single-source, fan-out, rewire | **`sway-graph`** (edge list + `Graph::connect`) |
| Entity lifecycle for projected scene nodes | **`sway-runtime`** projectors |
| Hierarchy / transform propagation | `bevy_transform` (`ChildOf`), projected from `children` edges |
| Value typing of connections | `bevy_reflect` at connect time |
| Dirty reaction after writes | graph dirty set (+ no-equal-write rule) |
| Editor metadata / reflect payloads | `bevy_reflect` |
| Fixed tick rate / accumulator | `bevy_time` (`Time<Fixed>`) |
| MIDI IO, typed messages, pulse-grid clock math | **`sway-midi-core`** |
| Beat / transport + control-change snapshots, `MidiTime`, `MidiCc` | **`sway-midi`** |
| Topological order, step list, walk | **`sway-graph`** |
| Selection | **`sway-selection`** (`Selection` resource), set by the editor's panes and by viewport picking |
| Node placement on the editor canvas | **`sway-editor`**, persisted as a `"pos"` annotation on the node |
| Viewport pointer/key vocabulary | **`sway-viewport-input`**, shared by `sway-editor` and `sway-editor-viewport` |
| Deferring an edit past a widget's event dispatch | **`sway-editor`** (`EditorEdit` + its `PreUpdate` applier) |
| Occurrence arena (this tick's batches) + its pre-tick clear | **`sway-events`** |
| Document parse/emit | **`sway-document`** from the `Graph` |
| Geometry tables / operators | **`sway-geo`** (CPU; dormant for the MVP, §6) |

`sway-graph` must not depend on `bevy_render`, MIDI types, a UI toolkit, a
channel, or the document format — see §11.

## 6. Scene composition

The scene is built by the graph, not loaded beside it. Camera, lights, meshes,
groupings, materials, and transforms are authored as **nodes**; projectors
derive the Bevy world from that graph. Ownership is total — teardown and
reload stay answerable because deleting a node despawns its entity and
releases its asset.

The world shape is not a generic mirror of the graph. A producer or material
node owns an asset handle and no entity; a scene node owns an entity. Handles
are allocated structurally at node creation so a connection is never waiting
on a handle that does not exist yet — only its content is ever pending.

The scene node set is closed: `MeshNode`, `Group`, `Camera`,
`DirectionalLight`, `PointLight`. `Group` carries translation, rotation, scale
and children only and refuses geometry.

### Protocols

A protocol is a ZST marker used as an outlet, plus a reflected trait the
projector calls. A node joins by declaring the marker and implementing the
trait.

| Protocol | Marker | Trait |
|---|---|---|
| Material | `SceneMaterial` | `MaterialNode::attach` |
| Image sequence | `ImageSequence` | `ImageSequenceNode::texture` |
| Mesh source | `MeshSource` | `MeshNode::handle` |
| Hierarchy | `SceneChild` | — (the projector needs only the `NodeId`) |

A material node is the only thing that knows its concrete `M`; it inserts
`MeshMaterial3d<M>` itself. `children` edges project into Bevy `ChildOf` so
transform propagation stays Bevy's, and a parent is inserted only where a
child connection exists.

### Geometry (past MVP)

The longer-term model is Houdini/USD-shaped: **operators act on streams, so
cardinality lives in the data, not the operator count.** A `Geometry`
component of named planar attribute tables remains part of that design.

**MVP:** geometry *operators* are out of scope entirely — the MVP's target scene
uses asset meshes, so `Grid` / `Displace` / `Scatter` / `CopyToPoints`,
geometry-intermediate ownership, and mesh-identity fingerprinting all move
past it. `sway-geo` stays dormant. Nothing in the MVP produces a `Geometry`
component.

GPU-resident graph ops and mixed CPU/GPU residency remain out of scope.

## 7. Graph state and the ECS

The graph is a `Resource`. Node fields are reflected values on that resource,
not components on graph vertices. Projectors copy what the scene needs onto
ordinary Bevy entities after the tick; Bevy takes over from there
(`Transform` propagation, rendering).

```
PreUpdate     drain EditorEdit → Graph (editor builds only)
FixedUpdate   MIDI feed → drain → sample Transport
              sway-events: clear the arena
              rebuild order if dirty
              graph tick (TruncateList / Propagate / Evaluate)
Update        projectors (producers → materials → scene → attach → parent)
              Changed<T> reactions, services
PostUpdate    transform propagation, visibility
Extract/Render
```

The tick and the scene/attachment projectors wait until every asset reports
loaded; the MIDI drain does not. `AssetEvent::Modified` for the graph asset
is ignored — reloading a project is an explicit action. Content still
hot-reloads through the ordinary asset watcher.

The document is at **format version 4**. A node entry stores its kind, its
authored inlets, and its annotations — a map of typed values the document
carries and does not interpret, which is where the editor keeps canvas
placement. Version 3 gave placement a field of its own; there is no
compatibility read for it, so a version-3 file is refused by version, naming
both. The only file affected was `demo.sway.ron`.

Because the tick holds `&mut Graph` through `resource_scope`, writes are
immediate within the tick.

### Never write an equal value

Propagate compares with reflected equality before writing, so an equal value
does not dirty the destination. Asset mutation follows the same discipline
outside the graph (`get`, compare, then `get_mut`).

### Unconnected values

A connected edge overwrites the inlet each tick; on disconnect the field keeps
whatever arrived last. Restore-to-authored-on-disconnect is out of MVP.

There is no authored value distinct from the live inlet: the node's inlets
*are* the value, and `to_document(graph)` serializes those inlets (state and
outlets are omitted). **Every inlet is editable**, driven or not — the
inspector does not refuse a driven field. Editing a driven field holds only
until the next tick, when propagate writes over it again. A save still bakes
the instantaneous driven value into the file; harmless, since the first tick
after load overwrites it. The gizmo writes through `Graph::set_field`, exactly as the inspector's edit
does — directly, because it already holds the graph.

Continuously driven transforms should write a
previous/next pair (`DrivenTransform`) and let a per-frame system lerp by
`Time<Fixed>::overstep_fraction`.

### Structural change

Spawning, despawning, and rewiring do not happen during a tick. Nothing
reachable during one *can* mutate the graph: `resource_scope` takes the `Graph`
out of the `World` a node sees. What `PreUpdate` decides is *when* an editor's
deferred edits land, which is before the next `FixedUpdate` rebuilds and
ticks. The document uses stable string ids; `NodeId` is runtime-only. Load
mints a `HashMap<FileId, NodeId>` once — there is no `claim.rs` and no
four-pass reconcile against a parallel entity world. Deleting a node despawns
its projected entity and releases its asset.

Observers may spawn and mutate the world freely but must not write the graph
mid-tick.

## 8. Crate layout

```
sway-gpu              wgpu instance/device/queue — bevy↔vello pin lives here
sway-graph            Graph resource, NodeId, mutation API, order, tick
                      (no MIDI, no document, no bevy_render, no UI, no channel)
sway-events           occurrence arena + EventHandle<P>; one plugin, one
                      pre-tick clear (the engine depends on none of it)
sway-document         version 4 on-disk format; stable ids; load/save the Graph
sway-base-nodes       the base value/signal node kinds (CurveSampler, Timer,
                      Trigger, Math, Remap, MakeVec3) — pure functions of their
                      inlets (a handle inlet is resolved through the arena)
sway-midi-core        MIDI IO, typed messages, PulseClock (no Bevy)
sway-midi             Bevy plugin, Transport snapshot, MidiTime, MidiCc,
                      channel-filtered MidiNotes, OnMidiNote
sway-geo              Geometry attribute tables and CPU operators (dormant)
sway-runtime          headless Bevy app; render-coupled node kinds; projectors
sway-viewport-input   the viewport's pointer/key/scroll vocabulary
sway-selection        which node the editor is pointed at
sway-editor           masonry UI; the editor's own deferred-edit payload
sway-editor-viewport  camera orbit, transform gizmo, picking (editor only)
sway-app              host: winit, device, editor shell or show presenter
```

Deleted with this layout: the point-cloud, scatter-compute and sprite-layer
pipelines, which were exported and reached by no application or test (§10).

There is no `sway-schema`. Connection typing is Rust's; editor metadata and
document payloads use `bevy_reflect`; registries and the document shape do not
need a separate schema crate.

## 9. Testing strategy

- **Graph engine** — golden-trace tests at a fixed tick delta.
- **MIDI / transport** — recorded clock traces (tempo changes, dropouts) in
  `sway-midi`.
- **Order** — determinism, cycle append behaviour, **one-tick** chain
  resolution.
- **Change detection** — an equal propagate leaves the destination undirtied.
- **Events** — the arena's publish/read/clear invariants; a stale handle
  reading empty; fan-out sharing one batch; what publishing does to the dirty set
  buffer.
- **Cooking** — pure geometry functions; unrelated changes recompute nothing.
- **Document** — round-trip; malformed input reports; load mints stable ids
  without shifting later nodes.
- **Runtime** — MIDI traces into a headless world; assert ECS state and service
  calls.
- **Rendering** — no pixel-diff tests; verify by eye.

## 10. Design decisions and MVP scope

**Settled**

- Evaluation cost is the author's problem; the tool reports rather than
  polices.
- Events as in §3. An event wire *is* a value wire — the value it carries is an
  `EventHandle<P>`, so payloads stay generic while the engine keeps one edge
  kind and one legality rule.
- Document lives outside `sway-graph`; authoring is the graph's own
  operations, not a second vocabulary restating them as data (§2, §11).
- MIDI IO and pulse-clock math live in `sway-midi-core`; the `Transport`
  snapshot and `MidiTime` live in `sway-midi`; the graph stays MIDI-free.
- Fixed graph tick retained for continuous values.
- Connections are typed at connect time from reflected field types. Transform,
  colour and tint inlets take `Vec3`, not per-axis floats; a `MakeVec3` node
  with driveable components is what produces them. (Named for the assembling:
  a node kind called `Vec3`, where `bevy_math::Vec3` is what its own outlet is
  made of, had to be aliased around at every use.) Reaching into one
  consumer's vector inlet by naming a component is the other route, and both
  are legal. Genuinely scalar fields take floats.
- A base node kind is a pure function of its inlets and state. One that
  advances over time takes that time as an **inlet** rather than reading a
  clock, so the source of time is a connection the author can see and change —
  `MidiTime`, in practice.
- The palette lists registered node kinds from the type registry.

**Out of MVP**

- `Merge` / `Sum` node kinds (variadic inlets themselves are in the model).
- Restore authored value on disconnect (see §7 — on disconnect a field simply
  keeps whatever value the edge last wrote; there is no authored-value shadow
  to restore from).
- Geometry operators and the geometry cook path (§6).
- GPU-resident geometry operators / compute cook path.
- The point-cloud, scatter-compute and sprite-layer render pipelines. They were
  written for the pre-graph model, no application or test reached them, and
  they were deleted rather than left exported (§11). When the roadmap reaches
  point clouds and GPU scatter, they are written against the node model.

**Open**

- None recorded for the graph model. Cycle members still tick; they read the
  previous tick's values.

## 11. Source structure

Which crate does a thing go in? These are the rules, and §8's layout is what
they produce.

### The engine crate knows no concrete domain

Exactly one crate — `sway-graph` — owns the generic graph mechanics: identity,
nodes, edges, path resolution, connect legality, ordering and the tick. It
names no concrete node kind, no UI toolkit type, no MIDI type, no render type
and no on-disk format. Its manifest is where that is enforced.

Its public surface stays as small as the mechanics require. Where the ECS
framework already provides a behaviour, the engine uses that provision rather
than introducing a type for it. An item with no consumer outside the engine is
not public — `EvalOrder`, the step list, the sort and the path parser are all
crate-private, and what a projector needs is `Graph::eval_order`.

The engine does not enumerate the concrete value types a node kind may declare.
Anything it carries on a node's behalf is reflected, so a node kind with a
field type no other kind uses registers, connects, evaluates and serializes
with no edit to the engine.

### A node domain is a self-contained crate with one plugin

Each domain of node kinds lives in its own crate holding both the node kinds
and their projection onto the ECS world. A domain crate exposes **exactly one**
top-level plugin, and adding it registers every type, system and resource the
domain needs. A host never adds a second plugin, registers a type, or inserts a
resource on a domain's behalf.

`sway-midi` is the shape to copy: `MidiPlugin` brings the transport's
resources, its systems *and* the `MidiTime` node kind that reads them.

A crate is named for the domain it covers, not for the language construct it
contains — which is why `sway-nodes` became `sway-base-nodes`.

### Dependencies point from host to domain to engine

```
sway-app ──▶ domain crates ──▶ sway-graph
```

The engine depends on no crate of this project. A domain crate depends on the
engine. The host depends on domain crates. **No domain crate depends on
another domain crate.**

`sway-editor-viewport → sway-runtime` is not an exception to that: it is not a
node domain but an editor surface, and the dependency runs surface → runtime,
the same direction the host runs. The rule is worded about *domain* crates so
that stays legible.

A declared dependency the crate does not use is removed.

### Shared vocabulary gets a crate of its own

Where two crates need the same vocabulary and neither may depend on the other,
that vocabulary lives in a crate of its own — never parked in the engine
because the engine is what both already depend on.

Two exist. `sway-viewport-input` holds the pointer/key/scroll events the
editor's masonry widget produces and the editor viewport consumes;
`sway-selection` holds which node the editor is pointed at, set by the editor's
panes and by viewport picking. `sway-editor` links masonry and not
`bevy_render`; `sway-editor-viewport` links the `bevy` facade. Neither can
depend on the other, and neither vocabulary is the graph's business.

### Code that nothing reaches is deleted

The workspace keeps no public item, module or plugin that no build path
reaches. Work deliberately deferred past the current milestone is recorded in
the roadmap (§10), not kept as unreachable code.
