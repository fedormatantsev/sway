# M2b — Structure edges, geometry, and the cook gate — Design

**Date:** 2026-08-01
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §2.1, §2.4, §2.5, §2.9, §2.10, §2.11, §3, §4, §5 (M2), §7
**Predecessor:** `2026-07-31-m2a-graph-engine-design.md`, and the findings at
`docs/superpowers/reports/2026-07-31-m2a-graph-engine-findings.md`

## 1. What this milestone builds

M2a split M2 and deferred five things: `ChildOf` and `Feeds` edges with the
structure validation pass, the `Geometry` component, cook gating on change
ticks, the reflect-driven debug inspector, and driving M1's visuals.

**M2b takes the first three plus the smallest scene node set that proves
them.** The inspector and M1's provisional point cloud and sprite layers are
not here — see §11 for the risk that carries.

In scope:

- `ParentEdge` and `FeedsEdge`, and the structure validation pass (§3, §4)
- The `Geometry` component and its `Arc`-backed attribute tables (§5)
- Cook gating (§6) and the tick's second pass (§7)
- Six node types: `Grid`, `Displace`, `Mesh`, `Group`, `StandardMaterial`,
  `Rgb` (§8)
- Deleting `crates/sway-app/src/bridge.rs` and the M0 cube (§9)

Out of scope, and where each goes: GPU-resident cooks and the extract-and-
dispatch path (M5); the rest of §2.10's scene node set — `Asset`, `Camera`,
one node per light type, `Scatter`, `CopyToPoints` (M5); the RON project
format (M4); the inspector (M7).

**Exit:** a Rust-built graph cooks geometry through a two-operator `Feeds`
chain into a `Mesh` parented under a `Group`, with live MIDI driving
displacement, colour and rotation; every structure-pass failure produces a
node-attributed message in the vocabulary of the edge kind that failed; and an
unrelated param change provably cooks nothing.

## 2. Crate layout

One new crate, per the parent spec's §3:

```
sway-geo     the Geometry component and the operators over it — Grid, Displace
```

**`sway-geo` is headless at M2b.** The parent spec's §3 puts it on the render
side depending on `bevy_render`, and §2.10 explains why: GPU-resident cooks.
M2b's cooks are all CPU (§6), so the dependency has no consumer yet and joins
at M5. This is M2a's own reasoning about `bevy_transform` and `bevy_asset`
applied again — the manifest is the only place the layering constraint is
actually enforced, so it should not list a dependency no code uses.

`sway-geo` depends on `sway-graph`, `bevy_ecs`, `bevy_math` and `bevy_reflect`.

`sway-nodes` gains the scene node types and therefore `bevy_pbr` and
`bevy_render`. §2.9's rule is a constraint on `sway-graph`, not on the node
crates — §2.10 says so explicitly — and `sway-graph` still depends on no
renderer. What `sway-nodes` loses is compile time, not testability: its
golden-trace tests build a `MinimalPlugins` app and adding the dependency does
not add the plugin.

`sway-graph` adds `bevy_transform`, which §3 always listed and which §4's
`ParentEdge` application now needs.

## 3. All three edge kinds are authored as edge entities

§2.10 says `ChildOf` and `Feeds` "carry no value at all and are relationships
between node entities directly rather than edge entities of their own."

**That cannot be implemented as written, and the obstacle is §2.5's own error
table.**

§2.5 requires the structure pass to diagnose "a `ChildOf` fan-out (illegal, an
entity has one parent)" and "a `Feeds` slot filled twice", each with an error
message in its own vocabulary. A Bevy relationship component cannot represent
either state: an entity holds exactly one `ChildOf`, and inserting a second
replaces the first silently. The illegal state the compiler is asked to
diagnose is unrepresentable, so the diagnostic is unwritable — the author
draws a second edge into a parent socket and the first one simply vanishes.

`Feeds` has a second, independent obstacle. A node needs several slots at once
— `Mesh` has `geo` and `material`, and M5's `CopyToPoints` has `points` and
`proto` — and one relationship component type per entity cannot carry two
targets. Working around it means one relationship type per slot name, which
would put the engine in the business of knowing every slot name any node will
ever declare, against §2.4's whole direction.

So **every edge is an entity carrying `EdgeFrom`/`EdgeTo`**, exactly as
`ParamEdge` already is:

```rust
/// Source is the child, target is the parent — §2.10's direction note.
#[derive(Component)]
pub struct ParentEdge;

#[derive(Component)]
pub struct FeedsEdge {
    /// Ordinal within the target node's Slots schema.
    pub slot: u16,
}
```

Three edge components, one authoring model, three compile treatments:

| Component | Compiles to | Enters |
|---|---|---|
| `ParamEdge` | an arena copy in the edge plan | `tick_order` |
| `FeedsEdge` | a per-node slot table on `NodeRuntime` | `cook_order` |
| `ParentEdge` | Bevy `ChildOf`, written by the compiler | neither |

This contradicts §2.10's prose and matches its table, whose "Compiles to"
column already says `ChildOf` compiles *to* the Bevy hierarchy rather than
being it. It also keeps the property that made param edges worth having:
`linked_spawn` on `EdgeFrom`/`EdgeTo` means despawning a node despawns all
three kinds of its edges, so M7's delete cannot leave a dangling reference.

Two further consequences, both wanted:

- **Validation now gates application.** `ChildOf` is engine-owned and written
  only after the structure pass passes, so a rejected hierarchy never reaches
  transform propagation. M4's reload needs exactly this: a bad edit must leave
  the previously compiled graph in force rather than half-applying itself.
- **Reparenting is a compile-time reconcile.** For each node the desired parent
  is its single `ParentEdge` target; the compiler inserts, updates or removes
  `ChildOf` to match. `ChildOf` joins `NodeRuntime` as engine-owned derived
  state, never authored directly.

The despawn hazard §5 of the parent spec names for M7 — deleting a scene node
cascades through `ChildOf` to its children — is unchanged by this, because
`ChildOf` still ends up in the world either way. M2b does not delete nodes.

## 4. The structure pass

§2.5's two-pass split holds: structure edges are validated separately from
dataflow, with their own failure vocabulary.

```
project → spawn node entities
        → structure pass: ParentEdge, FeedsEdge → apply ChildOf, emit cook_order
        → dataflow pass:   ParamEdge → validate types → tick_order
        → flat orders + port arena layout + per-node slot tables
```

### Slots are typed from types, not from a table

`NodeType` gains two associated types:

```rust
trait NodeType: 'static {
    type Params:  Reflect + Component;
    type Outputs: Reflect;
    type Slots:   Reflect;    // named Feeds inputs; () when the node has none
    type Produces: 'static;   // what a Feeds edge from this node carries; ()
    type State:   Component + Default;

    /// Does this node carry a Transform, i.e. can it appear in the scene tree?
    const SPATIAL: bool = false;
    /// Whether `cook` is meaningful. `register` stores `Some(cook)` only if set.
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);
    fn cook(_world: &mut World, _node: Entity, _slots: &SlotView) {}
}
```

`Slots` is a reflect struct whose fields are `Slot<T>` markers —
`Slot<Geometry>`, `Slot<MaterialOf<StandardMaterial>>`. Slot ordinals are
positional in field order and verified at registration by the same
`(name, ordinal)` check M2a already applies to ports, for the same reason:
reordering two fields would otherwise silently swap two slots. M2a's findings
record that name-only matching was insufficient; that correction carries over
unchanged.

Extracting `T` from a field typed `Slot<T>` uses the mechanism M2a already
built for `Event<T>`: a `ReflectSlot` `TypeData` carrying the capability's
`TypeId` and name, registered per instantiation by `register_slot::<T>()`,
exactly as `register_event_port` and `ReflectEventPort` work today. `Slot<T>`
derives `Reflect` with its `PhantomData<fn() -> T>` ignored — M2a's findings
record that this works in `bevy_reflect` 0.19 and needed no non-generic
fallback.

`Produces` names the capability a `Feeds` edge from this node carries.
`Produces = ()` means the node cannot be a `Feeds` source at all. **It is
bounded by `'static`, not `Reflect`**: the structure pass needs only type
identity, and requiring `Reflect` would force it onto `Geometry`, whose
`Arc<Vec<T>>` attribute storage has no reason to be reflectable and may not be
without work. Error messages use `type_name`, as `NodeTypeRegistry` already
does for node types.

`COOKS` is explicit because Rust cannot tell a defaulted trait method from an
overridden one, and §7's gate needs to know whether a node has a cook at all
rather than calling an empty one on every dirty node.

The structure pass compares the source's `Produces` `TypeId` against the
slot's declared `TypeId`. This is a **static** check against the registry: it
reads no components and does not depend on whether anything has cooked yet,
which matters because a node's `Geometry` does not exist before its first
cook. Adding a capability at M5 — a light rig, a curve, a volume — touches no
engine code.

`SPATIAL` exists because parenting is otherwise unvalidatable. Without it,
parenting an `LFO` to a `Group` writes a `ChildOf` onto an entity with no
`Transform`, which propagation silently ignores; §2.5 asks for a clear error
instead.

### Failure modes

Every message names the offending node and speaks the failing edge kind's
vocabulary, per the parent spec's §4:

| Failure | Message names |
|---|---|
| Parenting cycle | every node in the cycle |
| Two `ParentEdge`s from one child | the child and both proposed parents |
| `ParentEdge` touching a non-`SPATIAL` node | the node and its type |
| `Feeds` slot filled twice | the target node, the slot name, and both sources |
| `Feeds` slot type mismatch | both nodes, the slot name, the expected capability, and the source's `Produces` |
| `Feeds` slot ordinal out of range | the node, the ordinal, and the `Slots` arity |
| `Feeds` cycle | the blocked set, with M2a's caveat that Kahn's remainder is not exactly the cycle |
| `Feeds` source whose `Produces` is `()` | the source node and its type |
| Either edge kind referencing an absent node | the edge and the missing endpoint |

M2a's rule is unchanged: **all failure happens at compile, and the tick is
infallible.** After validation, a slot lookup is an index into a table the
compiler filled.

## 5. `Geometry`

```rust
#[derive(Component, Clone, Default)]
pub struct Geometry {
    attrs: BTreeMap<AttrName, Attribute>,
    point_count: usize,
    indices: Option<Arc<Vec<u32>>>,
}

enum Attribute {
    F32(Arc<Vec<f32>>),
    Vec2(Arc<Vec<Vec2>>),
    Vec3(Arc<Vec<Vec3>>),
    Vec4(Arc<Vec<Vec4>>),
    U32(Arc<Vec<u32>>),
}
```

Planar rather than interleaved, per §2.10 — Houdini's and USD's layout, and the
one the GPU wants when M5 moves this to buffers. Conventional names are `P`,
`N`, `uv`, `Cd`, `pscale`; arbitrary `@custom` attributes are legal, which is
§2.10's stated reason for one component holding a map rather than one component
per attribute: an author creates an attribute at runtime and component types
cannot be registered then.

`Arc` per attribute is what makes §2.11's claim real — "passing an unchanged
attribute through an operator is a refcount bump rather than a copy." An
operator that rewrites `P` and passes `N` through copies neither; §10 asserts
this with `Arc::ptr_eq` rather than leaving it as a comment.

`BTreeMap` rather than `HashMap` so attribute iteration order is deterministic.
Cook output is asserted directly (§10) and mesh upload walks the map, so
iteration order is observable.

## 6. The cook gate

§2.11 states the rule — a node cooks when "an input's geometry changed, its
params changed, or it is time-dependent" — and prescribes stored change ticks
for it. Stored ticks are right for `Geometry` and **wrong for params.**

The reason is a rule §2.11 states elsewhere and does not connect to this one: a
connected port *shadows* the authored value rather than overwriting it, so
`Params` is never written by the graph. An `LFO` driving `Displace.amount`
changes the effective parameter every tick while `Params`' change tick never
moves. A gate reading `get_change_ticks::<Params>()` would cook once and then
freeze — with correct-looking geometry, no error, and nothing in the graph to
suggest which node stopped responding.

So the gate is a **sticky dirty flag on `NodeRuntime`**, set from three places
and cleared only by a cook that actually ran:

| Set by | Mechanism |
|---|---|
| a driven input changing | edge gather compares incoming against current, `reflect_partial_eq` |
| an authored param edit | the existing change-tick-gated prefill, when it fires |
| an upstream cook | stored `Tick` per `Feeds` slot vs `get_change_ticks::<Geometry>()` on the source |

Only the third keeps stored ticks, which is where §2.11's prescription was
aimed and where it is correct: `Geometry` is large and not usefully
value-compared.

**Sticky is what makes it survive cadence.** §2.11's warning about
`Changed<T>` is that it means "changed since this system last ran", the tick
system runs every tick, so the flag is true for exactly one tick and a node
that skips for any other reason misses the change permanently. A flag that
accumulates until consumed has no such window, and needs no stored baseline
values to compare against. Compilation sets it on every node, so each cooks
once after a load.

A node whose input is driven by an LFO therefore cooks every tick. That is
correct and intended: §7's recorded position is that cook cost belongs to the
graph author and the tool reports rather than polices it.

**No time-dependent flag at M2b.** Neither `Grid` nor `Displace` needs one —
both are pure functions of their inputs — and M2a's precedent applies: it
declined to add observer triggers before a consumer existed, because an
unexercised mechanism's constraints cannot be tested. Recorded in §11.

## 7. The tick has two orders

Cooking must follow `Feeds` order, which is not the param order. Two
resolutions were considered.

**Merging `Feeds` into the topological sort** — rejected on two grounds. It
contradicts §2.5, which says structure edges must not enter the sort. And it
invents cycles: `A --param--> B` together with `B --feeds--> A` is legal and
well defined — `B` reads `A`'s port, `A` cooks from `B`'s geometry — and a
merged sort rejects it as circular.

**Two orders, two passes** — taken. `compile` emits `tick_order` over the param
DAG, unchanged from M2a, and `cook_order` over the `Feeds` DAG. `ParentEdge`
enters neither, exactly as §2.5 says.

```
clear event slots
pass 1, tick_order:  gather  → copy each incoming param edge into its input slot
                     prefill → unconnected continuous inputs from Params, tick-gated
                     tick    → registry fn; reads ports, writes its own components
                               (gather and prefill may set the node's dirty flag)
pass 2, cook_order:  if dirty and the type has a cook fn:
                       read Feeds sources' Geometry, compute, write own Geometry,
                       store the sources' change ticks, clear dirty
```

Ticks precede cooks globally, so a cook always sees its own node's effective
params already applied — §2.11's step B before its step C, as written. Nothing
crosses back: `Geometry` never enters the port arena, so there is no dependency
from pass 2 into pass 1.

**Cook is a separate registry fn, not a branch inside `tick`.** That keeps
"whether to cook" engine-owned rather than per-node, which is what lets §10's
negative test assert on a cook counter rather than on an output that merely
happens to be unchanged — the difference between testing the gate and testing
around it. `SlotView` is to slots what `PortView` is to ports: scoped to the
node being cooked, so a node cannot reach another node's slot table by
arithmetic accident.

Nothing else about the tick changes. It is still one exclusive system in
`FixedUpdate` holding `&mut World`, so writes are immediate and nothing routes
through `Commands` (§2.6, §2.11).

## 8. The node set

| Node | Params | Slots | Produces | Cooks | `SPATIAL` |
|---|---|---|---|---|---|
| `Grid` | `rows`, `cols`, `width`, `height` | — | `Geometry` | yes — `P`, `N`, `uv`, indices | no |
| `Displace` | `amount`, `frequency` | `geo` | `Geometry` | yes — `P += N * f(P)` | no |
| `Mesh` | `translation`, `rotation`, `scale` | `geo`, `material` | — | yes — `Geometry` → `Assets<Mesh>` | yes |
| `Group` | `translation`, `rotation`, `scale` | — | — | no | yes |
| `StandardMaterial` | `base_color`, `emissive`, `metallic`, `perceptual_roughness` | — | `MaterialOf<StandardMaterial>` | no | no |
| `Rgb` | `r`, `g`, `b` | — | — | no | no |

`Grid` and `Displace` live in `sway-geo`; the rest in `sway-nodes`.

`Mesh` is where a `Feeds` chain enters the `ChildOf` tree, which §2.10 calls
the boundary an author most needs to understand. Its cook is where the gate
earns its keep: an ungated version re-uploads a mesh asset every tick for a
scene that is not moving, which is precisely the failure §2.11 names.

`StandardMaterial` owns a `Handle<StandardMaterial>` and applies its effective
params through `Assets<StandardMaterial>` following §2.11's asset rule — `get`,
compare, and only then `get_mut`, because `get_mut` marks the asset changed by
the act of being called. `Mesh` reads the handle from its `material` slot's
source. There is no node that assigns a material to something else, per §2.10.

**`Rgb` is a sixth type beyond the five this milestone's scope implies, and is
flagged rather than smuggled.** §2.4 fixes a material node's ports as the
material's own fields, so `base_color` is a `Color` port, and nothing in M2a's
signal set produces a `Color`. Without `Rgb` the demo graph cannot drive
colour, which the cube it replaces already did — a regression in the live path
this milestone exists to keep. It also puts the first struct-typed value across
a *continuous* edge; M2a carried structs only as event payloads.

**No component hooks yet.** §2.2 gives `State`'s `on_add`/`on_remove` the job
of spawning and tearing down what a node owns, but at M2b a dropped `Handle`
tears down both the mesh and the material, so the mechanism has no consumer.
Same precedent as §6's deferred time-dependent flag.

### The demo graph

```
Grid ──feeds(geo)──→ Displace ──feeds(geo)──→ Mesh ←──feeds(material)── StandardMaterial ← Rgb
                                               └──parent──→ Group(root)

MidiCC 74 ────────param→ Displace.amount
MidiNote → Envelope ─param→ Rgb.r
LFO ──────────────param→ Group.rotation.y
```

Every edge kind, both sides of the gate, and a param path that reaches the GPU.
`Displace.amount` is deliberately the CC-driven one: it invalidates `Geometry`,
so the gate is visible on stage rather than only in tests.

## 9. The `sway-app` handover

`crates/sway-app/src/bridge.rs` is deleted, but not all of it was throwaway.

- `MidiRx`, `MidiTimeEpoch` and `feed_midi` are MIDI ingress, not the temporary
  graph, and move to `crates/sway-app/src/midi_feed.rs` with their tests
  intact. M2a's finding 9 — the epoch is sampled at first drain and long-session
  drift is uncorrected — moves with them and stays open for M3.
- `setup_cube_graph` becomes `demo_graph.rs`, building §8's graph. Still
  Rust-built; M4's project loader replaces it, calling the same `compile`.
- `scene.rs` keeps only the camera and light. `Cube`, `colour_for_level` and
  `apply_level` are deleted.

**`apply_level`'s three tests are ported onto the `StandardMaterial` node, not
dropped.** They cover §2.11's `get`/compare/`get_mut` rule, and that rule now
lives on the material node. Naming this here so a reviewer reads the deletion
as a move rather than as lost coverage — the same courtesy M2a's §10 extended
to M0's decay trace.

## 10. Testing

Extending the parent spec's §4.

- **Structure failure table.** One case per row of §4's table, each asserting
  the message names the offending node and uses the failing edge kind's
  vocabulary. "Cycle detected" is not an acceptable message for a doubly-filled
  slot.
- **Failed validation applies nothing.** A rejected hierarchy leaves no
  `ChildOf` in the world — the property §3 says M4's reload depends on.
- **Reparent reconcile.** Recompiling with a changed `ParentEdge` removes the
  old `ChildOf` as well as writing the new one, and `GlobalTransform`
  propagates through the result.
- **Cooks are pure functions of their inputs.** Assert `Geometry` attributes
  directly, per §4's cooking bullet. Assert `Arc::ptr_eq` on an attribute the
  operator passed through, which is the only thing that distinguishes §5's
  refcount claim from a copy.
- **The gate, four ways.** An unrelated param change cooks nothing, asserted on
  a counter. A driven param that changes cooks every tick, while a driven param
  held still cooks once — this is the case that fails if the gate reads
  `Params` change ticks, and it is the reason §6 exists. A node that skips a
  cook still picks up a missed upstream change, which is §2.11's named
  `Changed<T>` failure. Recompiling cooks every node exactly once.
- **Mesh upload gating.** Unchanged geometry does not mark the mesh asset
  modified.
- **Material asset writes.** §9's three ported tests.
- **Golden traces are unchanged.** The arena snapshot format covers signals;
  geometry is not in the arena and is asserted directly. M2a's determinism and
  sub-tick discrimination cases continue to run.

## 11. What this milestone leaves open

- **The inspector, and with it §7's reflect-ergonomics question.** The parent
  spec wanted M2 to walk `TypeRegistry` for one node type's params so that
  missing `TypeData` surfaced before an XL milestone began. Deferring it
  wholesale means that question is first answered at the start of M7. This is
  an accepted risk, decided deliberately, not an oversight. M2b's six node
  types do add `Color`, `Vec3` and slot marker types to the registry, so the
  sample M7 inherits is larger than M2a's.
- **The fixed tick rate.** Still open. M2b is the first milestone with cooks in
  the tick, so its measurement is worth more than M2a's floor-scale figure, but
  six node types over one grid is not the graph the number should be chosen
  against. Record it as a data point; do not close §7.
- **A time-dependent cook flag** (§6) and **`State` component hooks** (§8),
  both deferred for want of a consumer.
- **GPU residency and mixed-residency ping-pong.** M5, informed by M1's
  finding that compute output reached a readback but never the draw.
- **MIDI epoch drift** (§9). M3.
- **Event fan-in stability across recompiles.** Unchanged from M2a; M4's reload
  semantics decide it.

## 12. What this milestone must produce besides code

A findings report at
`docs/superpowers/reports/2026-08-01-m2b-scene-composition-findings.md`
answering:

1. Did the sticky dirty flag (§6) hold, or did a case appear that it gets
   wrong — in particular any cook whose correctness depends on a value the
   gate does not observe?
2. Did two orders (§7) hold, or did a real graph want an ordering constraint
   that spans both DAGs?
3. What did `Geometry`'s `Arc` sharing actually save, measured rather than
   assumed?
4. How did slot typing (§4) read at the call site — is `Slots` plus `Produces`
   the right split, or does one of them want to be the other?
5. What the tick costs with cooks in it, recorded as a data point for §11's
   tick-rate question and explicitly not as its answer.

If any decision here turns out wrong in implementation, this document gets a
**Revision** line at the top, in the style the parent spec and the M2a design
use. A design document that records what was believed beforehand and is never
corrected afterwards is worse than none.
