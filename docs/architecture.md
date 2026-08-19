# Sway — Architecture

Current-state architecture and key design decisions. Ongoing roadmap and open
work live in `docs/superpowers/specs/2026-07-25-sway-design.md`.

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
past the MVP. See
`docs/superpowers/specs/2026-08-09-mvp-roadmap-design.md`.

### Audience

Built for one performer's own sets first, architected so it could be handed to
other VJs later. Project format and the component/wire API get real design
attention; onboarding, docs, and distribution are deferred.

### Layers

**Engine (`sway-graph`)** owns what makes the control graph a graph: the
`Graph` resource, generational `NodeId`, node-kind registration, the command
set, Kahn rebuild into a flat step list, and the exclusive tick walk over
that list. It does not own MIDI, pixels, event buffers, or the on-disk
document. It depends on Bevy's non-rendering subcrates — not `bevy_render`.
Connection storage, single-source, fan-out, and rewire are the edge list and
the connect command. The authoring surface is `GraphCommand`. Show and editor
builds share the same engine; there are no topology-watch systems to omit.

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
- **Events (`sway-events`)** — separate crate; see §3.

**Document (`sway-document`)** is out of `sway-graph`. It reads and writes the
`Graph` resource — no parallel snapshot model inside the engine.

**Supporting crates:** `sway-nodes` (built-in value node kinds), `sway-midi-core`
(MIDI IO, messages, and pulse-clock math), `sway-midi` (Bevy MIDI plugin,
transport snapshot, and `MidiTime` as an ordinary node), `sway-geo` (geometry
tables and CPU operators), `sway-editor` (masonry UI on the live graph),
`sway-gpu` (single device-creation pin for the bevy↔vello coupling).

## 2. Decoupling and the graph contract

### Central decoupling

The graph is the **authored model**. It declares nodes and edges, evaluates
them, and tracks which nodes changed. The Bevy world is **derived**: projectors
spawn entities, allocate asset handles, and attach materials after the tick.
Nothing in the world writes graph values; authoring reaches the world only
through `GraphCommand`.

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

An **edge** is `(src NodeId, outlet path) → (dst NodeId, inlet path)` plus a
`slot` sort key. The path names a **declared field** of the inlets or outlets
part. The resolver prepends `inlets.` / `outlets.`, so stored paths stay short
(`"translation"`, not `"inlets.translation"`). A compound inlet is connected as
a whole; scene placement is three inlets (`translation`, `rotation`, `scale`)
so a `Vec3` can drive a cube without a nested path.

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

The connect command enforces the invariants `Relationship` used to provide
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

Everything outside the graph writes it through `GraphCommand` (create, delete,
set field, move, connect, disconnect, select, set slot). The gizmo emits the
same `SetField` the inspector does. Picking returns an `Entity` only to look
up a `NodeId` for selection.

## 3. Events

Events are not value wires and are not owned by `sway-graph`. **`sway-events`**
registers the systems that own buffer lifecycle.

- **Emitter entities** carry **`TriggerOut<P>`** components, generic over an
  event payload `P`. Each implementation decides how that outlet is populated
  (MIDI note edge, transport boundary, and so on).
- **Event wires** are `Relationship` components on the consumer, implementing
  an `EventWire` trait that names a `Payload` type and a wire `NAME`. They
  connect a `TriggerOut<P>` to a **`TriggerIn<W>`**.
- **Buffers exist only per event wire**, never on the `TriggerOut` — fan-out
  gives each consumer its own copy. `TriggerIn<W>` *is* that buffer, living on
  the consumer; a component hook installed by `register_event_wire::<W>`
  inserts it alongside `W`.
- Registration monomorphises the clear and copy functions so the tick never
  sees a generic.
- Event wires round-trip through the document like value wires, keyed by type
  path. Event-wire dispatch is a separate catalog from value `ReflectWire`
  (not implemented in this change). Drag-to-connect legality reads both.

**Order relative to the graph tick** (`FixedUpdate`):

1. Clear all event-wire buffers.
2. For each event wire, copy/append from the source `TriggerOut` into that
   wire's buffer.
3. Graph tick runs — behaviours may read their `TriggerIn` / wire buffer.
4. Clear `TriggerOut`s.

Continuous values still need the fixed graph tick; events use that epoch so
every behaviour in the order sees the same occurrences for the tick. Variadic
fan-in is out of scope for MVP.

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
| Value connection storage, single-source, fan-out, rewire | **`sway-graph`** (edge list + connect command) |
| Entity lifecycle for projected scene nodes | **`sway-runtime`** projectors |
| Hierarchy / transform propagation | `bevy_transform` (`ChildOf`), projected from `children` edges |
| Value typing of connections | `bevy_reflect` at connect time |
| Dirty reaction after writes | graph dirty set (+ no-equal-write rule) |
| Editor metadata / reflect payloads | `bevy_reflect` |
| Fixed tick rate / accumulator | `bevy_time` (`Time<Fixed>`) |
| MIDI IO, typed messages, pulse-grid clock math | **`sway-midi-core`** |
| Beat / transport snapshot + `MidiTime` | **`sway-midi`** |
| Topological order, step list, walk | **`sway-graph`** |
| Selection | **`sway-graph`** (`Graph::selection`), read by the editor from the resource |
| Event-wire buffers + pre-tick clear/copy | **`sway-events`** |
| Document parse/emit | **`sway-document`** from the `Graph` |
| Geometry tables / operators | **`sway-geo`** (CPU; dormant for the MVP, §6) |

`sway-graph` must not depend on `bevy_render`, MIDI types, or the document
format.

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
PreUpdate     drain GraphCommand → Graph
FixedUpdate   MIDI feed → drain → sample Transport
              rebuild order if dirty
              sway-events: clear wire buffers → copy TriggerOut → buffers
              graph tick (TruncateList / Propagate / Evaluate)
              sway-events: clear TriggerOuts
Update        projectors (producers → materials → scene → attach → parent)
              Changed<T> reactions, services
PostUpdate    transform propagation, visibility
Extract/Render
```

The tick and the scene/attachment projectors wait until every asset reports
loaded; the MIDI drain does not. `AssetEvent::Modified` for the graph asset
is ignored — reloading a project is an explicit action. Content still
hot-reloads through the ordinary asset watcher.

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
after load overwrites it. The gizmo writes through `SetField`, exactly as the
inspector does.

Continuously driven transforms should write a
previous/next pair (`DrivenTransform`) and let a per-frame system lerp by
`Time<Fixed>::overstep_fraction`.

### Structural change

Spawning, despawning, and rewiring do not happen during a tick: they are
commands applied in `PreUpdate`. The next `FixedUpdate` rebuilds before
ticking. The document uses stable string ids; `NodeId` is runtime-only. Load
mints a `HashMap<FileId, NodeId>` once — there is no `claim.rs` and no
four-pass reconcile against a parallel entity world. Deleting a node despawns
its projected entity and releases its asset.

Observers may spawn and mutate the world freely but must not write the graph
mid-tick.

## 8. Crate layout

```
sway-gpu          wgpu instance/device/queue — bevy↔vello pin lives here
sway-graph        Graph resource, NodeId, commands, order, tick
                  (no MIDI, no document, no bevy_render)
sway-events       event wires, per-wire buffers, pre-tick clear/copy
sway-document     version 3 on-disk format; stable ids; load/save the Graph
sway-nodes        built-in value node kinds (Oscillator, Lfo, Math, …)
sway-midi-core    MIDI IO, typed messages, PulseClock (no Bevy)
sway-midi         Bevy plugin, Transport snapshot, MidiTime
sway-geo          Geometry attribute tables and CPU operators
sway-runtime      headless Bevy app; services; pipelines; editor viewport plugin
                  (camera, picking, gizmo input) depends on sway-graph
sway-editor       masonry UI; links the runtime directly
sway-app          host: winit, device, editor shell or show presenter
```

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
- **Events** — clear/copy/clear-out invariants; fan-out isolation per wire
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
- Events as in §3; value wires and event wires are distinct. Event payloads are
  generic (`TriggerOut<P>`).
- Document lives outside `sway-graph`; authoring is `GraphCommand`.
- MIDI IO and pulse-clock math live in `sway-midi-core`; the `Transport`
  snapshot and `MidiTime` live in `sway-midi`; the graph stays MIDI-free.
- Fixed graph tick retained for continuous values.
- Connections are typed at connect time from reflected field types. Transform,
  colour and tint inlets take `Vec3`, not per-axis floats; a `Vec3 { x, y, z }`
  value node with driveable components is what produces them. Genuinely scalar
  fields take floats.
- The palette lists registered node kinds from the type registry.

**Out of MVP**

- `Merge` / `Sum` node kinds (variadic inlets themselves are in the model).
- Restore authored value on disconnect (see §7 — on disconnect a field simply
  keeps whatever value the edge last wrote; there is no authored-value shadow
  to restore from).
- Geometry operators and the geometry cook path (§6).
- GPU-resident geometry operators / compute cook path.

**Open**

- None recorded for the graph model. Cycle members still tick; they read the
  previous tick's values.
