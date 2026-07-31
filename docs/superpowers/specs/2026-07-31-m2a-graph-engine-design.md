# M2a — Graph engine core and signal nodes — Design

**Date:** 2026-07-31
**Status:** Approved, pre-implementation
**Revision:** implementation showed that ordinal checks must match `(name, ordinal)` when an input and output share a name, reflected prefill must use `reflect_clone()` rather than `to_dynamic()` to preserve concrete enums, `Envelope` needs a separate `release_trigger` event input, `PortView` needs explicit bounds, edge direction needs compile-time validation, Kahn's remainder is not exactly the cycle, and event fan-in still allocates temporary clones (see `docs/superpowers/reports/2026-07-31-m2a-graph-engine-findings.md`)
**Parent spec:** `2026-07-25-sway-design.md` §2.1–§2.6, §2.11, §3, §4, §5 (M2), §7

## 1. What this milestone builds

The parent spec's M2 bundles four separable things: the engine core, a signal
node set, the geometry/cook path, and a reflect-driven debug inspector. That is
more than one spec's worth, and the pieces fail differently — an engine that
cannot order its nodes is a redesign, a cook gate that invalidates too eagerly
is a performance bug. **M2 is therefore split, as M1 was.**

**M2a — this document — is the engine core plus the signal node set.**

In scope:

- The `NodeType` contract, the node type registry, and reflect-derived port
  schema
- The port arena, the `Continuous` / `Event` port kinds, `PortView`, `TickCtx`
- **Param edges only** — edge entities, type validation, topological sort
- The `FixedUpdate` tick runner as one exclusive system
- Eight signal nodes: `MidiNote`, `MidiCC`, `LFO`, `Envelope`, `Math`, `Remap`,
  `Switch`, `Select`
- MIDI ingress carrying real sub-tick offsets
- The golden-trace harness and the compiler failure tests
- Retiring `crates/sway-app/src/graph.rs` behind a throwaway arena→cube bridge

Deferred to **M2b**: `ChildOf` and `Feeds` edges with the structure validation
pass, the `Geometry` component, cook gating on change ticks, the reflect-driven
debug inspector, and driving M1's visuals.

**Exit:** a graph built in Rust from live CoreMIDI input drives the cube; golden
traces replay to bit-identical output; every compiler failure mode produces a
node-attributed error message.

**Not closed by this milestone:** §7's fixed tick rate. See §11.

## 2. Crate layout

Two new crates, following the parent spec's §3:

```
sway-graph   port kinds, arena, PortView, node type registry, NodeType,
             param edges, the dataflow compiler pass, the tick runner
sway-nodes   the eight signal node types; depends on sway-graph
```

`sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_reflect` and `bevy_time` —
not on `bevy`, and specifically not on `bevy_render`. The parent spec's §3 also
lists `bevy_transform` and `bevy_asset`; both join at **M2b**, where scene nodes
need them. Adding them now would put a dependency in the manifest that no code
uses, and the manifest is the only place this constraint is actually enforced.

`sway-graph` knows nothing about MIDI. `MidiNote` and `MidiCC` live in
`sway-nodes` and read a resource that `sway-app` fills — the boundary
`crates/sway-app/src/graph.rs:16` already anticipated in its own comment.

## 3. The node contract

```rust
trait NodeType: 'static {
    type Params:  Reflect + Component;   // authored inputs
    type Outputs: Reflect;               // output port schema
    type State:   Component + Default;   // per-instance runtime state

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);
}
```

`Params` and `State` are as the parent spec's §2.2 defines them. **`Outputs` is
new**, and closes a gap §2.4 leaves: it derives a node's schema from its params
struct and says inputs are its fields, but an `LFO`'s output is not an authored
value and cannot live in `Params` without breaking §2.11's authored-versus-driven
rule. A second associated type keeps the registry entry a constant derived from
types alone — no hand-written schema reappears.

`Outputs` is a schema, not storage. It is never inserted as a component; output
values live in the arena.

### Field type decides port kind

A plain field (`hz: f32`) is a **`Continuous`** port whose authored value is the
field itself, and on which `#[reflect(@Range(..))]` attributes work as §2.4
describes. A field typed `Event<T>` is an **`Event`** port: zero-sized in the
struct, connect-only, no authored value. The same rule applies to `Outputs`.

This is what keeps §2.4's claim literally true — the schema is derived from the
types, never written beside them.

### Port indices are positional, and checked

The two port kinds occupy separate index spaces (§4). Within each kind, a node's
ports are laid out contiguously — its inputs in `Params` field order, then its
outputs in `Outputs` field order — so one base per kind plus the schema's input
count locates every port. Node code refers to its ports through index consts
declared next to its structs.

Those consts are a hazard: reorder two fields and two ports silently swap. So
`register` verifies them — it walks the reflect fields, computes the same
per-kind ordinals, and matches each declaration by `(name, ordinal)`, not name
alone. The tuple is necessary because a node such as `Remap` can legitimately
have an input and an output with the same name. Registration fails at startup if
a node's consts disagree. A startup panic is the right failure for a wiring
mistake that would otherwise show up as an LFO modulating the wrong parameter.

A `#[derive(NodePorts)]` macro generating those consts is the obvious later
cleanup. A proc-macro crate is not M2a scope, and the registration check makes
the hand-written version safe in the meantime.

### Dispatch

`register` erases `tick` to a bare
`fn(&mut World, Entity, &mut PortView, &TickCtx)` stored in the registry
alongside the derived schema, and the tick loop dispatches through it. There is
no `NodeInstance` trait object — parent spec §2.2, unchanged.

## 4. The port arena

Two collections, two index spaces:

```rust
#[derive(Resource)]
struct PortArena {
    continuous: Vec<Box<dyn PartialReflect>>,   // persists across ticks
    events:     Vec<Vec<Occurrence>>,           // cleared at tick start
}

struct Occurrence { offset: f32, value: Box<dyn PartialReflect> }
```

`Box<dyn PartialReflect>` is the parent spec's §2.4 choice, taken as written.
Typed columns keyed by `TypeId` would remove the per-value allocation and the
downcast, and were considered; §2.1's "the graph does not need to be fast" is
what buys the simpler representation, and the typed `PortView` surface below is
what keeps the decision reversible without touching a single node.

**The two kinds are stored separately rather than as one `Vec<Slot>` enum**,
because nothing iterates slots kind-agnostically: clearing, gathering,
prefilling and reading all branch on the kind, so an enum would buy a
discriminant and a match arm at every access and nothing else. Separating them
produces three concrete wins. Clearing is one pass over one collection that
retains each destination vec's allocation, so clearing itself does not churn
allocations after warm-up, and the continuous side is never touched. Event
fan-in still uses a temporary `Vec<Occurrence>` and clones reflected payloads
while gathering because source and destination borrow the same collection. The
edge plan splits into two branch-free loops instead of one that re-decides the
kind per edge. And
"an event input has no authored value" becomes structural — the prefill code has
no access to event storage — rather than an unreachable match arm someone could
later make reachable.

The cost is that a node's arena position is two bases rather than one, and its
index consts are ordinals **within a kind** — the second continuous input, the
first event input — rather than raw field positions. That is less obvious to
read off a struct, which is precisely why §3's registration check exists.

`ContinuousIdx` and `EventIdx` are distinct newtypes, so reading a continuous
port as an event stream is a type error rather than a runtime panic.

### `PortView`

The runner takes the arena out of the world for the tick's duration
(`resource_scope`), which is what lets a node hold `&mut World` and
`&mut PortView` at once — §2.4's stated reason for the arena being a resource
rather than components.

`PortView` is **scoped to the node being ticked**: it carries the arena, that
node's two bases, the per-kind lengths, and its connected-mask. The explicit
length checks are necessary; bases alone would let an out-of-range ordinal land
in the next node's slots. A node's indices are its own, and it cannot reach
another node's ports by arithmetic accident.

```rust
let hz: f32 = ports.read(Lfo::HZ);
ports.write(Lfo::OUT_VALUE, phase.sin());
for ev in ports.events::<NoteMsg>(Envelope::TRIGGER) { /* ev.offset: f32 */ }
ports.emit(MidiNote::OUT_NOTE_ON, offset, msg);
```

The surface is typed. The `Box<dyn PartialReflect>` representation is an
implementation detail of `sway-graph`, and swapping it later is one file's
internals.

### Continuous persists, events clear

A continuous slot always holds a current value; an event slot holds zero or more
occurrences for **this tick only**. That is §2.4's distinction made concrete:
"CC is 0" is a slot holding `0.0`, "no CC arrived" is an empty event vec. Without
the split there is no way to express the difference.

### Authored-versus-driven shadowing

§2.11 requires that a connected port *shadow* the authored value rather than
overwrite it. Implementation: an **unconnected** continuous input has its slot
filled from the node's `Params` component; a **connected** one is filled by its
edge. `Params` is never written by the graph.

Three consequences, all of them the ones §2.11 wants: disconnecting a CC from a
parameter returns it to its authored value rather than freezing where the CC
left it; saving a project cannot bake in whatever the LFO happened to be; and
the inspector can show authored and live values at once.

**Prefill is gated on the params change tick, not on `Changed<Params>`.** The
compiler inserts an engine-owned `NodeRuntime { continuous_base, event_base,
last_params_tick }` component — one base per kind, with the input/output split
inside each read off the schema (§3) — and prefill compares against
`get_change_ticks::<Params>()` on the node entity. A `Changed<Params>` filter
would be wrong for exactly the reason §2.11 gives about cook gating: it means
"changed since this system last ran", the tick system runs every tick, so the
flag is true for one tick only and a node that skips prefill on that tick for
any other reason misses the change permanently. Comparing stored ticks is robust
regardless of cadence.

Because continuous slots persist, a gated prefill that does not run leaves the
correct value in place. Compilation resets `last_params_tick`, which is what
makes a disconnect take effect on the next tick.

Prefill clones reflected fields with `reflect_clone()`, which preserves the
concrete type. `to_dynamic()` is not equivalent: for reflected structs and
enums it can produce a `Dynamic*` proxy that no longer downcasts to the node's
declared port type.

## 5. Param edges and compilation

An edge is an entity carrying source and target relationship components with
their port indices, per §2.4. Bevy maintains the reverse index, and despawning a
node despawns its edges — which is what stops M7's delete from leaving a
dangling reference.

```rust
fn compile(world: &mut World) -> Result<CompiledGraph, CompileError>
```

The compiler **reads the world**. Nodes are spawned as entities carrying their
`Params`, `State` and a `GraphNode { id, node_type }` marker; `compile` collects
them, validates, and produces the compiled order plus the arena layout. At M4
the project loader spawns entities from RON and calls the same function, so the
file format arrives as a deserializer rather than as a second compiler.

M2a runs **only the dataflow pass**. The structure pass — `ChildOf` and `Feeds`,
with parenting cycles, fan-out, and slot typing — is M2b's, and §2.5's reasoning
for keeping the two passes separate is why it can be added without disturbing
this one.

### Fan-in, which §2.4 leaves open

§2.4 states fan-out is legal and says nothing about fan-in. This milestone
decides it:

- A **continuous** input takes exactly one edge. A second is a compile error;
  "which one wins" has no defensible answer, and silently picking one would make
  a graph's behaviour depend on edge creation order.
- An **event** input takes many. Occurrences merge sorted by
  `(offset, source's compiled index)` — in time order, with a deterministic
  tiebreak.

### Failure modes

Every one produces a message naming the offending node, per the parent spec's
§4:

| Failure | Message names |
|---|---|
| Unknown node type | the node, and the unregistered type path |
| Port index out of range | the node, the port, and the schema's arity |
| Type mismatch across an edge | both nodes, both ports, both types |
| Source is an input, or target is an output | the node, port, kind, name, and expected direction |
| Second edge into a continuous input | the target node and port, and both sources |
| Edge referencing an absent node | the edge and the missing endpoint |
| Cycle | every node left with nonzero in-degree after Kahn's algorithm (the cycle and possibly downstream nodes) |

The topological sort is Kahn's, and ours. §2.5's reasoning against borrowing
Bevy's `ScheduleGraph` — one system per node *instance*, and errors naming
systems rather than nodes — holds unchanged.

**All failure happens at compile. The tick is infallible**: after validation
every edge copy is a type-checked `apply` that cannot fail.

## 6. The tick

One exclusive system in `FixedUpdate`, per the parent spec's §2.6. Per tick:

```
clear event slots
for node in compiled order:
    gather:   copy each incoming edge's source slot into the input slot
              (continuous overwrite; event append, merged per §5)
    prefill:  unconnected continuous inputs from Params, if the change tick moved
    tick:     registry fn(&mut World, node, &mut PortView, &TickCtx)
```

```rust
struct TickCtx { dt: f32, tick_start: f64, tick_index: u64 }
```

`tick_start` is the tick window's absolute start, in seconds. It is the second
half of the offset decision in §7: offsets stay bounded and f32-precise in the
arena, and a node needing absolute time writes `ctx.tick_start + offset as f64`.
Wall time otherwise comes from `Time<Real>`; `Time<Transport>` arrives at M3.

Because the system is exclusive and holds `&mut World`, **writes are
immediate** — a node later in topological order sees an earlier node's component
writes within the same tick. Nothing routes through `Commands`, which would add
a flush boundary and a tick of lag (§2.11).

Two rules M2a's nodes are held to, both from failures already recorded in this
repository:

- **Derive time-varying values from absolute time; never accumulate per tick.**
  `crates/sway-app/src/graph.rs:53` documents its own violation of this and the
  drift it causes under `Time<Fixed>::max_delta` tick-dropping. Its replacement
  must not repeat it.
- **No node fires observer triggers at M2a.** §2.11's node→world path exists to
  reach service facades, which do not exist until M2b/M5. Adding the mechanism
  before it has a consumer would make its constraints — notably that an observer
  must not touch the arena, which is not in the world during the tick —
  untestable.

## 7. MIDI ingress and sub-tick offsets

`sway-midi`'s `read_proc` is a realtime callback and keeps storing raw
`host_time`, converting nothing. The `mach_timebase_info` conversion to seconds
happens at drain, on the main thread.

The resource the MIDI nodes read lives in `sway-nodes`:

```rust
#[derive(Resource, Default)]
struct MidiInbox { events: VecDeque<(f64 /* seconds */, RawMidi)> }
```

`RawMidi` is `sway-nodes`' own three-byte struct, **not**
`sway_midi::MidiEvent`. That keeps `sway-nodes` free of a macOS-only FFI crate
and testable anywhere; `sway-app` converts as it drains the channel in
`PreUpdate`.

### Offsets, not absolute timestamps, in the arena

Both were considered. An offset is bounded — at 120 Hz it is `0.0..0.0083`, so
f32 has precision to spare, where an absolute time in f32 seconds is unusable
after minutes of runtime. It is self-contained: the arena's event slots are
cleared each tick, so an offset needs no reference clock to interpret, and a
recorded trace is tick-relative and therefore comparable without re-basing.

What absolute timestamps have over offsets — that they are what nodes actually
consume — is recovered by `TickCtx::tick_start` in one addition, at the node,
where f64 is free.

### The window rule

A tick takes events with `t <= tick_start + dt` and stamps each with
`offset = (t - tick_start).clamp(0.0, dt)`. Later events stay buffered for the
next tick. An event that arrived before the window — a late drain — clamps to
offset 0 rather than being dropped or given a negative offset.

**Replay writes `MidiInbox` directly with explicit times**, so
`mach_absolute_time` is on the live path only. A recorded trace replays without
touching CoreMIDI or the system clock, which is what makes §9's traces exact
rather than approximate.

## 8. The node set

| Node | Inputs | Outputs |
|---|---|---|
| `MidiNote` | `channel`, `note_range` | `note_on: Event<NoteMsg>`, `note_off: Event<NoteMsg>` |
| `MidiCC` | `channel`, `cc` | `value: f32` (0..1) |
| `LFO` | `hz`, `shape`, `phase`, `amplitude` | `value: f32` |
| `Envelope` | `trigger: Event<NoteMsg>`, `release_trigger: Event<NoteMsg>`, `attack`, `decay`, `sustain`, `release` | `value: f32` |
| `Math` | `op`, `a`, `b` | `value: f32` |
| `Remap` | `value`, `in_min`, `in_max`, `out_min`, `out_max`, `clamp` | `value: f32` |
| `Switch` | `select: bool`, `a`, `b` | `value: f32` |
| `Select` | `trigger: Event<NoteMsg>`, `field` | `value: f32` |

`Math` and `Switch` are binary and compose, per §2.4's design-out of variable
arity: `Switch(s1, Switch(s2, a, b), c)` is the three-way case, and needs no
count param.

**One definition this milestone invents, flagged rather than smuggled.** The
parent spec's roadmap names both `Switch` and `Select` without distinguishing
them, and §2.4 pins only `Switch`, as the multiplexer. `Select` is defined here
as the **event→continuous latch**: it samples a chosen field of the most recent
`NoteMsg` — note number, velocity — and holds it. That is the missing piece
between the event world and the continuous world, and nothing else in the set
provides it. If the roadmap meant something else, this is the line to correct.

`LFO` and `Envelope` are where §6's absolute-time rule bites. `Envelope` stores
the note-on time as `ctx.tick_start + offset` in its `State`, and its output is a
pure function of `now - t0`. That is what makes §7's sub-tick work observable
rather than decorative — and what §9's discrimination test checks.

## 9. Testing

Following the parent spec's §4, which makes golden traces the engine's primary
strategy.

- **Golden traces.** A RON input file (timestamped MIDI) and a RON expected file
  (per-tick arena snapshot) per case, under `tests/traces/`. `SWAY_BLESS=1`
  regenerates the expected files. Diffs are readable in review, adding a case is
  adding a file, and the input format rehearses M4's RON project format. Inline
  const arrays — M0's approach at `crates/sway-app/src/graph.rs:188` — do not
  survive a multi-port arena over hundreds of ticks; a single hash per case
  would tell us that something changed and not which port or which tick, which
  is the information a regression actually needs.
- **Determinism.** The same trace run twice is bit-identical. Carried over from
  `graph.rs:161`, which already got this right, and still necessary: a trace
  compared only against itself would pass while the values were wrong, which is
  why it does not replace the golden files.
- **Sub-tick discrimination.** Two notes landing in the *same* tick at different
  offsets must produce different envelope values. This is the test that fails if
  offsets silently become zero; without it, §7 is unfalsifiable.
- **Compiler failure table.** One case per row of §5's table, each asserting the
  error names the offending node.
- **Shadowing.** A connected port overrides the authored value; disconnecting and
  recompiling returns it to the authored value; `Params` is never mutated by the
  graph.
- **Event merge order** is stable given the same graph and the same inputs.
- **Registration check.** A node type whose index consts disagree with its
  reflect fields fails at startup — proving §3's guard is not vacuous.
- **Prefill gating.** Three cases, because this is the §4 hazard that fails
  silently: a params write is picked up even when many ticks elapse between the
  write and the next read; a node whose params have not changed does not
  re-prefill; and recompiling resets the gate, so disconnecting an edge takes
  effect on the following tick rather than leaving the last driven value in the
  slot. The first is what a `Changed<Params>` filter would get wrong.

## 10. The `sway-app` handover

`crates/sway-app/src/graph.rs` is deleted. `MidiRx` stays in `sway-app` and now
feeds `MidiInbox`. The cube is driven by a graph built in Rust —
`MidiNote → Envelope` — plus a bridge system that reads the envelope's output
port from the arena and writes the material.

The bridge is throwaway: M2b's scene nodes delete it. It earns its place for one
milestone because it is the difference between an engine that passes tests and
an engine that runs on real hardware with a real Octatrack plugged in, and M2a
otherwise has no live path at all.

The bridge follows §2.11's asset rule — `get`, compare, and only then
`get_mut` — because `Assets::get_mut` marks an asset changed by the act of
calling it, and a cube whose material is re-uploaded every tick while nothing
moves is the exact failure §2.11 names.

### Deliberate regression

`graph.rs`'s stored-trace test pins M0's linear decay
(`DECAY_PER_SEC`, `graph.rs:188`). The envelope replaces that behaviour, so
those expectations do not survive the move, and are not ported. The equivalent
coverage moves into §9's trace harness. Named here so a reviewer reads the
deletion as intended rather than as lost coverage.

## 11. What this milestone leaves open

- **The fixed tick rate.** `TICK_HZ = 120` carries over from M0 as provisional.
  §7 of the parent spec asks for a measured number, but M2a's graph is
  signal-only and cheap — cooks do not exist until M2b/M5 — so any measurement
  taken here would bound the floor and be mistaken later for the answer. The
  question stays open, explicitly, rather than being closed on evidence that
  cannot support it.
- **Reflect ergonomics under a real node set.** The parent spec's §7 says M2 is
  where this is found out. Eight node types carrying enums, ranges and event
  markers are the first real sample, so **M2a ends with a findings report**
  recording what resisted `Reflect` and what the workaround was. The stated
  fallback — a hand-written schema for the few types that resist, not abandoning
  the registry — is unchanged.
- **Event fan-in across recompiles.** §5 makes merge order deterministic within
  a compiled graph. Whether it should also be stable across a recompile that
  reorders nodes is a question M4's reload path is better placed to answer, and
  answering it now would be guessing at a constraint reload has not yet stated.

## 12. What this milestone must produce besides code

A findings report at `docs/superpowers/reports/2026-07-31-m2a-graph-engine-findings.md`
answering:

1. What resisted `Reflect` in a real node set, and what the workaround was
   (parent spec §7).
2. Whether `Box<dyn PartialReflect>` in the arena proved adequate, and what
   would have to be true to force typed columns (§4).
3. Whether the positional-index-const scheme held up across eight node types, or
   whether the derive macro should be pulled forward (§3).
4. What the tick actually costs at this cardinality — recorded as a data point
   for the tick-rate question, explicitly not as its answer (§11).

If any of §3–§7's decisions turn out wrong in implementation, this document gets
a **Revision** line at the top, in the style the parent spec and the M1b design
use. A design document that records what was believed beforehand and is never
corrected afterwards is worse than none.
