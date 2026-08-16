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

**Engine (`sway-graph`)** owns what makes the control graph a graph: the `Wire`
trait and behaviour registration, the wire and behaviour registries (for
rebuild, editor, and anything that enumerates types), Kahn rebuild into a flat
`GraphOrder` step list, the exclusive tick walk over that list, and rebuild
diagnostics. It does not own MIDI, pixels, event buffers, or the on-disk
document. It depends on Bevy's non-rendering subcrates — not `bevy_render`.
Connection storage, single-source, fan-out, and rewire are Bevy `Relationship`s;
rustc types `Source` / `Target`; the engine adds only **ordering** and
author-facing graph diagnostics. The authoring surface is the **ECS itself**
(spawn/despawn, insert/remove components and relationship wires). Show builds
omit topology-watch systems.

**Runtime (`sway-runtime` + host)** is the Bevy app that exists every frame:
world, render pipelines, animation systems, and a small set of service facades
(`PointCloudSet`, `SpriteLayers`, `Emitters`, `CameraRig`, `AnimationDirector`,
and similar) with owned invariants. Fire-and-forget intents from behaviours use
observer triggers into this layer. Rendering is headless to a texture; `sway-app`
owns winit and the wgpu device and presents either into the editor or fullscreen
HDMI. The runtime runs whether or not a graph is loaded.

**Wires, behaviours, and events** bridge engine and runtime:

- **Value wires** — `Relationship` components on the consumer; `propagate`
  writes source into target; must not write an equal value.
- **Behaviours** — only when output depends on a wired inlet in the same tick
  and must sit in dataflow order. Otherwise ordinary Bevy systems.
- **Events (`sway-events`)** — separate crate; see §3.

**Document (`sway-document`)** is out of `sway-graph`. It reads and writes the
world only through ECS authoring — no parallel snapshot model inside the engine.

**Supporting crates:** `sway-nodes` (built-in components, outlets, wires, and
behaviours), `sway-midi-core` (MIDI IO, messages, and pulse-clock math),
`sway-midi` (Bevy MIDI plugin, transport snapshot, and `MidiTime`),
`sway-geo` (geometry tables and CPU operators), `sway-editor` (masonry UI on
the live world), `sway-gpu` (single device-creation pin for the bevy↔vello
coupling).

## 2. Decoupling and the wire contract

### Central decoupling

The graph does two things only. It **declares structure** — what exists in the
scene and how it composes — and it **fires** — "burst here", "retarget that
colour", "start clip 3". It does not drive the world frame by frame. Structure
is rebuilt when topology changes; a fired event belongs to ECS systems from the
moment it lands.

Corollary: the graph is the nervous system, the Bevy world is the body.
Low-cardinality signals are small components on entities. High-cardinality data
(points, rigid bodies, particle lifetimes) lives in the ECS, parameterised by
the graph. Geometry is a component on an entity, never a value carried by a
connection. **Physics is never wired.**

### Value wires

There is no node type and no node instance. An entity is a graph vertex because
it carries components; a value connection is a component too.

```rust
pub trait Wire: Relationship {
    type Source: Component;
    type Target: Component<Mutability = Mutable>;
    const NAME: &'static str;

    fn propagate(src: &Self::Source, dst: Mut<Self::Target>);
}
```

The relationship component lives on the **consumer** and names the producer;
Bevy's `RelationshipTarget` on the producer collects consumers. **Outlets are
components** (an entity has an `f32` outlet because it has `FloatOut`). **Inlets
are wire types** (an entity has a `translation.y` inlet because it has
`Transform` and that wire's `Target` is `Transform`).

Every connection invariant that used to need an arena is a property of Bevy
`Relationship` (at most one source per inlet type, compile-time value typing,
direction, fan-out, rewire eviction, consumer despawn cleanup, no self-edges).
Those behaviours are pinned by characterization tests. A producer missing the
source component or a consumer missing the target is skipped at propagate time
and reported in diagnostics.

**Behaviours** are the second registered thing. Most computation is not a
connection and does not belong in the graph:

| What the output depends on | Where it runs |
|---|---|
| Only external state — `Time`, input | Ordinary Bevy system, before the tick |
| Nothing; it only consumes — mesh upload, material rebuild | Ordinary Bevy system on `Changed<T>` |
| A wired inlet, in the same tick | A **behaviour**, placed in the order |

Registration is per component type. Hierarchy costs one `Wire` impl on
`ChildOf` (`Source` and `Target` both `Transform`, empty `propagate`); authoring
inserts `ChildOf`, Bevy maintains `Children`.

`TickCtx` carries only what is specific to this tick — duration, start, index.
Wall time comes from `Time<Real>`. Beat time, when present, comes from a
resource registered by the MIDI family (§8), not from the engine. Behaviours
derive time-varying values from absolute time rather than accumulating per tick.

### Runtime surface

Mechanically a behaviour receives `&mut World` and can touch anything. By
convention it goes through registered service resources. A wire's `propagate`
sees one source component and one target component only.

Fire-and-forget interactions ("burst here", "start clip 3") use **observer
triggers** rather than facade methods — graph → world only. Observers are not
used to carry connections.

Components and wires are strongly typed. There is one component type and one
wire type per material or light kind, matching Bevy's own types where applicable.

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
- Registration monomorphises the clear and copy functions exactly as
  `register_wire` does, so the tick never sees a generic.
- Event wires round-trip through the document like value wires, so there is a
  **separate event-wire registry** alongside the value-wire one. Drag-to-connect
  legality reads both.

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

One derived artifact exists — a flat list:

```rust
#[derive(Resource, Default)]
pub struct GraphOrder { pub steps: Vec<Step> }

pub enum Step {
    Propagate { run: PropagateFn, src: Entity, dst: Entity, wire: &'static str },
    Run       { run: BehaviourFn, entity: Entity },
}
```

A rebuild collects every instance of every registered value wire, Kahn-sorts the
**entities** they connect, and emits, per entity in that order, inbound
propagations followed by behaviours. Each step carries a monomorphised function
pointer from registration; the tick never reads the registries.

### Rebuild

`GraphOrder` rebuilds when a `TopologyDirty` flag is set (starts set). Per-wire
type watch systems notice insert/remove and set the flag; they live in a system
set gated on an `Authoring` resource — **absent from a show build**. Authoring
is plain ECS mutation; there is no `connect` API.

A cycle never stops the render. The sort emits the acyclic part in topological
order and appends cycle members in entity order, where they read the previous
tick's value. Cycles and missing source/target components land in
`GraphDiagnostics` at rebuild time only. Ties break by ascending entity index
(Bevy's `Entity` `Ord` is descending in raw index; the sort compensates).

The topological sort stays ours: Bevy's `ScheduleGraph` orders systems per type,
not instances, and its errors would name systems rather than entities.

**Open:** vertices are entities, not `(entity, component)` pairs, so unrelated
components on one entity flowing in opposite directions can look like a cycle.
Cycles are allowed; no special case for false cycles for now.

### Tick

The graph runs as a single **exclusive system in `FixedUpdate`**, rate decoupled
from render framerate via `Time<Fixed>`. Serial evaluation, direct `&mut World`,
immediate writes within the tick.

`FixedUpdate` decouples tick rate from frame rate, not tick cost — a heavy cook
still hitch the frame; `max_delta` may drop ticks under sustained overload.
**Evaluation cost belongs to the graph author**, not the tool. The tool reports
cost rather than silently deferring work.

Behaviours are not per-type systems: instance order cannot be expressed as
system-order constraints without one-tick latency or a schedule traversal per
DAG level. A serial walk of the step list is the chosen semantics.

Consequences:

- Zero to n ticks per rendered frame; between ticks the world keeps animating.
- A recorded MIDI trace plus a fixed delta yields bit-identical tick sequences
  for golden-trace tests. Live overload that drops ticks may diverge — acceptable
  for an instrument, not an unqualified promise.

### Transport ownership

`sway-graph` is MIDI-agnostic. **`sway-midi-core`** owns MIDI IO, typed
messages, and the Bevy-free `PulseClock`. **`sway-midi`** owns the Bevy plugin,
its inbox and tick buffers, the `Transport` snapshot resource, and the
`MidiTime` source. Each fixed tick drains timestamped messages into
`PulseClock`, samples a fresh `Transport`, and writes `MidiTime` before the
graph tick. `sway-graph` owns no beat clock or transport type.

## 5. Editor integration and what Bevy owns

### Editor and runtime

The editor embeds a live runtime viewport in one process, sharing one wgpu
device. Bevy renders to an offscreen texture; a thin presenter composites into a
masonry widget when authoring or blits fullscreen on stage. Bevy runs headless;
the host owns winit and the device (`sway-gpu`) and drives `app.update()`.

The editor reads the world directly — entities, registered wire types, live
component values, `GraphOrder`, diagnostics — with no control socket and no
second schema. UI is retained-mode masonry; pan/zoom, bezier edges, and
hit-testing are hand-rolled.

**Risk taken deliberately:** masonry/Vello and Bevy must resolve the same wgpu
and winit. Mitigations: exact workspace pins, duplicate detection as a build
failure, device creation confined to `sway-gpu`, upgrade only when the known-good
tuple realigns. If that gate fails, the fallback is Syphon-style frame sharing —
not months of dependency patching.

### Ownership table

| Concern | Owner |
|---|---|
| Value/event connection storage, single-source, fan-out, rewire | `bevy_ecs` relationships |
| Entity lifecycle; consumer wires drop with consumer | `bevy_ecs` |
| Hierarchy / transform propagation | `bevy_transform` (`ChildOf`) |
| Value typing of connections | rustc (`Wire::Source` / `Target`) |
| Dirty reaction after writes | `Changed<T>` (+ no-equal-write rule) |
| Editor metadata / reflect payloads | `bevy_reflect` |
| Fixed tick rate / accumulator | `bevy_time` (`Time<Fixed>`) |
| MIDI IO, typed messages, pulse-grid clock math | **`sway-midi-core`** |
| Beat / transport snapshot + `MidiTime` | **`sway-midi`** |
| Topological order, step list, walk, graph diagnostics | **`sway-graph`** |
| Selection | **`sway-graph`** (`Selection` resource), read by editor via snapshot |
| Event-wire buffers + pre-tick clear/copy | **`sway-events`** |
| Document parse/emit | **`sway-document`** via ECS |
| Geometry tables / operators | **`sway-geo`** (CPU; dormant for the MVP, §6) |

`sway-graph` must not depend on `bevy_render`, MIDI types, or the document
format.

## 6. Scene composition

The scene is built by the graph, not loaded beside it. Camera, lights, meshes,
groupings, materials, and transforms are authored in the graph; content arrives
through `Asset` entities at the leaves. Ownership is total — teardown and reload
stay answerable.

The model is Houdini/USD-shaped: **operators act on streams, so cardinality
lives in the data, not the operator count.** A graph entity that also carries
`Transform` and/or `Geometry` is a scene entity — no handle map, no reconcile
pass.

### Components

- **`Geometry`** — named planar attribute tables (`P`, `N`, `Cd`, `pscale`, plus
  custom attributes). One map component, because authors can invent attributes at
  runtime.
- **`Transform` / `GlobalTransform`** — Bevy's local↔world pair.

### Connection kinds

| Connection | Mechanism | In the order |
|---|---|---|
| Parenting | `ChildOf` wire, empty `propagate` | yes |
| Geometry flow | wire delivering `Geometry` + behaviour that cooks | yes |
| Driving a value | value wire into a field | yes |

Object-level place/group/instance → `ChildOf`. Element-level scatter/noise/
displace → geometry wires. Driving colour/rotation/intensity → value wires.
Operators often carry `Geometry` without `Transform`; `Mesh` and `Asset` are
where geometry enters the scene tree. Materials are wired (one type per material
kind), not assigned — sharing is visible topology.

**Cook invalidation** is `Changed<T>`. Wires must not write equal values. No
cook-gate resource. Intermediate geometry on non-renderable entities stays
inspectable.

**MVP:** geometry *operators* are out of scope entirely — the MVP's target scene
uses asset meshes, so `Grid` / `Displace` / `Scatter` / `CopyToPoints`,
geometry-intermediate ownership, and mesh-identity fingerprinting all move
past it. `sway-geo` stays dormant. The `Geometry` component and the
geometry-flow connection kind remain part of the design; nothing in the MVP
produces one.

GPU-resident graph ops and mixed CPU/GPU residency remain out of scope.

## 7. Graph state and the ECS

Values, geometry, and transforms are components on ordinary entities. A wire
writes the real component; Bevy takes over from there.

```
PreUpdate     (authoring) topology watches mark dirty
FixedUpdate   MIDI feed → drain → sample Transport → write MidiTime
              rebuild GraphOrder if dirty
              sway-events: clear wire buffers → copy TriggerOut → buffers
              graph tick (Propagate / Run)
              sway-events: clear TriggerOuts
Update        Changed<T> reactions, services
PostUpdate    transform propagation, visibility
Extract/Render
```

Because the tick holds `&mut World`, writes are immediate within the tick.

### Never write an equal value

`get_mut` marks `Changed` unconditionally. `propagate` must use
`Mut::map_unchanged` and `set_if_neq` (or equivalent) so equal values do not
dirty downstream work. Every value wire type needs a test for that. Asset
mutation follows the same discipline outside the graph (`get`, compare, then
`get_mut`).

### Unconnected values

A connected wire overwrites the field each tick; on disconnect the field keeps
whatever arrived last. Restore-to-authored-on-disconnect is out of MVP.

There is no authored value distinct from the live value: the component *is* the
value, and `to_document(world)` serializes what the world holds. **Every
field is editable**, wire-driven or not — the inspector does not refuse a
driven field, and nothing renders inert (M6-5;
`docs/superpowers/specs/2026-08-10-m6-editor-write-half-design.md`). Editing a
driven field holds only until the next tick, when the wire writes over it
again. A save still bakes the instantaneous driven value into the file (a wire
targets a field path, but a component is emitted whole); harmless, since the
first tick after load overwrites it, and no machinery is built against it.
The gizmo (M7) writes through, exactly as the inspector does; a drag on a
wire-driven field holds for one tick.

Continuously driven transforms should write a
previous/next pair (`DrivenTransform`) and let a per-frame system lerp by
`Time<Fixed>::overstep_fraction`.

### Structural change

Spawning, despawning, and rewiring do not happen during a tick. On load, reload,
or editor edit, ECS mutations set the topology flag; the next `FixedUpdate`
rebuilds before ticking. Reload reconciles by document id inside
`sway-document`: surviving ids keep their `Entity` and runtime-attached
components; removed entities despawn; added ones spawn.

Observers may spawn and mutate freely but must not insert or remove wire
components mid-tick.

## 8. Crate layout

```
sway-gpu          wgpu instance/device/queue — bevy↔vello pin lives here
sway-graph        Wire trait, registries, order, tick, graph diagnostics
                  (no MIDI, no document, no bevy_render)
sway-events       event wires, per-wire buffers, pre-tick clear/copy
sway-document     on-disk format; read/write only via ECS authoring
sway-nodes        built-in components/outlets, wires, behaviours
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
- **Relationship semantics** — characterization tests the engine depends on.
- **Order** — determinism, cycle append behaviour, **one-tick** chain
  resolution.
- **Change detection** — each value wire leaves `Changed<Target>` false when
  the value is unchanged.
- **Events** — clear/copy/clear-out invariants; fan-out isolation per wire
  buffer.
- **Cooking** — pure geometry functions; unrelated changes recompute nothing.
- **Document** — round-trip; malformed input reports; reload keeps surviving
  `Entity`s and runtime components.
- **Runtime** — MIDI traces into a headless world; assert ECS state and service
  calls.
- **Rendering** — no pixel-diff tests; verify by eye.

## 10. Design decisions and MVP scope

**Settled**

- Evaluation cost is the author's problem; the tool reports rather than
  polices.
- Events as in §3; value wires and event wires are distinct. Event payloads are
  generic (`TriggerOut<P>`).
- Document lives outside `sway-graph`; authoring API is the ECS.
- MIDI IO and pulse-clock math live in `sway-midi-core`; the `Transport`
  snapshot and `MidiTime` live in `sway-midi`; the graph stays MIDI-free.
- Fixed graph tick retained for continuous values.
- Components and wires are strongly typed throughout. Transform, colour and
  tint inlets take `Vec3`, not per-axis floats; a `Vec3 { x, y, z }` value node
  with driveable components is what produces them. Genuinely scalar fields take
  floats.
- Component sets that belong together are declared with Bevy's `#[require]`,
  not with an editor-side template registry — so the palette can list component
  types straight from `ComponentDocRegistry` and still spawn a working node.

**Out of MVP**

- Variadic inlets (`Merge` / `Sum`).
- Restore authored value on disconnect (see §7 — on disconnect a field simply
  keeps whatever value the wire last wrote; there is no authored-value shadow
  to restore from).
- Geometry operators and the geometry cook path (§6).
- GPU-resident geometry operators / compute cook path.

**Open**

- Entity-level sort vertices can report false cycles when unrelated components
  on one entity flow in opposite directions. Cycles are allowed; no richer
  vertex representation yet.
