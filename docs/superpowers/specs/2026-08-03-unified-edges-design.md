# Unified edges — Design

**Date:** 2026-08-03
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §2.2, §2.4, §2.5, §2.6, §2.10, §2.11, §3, §5 (M4), §7
**Supersedes:** the three-edge-kind model of parent §2.10, the `Slots`/`Produces`
capability system of §2.4, and the two-pass compiler of §2.5
**Placement:** opens M4, before the RON schema is written

## 1. What this is

Sway has three edge kinds — param, `Feeds`, `ChildOf` — and a node describes its
connectivity through five unlike mechanisms: a `Params` struct, an `Outputs`
struct, a `Slots` struct, a single `Produces` associated type, and a
`SPATIAL: bool` const. This replaces all of it with one edge, one inlet
concept, two structs, and one compiled order.

The symptoms that prompted it are concrete. A `Feeds` edge has no source
endpoint to draw from, because a node's product is a single implicit thing
rather than a named outlet. A parent edge has no endpoint at either end, which
is why `snapshot.rs:230` drops it from the canvas rather than inventing
positions for it — **the graph the editor draws is not the graph that exists.**
And adding a node type means choosing among five declaration mechanisms whose
shapes have nothing in common.

The underlying cause is that connectivity was modelled three times, once per
edge kind, when it is one thing.

## 2. The model

**Every inlet is a value slot, holds exactly one authored or driven value, and
accepts exactly one edge.** There are no exceptions, no arity classes, and no
carrier taxonomy. What varies is the *type* of the value.

Three type shapes cover everything the current three edge kinds did:

| Inlet type | Holds | Replaces |
|---|---|---|
| plain reflect value (`f32`, `Vec3`, `Waveform`) | the value | a continuous param port |
| `Events<T>` | the occurrences that landed this tick, each with a sub-tick offset | an event port |
| `Product<T>` | `Option<Entity>` — the source node's entity | a `Feeds` slot / a `ChildOf` edge |

`Product<T>` is the load-bearing one. **The produced data never enters the
arena — only a reference does.** A `Product<Geometry>` inlet holds the entity
whose `Geometry` component the cook should read, so §2.1's rule that
high-cardinality data lives in the ECS rather than on edges is untouched. What
changes is that a structural connection is now an ordinary typed value, which
means the capability system is no longer needed: `Product<Geometry>` matches
`Product<Geometry>` by `TypeId`, through the same check that matches `f32` to
`f32`.

An unconnected `Product` inlet holds `None`, which is its authored value. §2.11's
authored-versus-driven rule therefore covers structural inputs with no special
case.

A `Product` **outlet** is not written by `tick`. The compiler seeds it with the
node's own entity once, because it is constant for the life of a compiled graph.

`Events<T>` replaces today's `Event<T>`, and the change is more than a rename.
`Event<T>` is a zero-sized marker whose occurrences live in a parallel arena;
`Events<T>` is an ordinary value holding `Vec<Occurrence<T>>`, where
`Occurrence<T> { offset: f32, value: T }` is **typed rather than boxed**. One
box for the list replaces one box per occurrence, which removes the
per-occurrence reflected clone M2a identified as the tick's dominant
allocation. That gain is what pays for the churn risk in §8, and the two must
be measured together.

### Multiplicity

A node's inlet *count* varies; an inlet's arity does not. Variable fan-in is a
`Vec` field:

```rust
struct GroupInlets  { children: Vec<Product<Spatial>>, translation: Vec3, .. }
struct MergeInlets  { inputs:   Vec<Product<Geometry>> }
struct SumInlets    { terms:    Vec<f32> }
struct MixerInlets  { triggers: Vec<Events<NoteMsg>> }
```

Each element is an independent single-source inlet. This is what makes fan-in
free of engine machinery: a `Vec<f32>` hands the node N contiguous arena slots
and its `tick` folds them however it wants, so no combining rule has to be
invented, and each element keeps its own authored value. A node wanting merged
event streams reads N single-source streams and merges them itself, in an order
that is authored rather than derived.

**A bare `Vec<T>` field means "N inlets", so a node cannot also carry an
authored array that is not connectable.** The escape is a named type —
`ControlPoints(Vec<f32>)` rather than `Vec<f32>` — and node authors have to
know the rule. An authored array nothing can drive is better served by a curve
or an asset anyway.

### Addressing

```rust
#[derive(Clone, Copy)]
pub struct Endpoint { pub field: u16, pub index: u16 }  // index 0 for non-Vec fields

#[derive(Component)]
pub struct Edge { pub from: Endpoint, pub to: Endpoint }
```

`EdgeFrom` / `EdgeTo` are unchanged, and keep their `linked_spawn` cascade.
`ParamEdge`, `FeedsEdge` and `ParentEdge` are deleted.

Addressing by `(field, index)` rather than a flat ordinal is what makes a
`Vec` resize local: inserting a child renumbers nothing outside that field, so
authored edges in RON and widget identity in the editor survive it. Field
ordinals stay pinned by a single `ORDINALS` const; element index is positional
within its field and needs no declaration.

### Node contract

```rust
trait NodeType: 'static {
    type Inlets:  Reflect + Typed + GetTypeRegistration + Component + Default;
    type Outlets: Reflect + Typed + GetTypeRegistration + Default;
    type State:   Component + Default;

    const ORDINALS: &'static [(&'static str, u16)];  // inlets then outlets
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);
    fn cook(_world: &mut World, _node: Entity, _ports: &PortView) {}
    fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> { None }
}
```

`Params`, `Outputs`, `Slots`, `Produces`, `SPATIAL`, `PORT_ORDINALS` and
`SLOT_ORDINALS` are gone. Direction comes from which struct a field is in, so
`Events<T>` and `Product<T>` are legal in both, exactly as a plain `f32`
already means an input in one and an output in the other.

**At most one `Product` outlet per node**, enforced at registration. That is
today's expressiveness under `Produces`, and it is what keeps
`produced_change_tick` a per-node function rather than a per-outlet table.
Lifting it is a contained change if `Asset` ever needs to produce both a
subtree and geometry.

## 3. What the engine knows about types

The carrier discrimination dissolves into the type system, but not to nothing.
Two facts survive, and naming them honestly is better than pretending the model
is uniform when it is not.

**Occurrence lists are emptied before each tick.** Otherwise a node that stops
writing its event outlet leaves the last occurrence firing forever — a silent
failure, and the exact class of bug the current `PortArena::clear_events`
prevents. `register_event_port::<T>` registers a `ReflectOccurrenceList` type
data carrying `clear: fn(&mut dyn PartialReflect)`; compilation collects one
`(slot, clear_fn)` list for the whole graph and the tick runs it first. This
preserves the property that makes today's layout fast — each list keeps its
allocation across ticks — without a second arena.

**`Spatial` is a capability the engine acts on**, in three ways:

1. an edge into a `Product<Spatial>` inlet also emits Bevy's `ChildOf`;
2. a `Product<Spatial>` outlet may feed **at most one** inlet, because
   `ChildOf` is a one-parent relationship. Outlets otherwise fan out freely,
   so this is the one outlet-side arity rule, and it carries today's "one
   parent" error wording;
3. `Spatial` edges are **excluded from the compiled order** (§5).

Three special behaviours on one capability is a real cost, and it is the price
of Bevy owning the scene hierarchy. It is contained: one capability, one place
in the compiler, one paragraph here.

## 4. Compilation — one pass

```
expand      per node: walk the registered Inlets/Outlets types; read Vec
            lengths off the instance → inlet layout and arena offsets
validate    per edge: type match, direction, inlet-already-connected;
            Spatial single-consumer and parenting acyclicity
order       one topological sort over every edge except Spatial → order
emit        ChildOf for Spatial edges; seed Product outlets with their entity
```

**One sort, not two.** `cook_order` and `tick_order` merge, because a
`Product` edge now *is* a data dependency expressed as a value — the consumer
reads something the producer computed. §2.5's argument for keeping structure
out of the sort survives exactly where it was true and no further: a parent
does not read anything from its child, so `Spatial` edges are excluded. That
exclusion is not cosmetic. Including them would reject a legitimate graph — a
child node driving a param on its own parent — which today's separate sorts
accept.

**The union of the old two DAGs can contain a cycle where neither did alone,
and those graphs are now rejected.** This is a deliberate behaviour change. A
graph where A feeds B and B drives a param on A is a genuine circular
dependency; today it compiles and one side silently reads stale data because
phase ordering resolves it. An error is the better outcome.

**Error vocabulary is preserved by construction**, because the type of every
edge in a diagnostic is known before the message is built. `DuplicateContinuousInput`,
`DuplicateSlot` and `DuplicateParent` collapse into one `InletAlreadyConnected`
that names the field and its type. A cycle whose edges are all `Product<Spatial>`
is reported as a parenting cycle in parenting's vocabulary. §2.5's requirement —
that every load failure produce a clear, node-attributed message in the
vocabulary of what failed — is met by inspecting types rather than by keeping
three code paths.

## 5. The tick

One exclusive system in `FixedUpdate`, one compiled order, and each node ticks
and then cooks when its turn comes:

```
clear       occurrence-list slots (§3)
per node, in compiled order:
  gather    copy each connected inlet from its source's outlet slot
  tick      node reads inlets, writes outlets and its own components
  cook      if the gate says dirty and the node COOKS
```

The two-phase split — every tick, then every cook — is deleted. This closes
parent §7's open question about a node whose tick depends on a cook from the
same tick: `Grid → PointCount → Displace.amount` is now expressible, because
the edges order it.

It also replaces a guarantee-by-phase with a guarantee-by-edge. M2b relied on
ticks globally preceding cooks so that a material node's handle was written
before `Mesh`'s cook read it; under one order the material→`Mesh` edge
guarantees it directly, which is the same result for a reason an author can
see in the graph.

The cook gate itself is unchanged: sticky, per node, comparing a source's
`produced_change_tick` against the tick stored in `State`. Its source entity now
comes from the inlet's arena slot rather than a slot table.

## 6. Editor

`EdgeView` carries both endpoints, so the canvas draws every edge socket to
socket, parenting included. Node widgets render sockets from the snapshot
rather than from the registered type — which they must anyway, now that inlet
counts are per-instance. `snapshot.rs:230`'s deliberate `ParentEdge` skip is
deleted, along with `EdgeKind`'s comment explaining why parenting is absent.

Both views keep their jobs: the tree pane shows the hierarchy as a tree, the
canvas shows it as edges. That is not duplication — a deep hierarchy reads
better as a tree, and its relationship to the rest of the graph reads better as
edges.

## 7. What this deletes

- `ParamEdge`, `FeedsEdge`, `ParentEdge`, and `PortKind`
- `Slot<T>`, `ReflectSlot`, `SlotField`, `derive_slots`, `SlotView`,
  `SlotSource`, and `slots.rs` entirely
- `Produces`, `produces`, `produces_path`, `SPATIAL`, `SLOT_ORDINALS`
- `structure.rs`'s parallel validation pass and its slot tables
- `compile.rs`'s `event_merges` (583-614) and `tick.rs`'s merge loop (152) —
  merging moves into whichever node wants it
- `NodePlan::cook_order` and the second topological sort
- `PortArena::events` as a separate collection, the `ContinuousIdx` /
  `EventIdx` split, and the boxed `Occurrence::value`
- three duplicate-connection error variants, collapsed to one

## 8. What it costs

**Allocation churn on event slots is the one place a merged arena is worse.**
Today's `Vec<Vec<Occurrence>>` clears in place and keeps every allocation;
a `Box<dyn PartialReflect>` holding a list does not, unless cleared through the
`ReflectOccurrenceList` fn pointer of §3 rather than replaced. M2a already
found the event path is the tick's main allocator, so this must be implemented
as clear-in-place from the start, not fixed later. If it still regresses,
the fallback is a typed column for list-shaped ports behind `PortView`, which
the engine boundary already permits.

**Per-instance inlet counts revise §2.4.** That section designs variable arity
out and states that the compiler never evaluates a per-instance schema. The
schema stays per-type and constant — which fields exist, their types, their
ordinals — and only one number, a `Vec` field's length, is read from the
instance, at compile time rather than per tick. §2.4 needs rewriting in any
case, since it resolves variable arity by routing `Merge` through `ChildOf`,
which this design replaces.

**Graphs that compile today may not after.** Two cases: a union-DAG cycle
(§4), and any node relying on implicit event fan-in. `traces.rs:323`'s
`event-fan-in` fixture wires two `MidiNote` nodes into one `Envelope.trigger`,
and `compile.rs:770` asserts that is legal; `Envelope` must declare
`triggers: Vec<Events<NoteMsg>>` and merge them itself.

**The refactor is wide.** It touches every node type, `sway-graph`'s registry,
schema, compiler, arena, tick runner and views, `sway-editor`'s snapshot and
canvas, and the demo graph. It is mechanical, but it is not small.

## 9. Parent spec revisions this forces

Recorded here so the parent document is corrected rather than left contradicted:

- **§2.4** — variable arity is declared rather than designed out; the port
  type registry subsumes capabilities; a registry entry is constant per type
  except for one per-instance count.
- **§2.5** — one pass and one sort, not two; the structure/dataflow split is
  replaced by "everything except `Spatial` is a dependency".
- **§2.10** — the three-edge-kind table becomes one edge; `Feeds` and `ChildOf`
  become inlet types; the rule that tells an author which edge they want
  becomes a rule about which inlet type a node declares.
- **§2.11** — step A and step C merge; the arena carries entity references as
  well as signals.
- **§7** — the same-tick cook dependency question is closed by §5 above.
- **§5 (roadmap)** — M4 opens with this work.

## 10. Placement and migration

**Opens M4, before the RON schema is written.** The format would otherwise
encode three edge kinds and five declaration mechanisms and then have to break
them, and only one serializer should ever be written.

M3 runs on the current model and migrates cheaply: its nodes are signal-only,
so each loses two structs and two consts and gains two structs and one const,
with no slots, no capability and no parenting to rework.

No compatibility shim. Both models live at once for the duration of one
milestone's branch and never in `main`.

## 11. Testing

Following parent §4, and its rule that rendering is verified by eye.

**The acceptance criterion is preservation, not new behaviour.** Every existing
compile-error test must survive in meaning: a second parent still says "one
parent", a slot type mismatch still names the expected and produced types on
their own sides, a dataflow cycle still says cycle and names the blocked set.

**The strongest single regression is `event-fan-in`.** With `Envelope`
declaring `triggers: Vec<Events<NoteMsg>>` and merging in its own tick, the
golden trace must reproduce **bit-identically**. That proves the merge moved
from engine to node without changing semantics, on a real MIDI trace.

Also required:

- an edge whose endpoint types differ is rejected naming both types;
- an edge whose `from` names an inlet, or whose `to` names an outlet, is
  rejected as a direction error;
- resizing a `Vec` inlet field leaves every other field's addressing untouched,
  and edges into surviving elements still resolve;
- a `Product<Spatial>` outlet feeding two inlets is rejected in parenting's
  vocabulary;
- a `Spatial` edge does not constrain the compiled order — the child-drives-parent
  graph of §4 compiles;
- a union cycle across what used to be two DAGs is rejected with both edge
  types named;
- an occurrence list is empty at the start of every tick, and its allocation
  survives — asserted by capacity, not by timing;
- the M2b cook-gate suite passes unchanged, including that an unrelated param
  change cooks nothing and that a node added mid-session still cooks.

## 12. Out of scope

Topology editing (M7). Serializing this model to RON — M4 does that
immediately after, but it is a separate task with its own decisions. Per-outlet
`produced_change_tick`. `Vec` fields in `Outlets`. Engine-side combining rules
for `Vec` value inlets. Any change to the cook gate's own logic, which M2b
proved and this design only re-plumbs.

## 13. What this milestone must produce besides code

A findings report at `docs/superpowers/reports/2026-08-03-unified-edges-findings.md`
answering:

1. Did clear-in-place hold the event path's allocation profile, measured
   against M2b's `graph_tick` figures on the same graph and the same
   `--test-threads=1` discipline?
2. Did one order cost anything real — did any graph in the node set want the
   two-phase split back, and did any legitimate graph get rejected as a union
   cycle?
3. How did `(field, index)` addressing read at the call site, and did the
   registration guard still catch a field reorder now that it pins field
   ordinals only?
4. Did `Product<T>`-as-entity-reference remove the capability system cleanly,
   or did something have to be reintroduced to type-check structural edges?
5. What did `Spatial`'s three special behaviours cost in practice — is one
   engine-known capability the right shape, or did a second one want the same
   treatment?

If any decision here turns out wrong in implementation, this document gets a
**Revision** line at the top, in the style the parent spec and the M2a/M2b
designs use.
