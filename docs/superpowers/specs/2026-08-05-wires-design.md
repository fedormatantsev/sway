# Wires: the graph as ECS relationships — Design

**Date:** 2026-08-05
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §2.1, §2.2, §2.4, §2.5, §2.6, §2.10, §2.11
**Prior:** `2026-08-03-unified-edges-design.md`, `2026-08-04-ports-as-component-values-design.md`
**Supersedes:** the node/edge model in its entirety — `NodeType`, `NodeTypeRegistry`,
`GraphNode`, `Edge`, `Endpoint`, `PortArena`, `PortView`, `Product`, `Spatial`,
`FieldKind`, `compile`, `prefill`, `seed_outlets`, `cook`, and the cook gate
**Placement:** replaces M2a/M2b's engine; precedes the RON schema (M4)

## 1. What this is

The engine built through M2a–M3 works, and the demo runs. It also generalises
three unrelated concepts through one mechanism: hierarchy, event propagation
and data flow all travel as edges between node entities, so every one of them
pays for a port arena, a slot-addressing scheme, a compile pass with per-kind
special cases, and a cook gate with its own dirty bookkeeping. `compile.rs` is
1132 lines; `tick.rs` is 614; `registry.rs` is 581. None of it is wrong. All of
it exists because a node is a thing *beside* the ECS rather than a thing *in*
it.

This design deletes the node layer. Entities and components are the graph.
A connection is a Bevy `Relationship` whose type carries what it does, and
`sway-graph` shrinks to a wire registry, a topological order, and a walk.

Four commitments frame it:

1. **No nodes.** Behaviour and data attach to ordinary entities through
   ordinary components. Nothing wraps a `Mesh3d` to make it connectable.
2. **No edge entities.** A connection is a relationship component on the
   consumer. `ChildOf` is already exactly this, so hierarchy needs no
   mechanism of its own.
3. **The graph propagates and orders; it does not cook or compile.** What a
   connection *means* lives in the connection's own type.
4. **Main world only.** No GPU work, no render-world coupling, no asset
   uploads inside the graph.

## 2. Core model

### 2.1 `Wire`

```rust
/// A connection type. Its Relationship component lives on the CONSUMER and
/// names the producer; the RelationshipTarget on the producer collects its
/// consumers.
pub trait Wire: Relationship {
    /// The component read on the producer. Also the legality rule: this wire
    /// may only originate at an entity that has one.
    type Source: Component;
    /// The component written on the consumer.
    type Target: Component;

    /// Display name, for the editor.
    const NAME: &'static str;

    /// The entirety of this connection's behaviour.
    fn propagate(src: &Self::Source, dst: Mut<Self::Target>);
}
```

A value wire in full:

```rust
#[derive(Component)]
#[relationship(relationship_target = DrivesTranslation)]
pub struct TranslationFrom(pub Entity);

#[derive(Component)]
#[relationship_target(relationship = TranslationFrom)]
pub struct DrivesTranslation(Vec<Entity>);

impl Wire for TranslationFrom {
    type Source = Vec3Out;
    type Target = Transform;
    const NAME: &'static str = "translation";

    fn propagate(src: &Vec3Out, dst: Mut<Transform>) {
        dst.map_unchanged(|t| &mut t.translation).set_if_neq(src.0);
    }
}
```

**Outlets are components.** An entity has a `Vec3` outlet because it has
`Vec3Out`. Two outlets of one type means two newtypes. One outlet feeding
differently-shaped inlets means several wire types, each taking what it needs
in its own `propagate`.

**Inlets are wire types.** An entity has a `translation` inlet because
`TranslationFrom::Target = Transform` and the entity has a `Transform`.

### 2.2 What the ECS enforces for free

| Invariant | Enforced by |
|---|---|
| An inlet has at most one source | One component per type per entity |
| Value types match | `Wire::Source` / `Wire::Target` — compile time |
| Direction | Which side holds the relationship component |
| Fan-out from one outlet to many inlets | The `RelationshipTarget` collection |
| Rewiring replaces the old source | `Relationship::on_insert` evicts the previous one-to-one relationship before adding the new one |
| A despawned consumer takes its wires with it | The wire component lives on the consumer |

A despawned *producer* is the one case the ECS does not clean up: the
consumer's wire component is left naming a dead entity. §3.1 skips such a wire
on the spot, which is the whole of the handling.

There is no runtime `TypeId` comparison, no direction check, no
inlet-already-connected check, and no fan-out rule anywhere in the engine.
Bevy 0.19's `Relationship` is `Component + Sized` with `fn get(&self) -> Entity`,
which is precisely a one-source-per-inlet, many-inlets-per-outlet edge.

### 2.3 Hierarchy

```rust
impl Wire for ChildOf {
    type Source = Transform;      // a spatial may parent a spatial
    type Target = Transform;
    const NAME: &'static str = "parent";
    fn propagate(_: &Transform, _: Mut<Transform>) {}   // the wire IS the state
}
```

`Wire` is local and `ChildOf` is foreign, so the orphan rule permits the impl.
Nothing in `sway-graph` inserts `ChildOf`; authoring does, and Bevy's own hooks
maintain `Children`. `propagate` is empty because a structural connection
carries no per-tick value — its existence is the state. The `Source`/`Target`
associated types are not vestigial here: they are the legality rule the editor
uses to decide whether a parenting wire may be drawn at all.

This is the whole of the hierarchy mechanism. There is no `Spatial` marker, no
`Entity`-typed port policy, no separate parenting-acyclicity pass, and no
compile step that emits `ChildOf`.

### 2.4 Behaviours

Most computation is not a connection, and most of it does not belong to the
graph at all:

| What the output depends on | Where it runs |
|---|---|
| Only external state — `Time`, MIDI, input | An ordinary Bevy system, before `graph_tick` |
| Nothing; it only consumes — mesh upload, material rebuild | An ordinary Bevy system on `Changed<T>` |
| **A wired inlet, in the same tick** | A **behaviour**, in the order |

Only the third case needs the graph, and it needs it for a reason an ordinary
system cannot satisfy: it must run *after* its inlets are propagated and
*before* its output is read downstream — a position determined by data flow,
not by a fixed slot in the schedule.

An LFO is this third case, not the first. `LfoInlets` is
`{ hz, shape, phase, amplitude }`, so an LFO whose amplitude is driven by
another LFO — the canonical modulate-the-modulator patch — must compute
between the two propagations. An LFO with nothing wired could indeed be a
plain system; registration is per component type, so a type that *can* be
driven is a behaviour even for instances that happen not to be.

```rust
pub type BehaviourFn = fn(&mut World, Entity, &TickCtx);
pub fn register_behaviour<C: Component>(app: &mut App, run: BehaviourFn);
```

`Wire` and behaviour are the only two things `sway-graph` registers.

### 2.5 Vertices are entities

A relationship component is per-entity, so the graph is a graph over entities.
Two unrelated components on one entity that flow in opposite directions read as
a cycle even though nothing circles. This is accepted: it is the same
simplification as §3.3's single sort, the case is rare, and splitting such an
entity in two resolves it.

## 3. Ordering and the tick

### 3.1 The one derived artifact

```rust
#[derive(Resource, Default)]
pub struct GraphOrder { steps: Vec<Step> }

enum Step {
    Propagate { wire: WireId, src: Entity, dst: Entity },
    Run       { behaviour: BehaviourId, entity: Entity },
}
```

The tick is a flat walk with no component-type lookups:

```rust
for step in &order.steps {
    match *step {
        Step::Propagate { wire, src, dst } => (wires[wire].propagate)(world, src, dst),
        Step::Run { behaviour, entity }    => (behaviours[behaviour].run)(world, entity, &ctx),
    }
}
```

One erased propagate per wire type, monomorphised at registration:

```rust
fn propagate_of<W: Wire>(world: &mut World, src: Entity, dst: Entity) {
    let Ok([src_ref, mut dst_mut]) = world.get_entity_mut([src, dst]) else {
        return;                     // producer despawned
    };
    let (Some(source), Some(target)) =
        (src_ref.get::<W::Source>(), dst_mut.get_mut::<W::Target>()) else {
        return;                     // legal transient state during spawn
    };
    W::propagate(source, target);
}
```

`World::get_entity_mut([a, b])` yields `[EntityMut; 2]` and errors on aliasing,
which is exactly the disjoint two-entity access propagation needs. The `else`
arms are the entirety of the dangling-wire story; there is no prune pass.

### 3.2 When the order is rebuilt

**The graph's shape changes only while authoring. During a show it is
constant.** Ordering therefore costs nothing at tick time.

`GraphOrder` is rebuilt when a `TopologyDirty` flag is set, and starts dirty.
`register_wire::<W>` also registers a watch system into an **editor-only
plugin**:

```rust
fn watch<W: Wire>(
    added: Query<(), Added<W>>,
    mut removed: RemovedComponents<W>,
    mut dirty: ResMut<TopologyDirty>,
) {
    if !added.is_empty() || !removed.is_empty() { dirty.0 = true; }
}
```

Monomorphised per wire type, archetype-cheap, and **absent from a show build**.
Behaviour component types get the same treatment. Authoring is therefore plain
ECS insertion — there is no `connect` API to route changes through and no
discipline for the show path to honour.

In a show build, whatever loads the project sets the flag once when the load
completes. If that is forgotten the step list stays empty and the graph is
visibly inert, which is a loud failure rather than a subtle one.

### 3.3 The sort, and diagnostics

One Kahn sort over every wire type, structural included. Rebuild-time checks
populate a `GraphDiagnostics` resource for the editor to render:

| Diagnostic | Meaning |
|---|---|
| `WouldCycle { via }` | The wires participating in a cycle |
| `MissingSource` | The producer lacks `W::Source` |
| `MissingTarget` | The consumer lacks `W::Target` |

A cycle never stops the render: the sort emits the acyclic part in topological
order and appends cycle members in entity order, so members read the previous
tick's value. All of this is computed at rebuild, so a live show pays for none
of it.

### 3.4 Change detection replaces the cook gate

`get_mut` marks `Changed<Target>` unconditionally, so a wire that writes every
tick would defeat change detection entirely. **`propagate` is responsible for
not writing an equal value.** Bevy's own API is enough for this — `Mut::map_unchanged`
narrows to a field without marking anything, and `DetectChangesMut::set_if_neq`
then marks only on a real change — so `sway-graph` adds no helper of its own.
This is the one discipline a wire author can get wrong, and §6 pins it with a
test.

Given that discipline, `Changed<T>` on a real component is the whole dirty
story. Downstream reaction — uploading a mesh, rebuilding a material — becomes
an ordinary Bevy system filtered on `Changed<T>`, outside the graph. That
deletes `NodeRuntime`, `cook_dirty`, `last_product_ticks`,
`produced_change_tick`, `COOKS`, and the cook gate, and it is why commitment 4
holds: nothing in the graph touches an asset or the GPU.

### 3.5 Unconnected values

A wire writes into the real component, which is also where the authored value
lives. A connected wire overwrites it each tick; on disconnect the field keeps
whatever arrived last and the author edits it directly from there. There is no
shadow copy, no prefill pass, and no connected-slot mask — the graph stores no
per-port state at all.

The cost is explicit: authoring a value, connecting, then disconnecting does
not restore what was typed. Restoring it is undo-shaped editor policy, not
engine state.

## 4. What `sway-graph` becomes

| Concern | Today | Here |
|---|---|---|
| Node types | `NodeType`, `ORDINALS`, `NodeTypeRegistry` (581 lines) | Gone |
| Schema | `derive_fields`, `FieldSpec`, `FieldKind` (442) | Gone — associated types |
| Compile | `compile.rs` (1132) | A Kahn sort |
| Slot storage | `PortArena`, `PortView` (201 + 212) | Gone — real components |
| Tick | `tick.rs` (614) | A flat step walk |
| Edges | `Edge`, `Endpoint`, `EdgeFrom`/`EdgeTo` | Relationship components |
| Cook | `cook`, `COOKS`, `NodeRuntime`, the gate | `Changed<T>` |
| Registration | `register_node_type` | `register_wire`, `register_behaviour` |

Kept unchanged: `transport.rs`, `Time<Transport>`, `MusicalTime`, `TickCtx`,
`EditorPos`, `GraphTickCount`, and `graph_tick` as an exclusive system in
`FixedUpdate`.

## 5. Scope

### 5.1 In this spec

The core model above, plus a vertical slice chosen to exercise every mechanism
exactly once:

- an `Lfo` **behaviour**, whose amplitude is itself driven by a second LFO —
  so the slice contains a genuine transformer and the order is load-bearing
- an `AmplitudeFrom` and a `TranslationFrom` **value wire**
- **fan-out** from one LFO to two consumers
- the `ChildOf` **structural wire** parenting meshes to a group
- a mesh built once at spawn by a plain Bevy system

The modulated LFO is the point of the slice. A graph of pure sources wired to
pure sinks is depth-one: every test would pass under any evaluation order, and
the design's central claim — that a chain resolves within a single tick —
would go unproven. The chain `Lfo A → Lfo B.amplitude → Transform.translation`
fails visibly if the order is wrong.

The demo graph is rebuilt on this slice and remains beat-locked through the
untouched transport layer.

Editor work is scoped to **read-only display**: a box per entity, inlets from
registered wire types whose `Target` the entity has, outlets from `Source`
components it has, and a line per wire instance. Authoring in this spec is
programmatic in `demo_graph.rs`; the watch systems are tested directly by
inserting wire components.

### 5.2 Deleted here

`compile.rs`, `PortArena`, `PortView`, `Product`, `Spatial`, `FieldKind`,
`ProductAccess`, `ReflectProduct`, `register_product`, `NodeType`,
`NodeTypeRegistry`, `NodeTypeId`, `ORDINALS`, `GraphNode`, `NodeId`,
`NodeRuntime`, `Edge`, `Endpoint`, `EdgeFrom`, `EdgeTo`, `InEdges`, `OutEdges`,
`prefill`, `seed_outlets`, `cook`, `produced_change_tick`, and the editor's
`EdgeKind`.

### 5.3 Kept from `sway-nodes`

Node types outside the slice lose their `NodeType` impls here and return as
wires plus behaviours in the follow-up. **Their pure logic and its tests stay.**
`BeatTrigger`'s boundary math, the envelope curves, the LFO phase advance and
the MIDI parsing are tested behaviour with golden traces behind them; only the
node wiring dies. `sway-nodes/tests/traces.rs` keeps passing. The previous form
of every node type remains reachable at commit `80dfeb8`.

### 5.4 Follow-up, not here

- The remaining node types as wires and behaviours
- Drag-to-connect and the full editor snapshot/canvas rework
- Geometry and asset flow — how a `Handle<Mesh>` reaches `Mesh3d`
- `Events<T>` and its per-tick clearing policy
- Variadic inlets: a `Vec` of sources has no single-relationship
  representation, and the slice does not need one
- M4's project format, which now serializes wire components rather than an
  edge table

## 6. Testing

Unit tests beside the code, integration through a headless `App`, matching the
project's existing shape.

1. **`LINKED_SPAWN` does not cascade-despawn consumers.** `RelationshipTarget::on_despawn`
   despawns every source entity, and `LINKED_SPAWN` defaults to false when
   derived — but if that gating is not what it appears, despawning an LFO would
   despawn everything it drives. **This test is written first**, before any
   wire type exists.
2. **A wire carrying an unchanged value leaves `Changed<Target>` false.** The
   single discipline §3.4 rests on.
3. Rewiring an inlet evicts the previous source, pinning `Relationship::on_insert`.
4. The Kahn sort's order, and cycle members appended deterministically.
5. A despawned producer leaves the consumer untouched rather than panicking.
6. Watch systems mark the order dirty on insert and on remove.
7. End-to-end: build the slice, tick **once**, and assert the child's world
   transform already reflects the full `Lfo A → Lfo B.amplitude →
   Transform.translation` chain. A one-tick assertion is what distinguishes a
   correct order from a schedule that merely converges over several frames.

## 7. Risks

**The `LINKED_SPAWN` reading is unverified.** Test 1 exists because the derive
gating was read from `RelationshipTarget`'s documented default and from today's
`edges.rs` opting in explicitly, not from the expanded macro. If the reading is
wrong, wire targets need `RelationshipHookMode` handling or a hand-written
`RelationshipTarget` impl.

**Entity-granular cycles** (§2.5) may prove more annoying in practice than
expected. The escape hatch is splitting an entity, which is cheap; if it
recurs, the next revision moves vertices to (entity, component) and pays for a
richer edge representation.

**One outlet component per outlet** is honest but verbose for a producer with
several outputs of the same type. The slice will show whether that is a real
cost or a theoretical one.

## 8. Success criteria

- No `NodeType`, `Edge`, `PortArena`, `Product`, `Spatial`, or `FieldKind`
  anywhere in the workspace
- A connection is a relationship component, and hierarchy is one `impl Wire for ChildOf`
- `sway-graph`'s runtime is a step list, a Kahn sort, and two registries
- No reflection on the tick path
- The demo runs, beat-locked, on the slice
- `sway-nodes/tests/traces.rs` still passes
