## Context

See `proposal.md` — Why. The constraints that shape the approach:

- `sway-graph` is depended on by every other crate except `sway-gpu` and
  `sway-midi-core`, so any change to its surface fans out. The compensating
  factor is that the callers are few and all in this workspace.
- The audit that produced this change measured external use of every
  `sway-graph` public item. Nine are used nowhere outside the crate
  (`EvalOrder`, `GraphStep`, `PropagateStep`, `Link`, `Sorted`,
  `topological_order`, `absolute_path`, `compatibility_of_values`, `PartType`);
  `tick_graph` is reached only through `GraphPlugin`. That measurement is what
  makes the surface reduction cheap.
- `sway-editor` deliberately links neither the `bevy` facade nor `bevy_render`;
  `sway-runtime` links both. They cannot depend on each other, which is why
  `viewport_input` ended up parked in `sway-graph` in the first place.
- The demo project (`crates/sway-app/assets/demo.sway.ron`) is the only real
  graph in the tree and the only version-3 document anywhere in it. It uses
  `MidiTime`, `Oscillator`, `Remap`, `Vec3`, `MeshAsset`, `PlaneMesh`,
  `FrameSequence`, `PbrMaterial`, `SpriteMaterial`, `MeshNode`, `Group`,
  `Camera` and `DirectionalLight` — 25 nodes, each with a `pos`. It is both the
  end-to-end check for this refactor and a file the refactor has to rewrite.

## Goals / Non-Goals

**Goals:**

- A written rule in `docs/architecture.md` that answers "which crate does this
  go in?" without reading the code.
- A `sway-graph` whose public surface contains only graph mechanics, with no
  concrete value type, UI type or channel type named anywhere in it.
- Node domains that a host wires with one `add_plugins` call each.
- A crate graph on paper that matches the crate graph in fact.

**Non-Goals:**

- No behavioural change to evaluation, ordering, propagation, legality or
  projection. Every golden trace must produce identical numbers afterwards.
- No attempt to keep version-3 documents loadable. The format breaks cleanly at
  version 4 rather than growing a compatibility path for a field that should
  never have been in the format.
- No change to any node kind's semantics beyond `Envelope`'s time source. The
  `Vec3` node kind is renamed, not removed or reshaped.
- Not splitting `sway-runtime`'s node kinds from its projectors. Those two are
  halves of one domain and stay together (see Decision 8).
- `sway-events` remains unimplemented; the rules describe it as a future
  domain crate, nothing more.

## Decisions

### 1. Node annotations are `HashMap<String, Box<dyn PartialReflect>>`

`Node` gains `metadata: HashMap<String, Box<dyn PartialReflect>>` with
`metadata()` / `metadata_mut()` accessors, replacing `pos` / `set_pos`. The
editor stores its canvas position under the key `pos` as a real `Vec2`.

*Why reflected values rather than strings.* The engine already carries a node's
whole value as `Box<dyn Reflect>`; an annotation is the same kind of thing, so
this introduces no new concept. It keeps the one live annotation typed — `pos`
is a `Vec2`, not a string parsed by a helper — and it means a surface can
annotate with any registered type without a format change or a serialization
convention of its own.

*Why not `DynamicStruct`.* `DynamicStruct` models a struct whose fields were
invented at runtime; annotations are an open-ended map, so the map type is the
honest one. Nothing about the storage choice changes the (de)serialization
story below.

*Why not keep `pos` and add metadata alongside.* Two homes for placement is how
the current drift started. The point of the annotation map is that placement is
not special.

*Alternative rejected:* an editor-side `HashMap<NodeId, Vec2>`. It would keep
the graph cleaner still, but `NodeId` is runtime-only and the document keys by
stable id, so the editor would need its own id table and its own file. The
annotation map is the smaller total system.

### 1a. Annotations serialize untyped, and that costs registry coupling

Everything `sway-document` does today uses `TypedReflectDeserializer` /
`TypedReflectSerializer`, which work because the expected type is known: a
node's `inlets` deserialize against the node kind's registered `inlets` type.
An annotation has no such schema, so it needs the *untyped*
`ReflectSerializer` / `ReflectDeserializer`, which tag each value with its type
path on disk and recover the concrete type from the registry:

```ron
"lfoA": Node(type: "Oscillator", metadata: {"pos": {"glam::Vec2": (x: -460.0, y: 40.0)}}, inlets: (period: 8.0))
```

Two consequences, both accepted:

- **Every annotation value type must be registered.** This is the same coupling
  the document already has to node-kind types, so it adds a dependency of the
  same kind rather than a new one. An annotation whose type is not registered is
  *reported and skipped*, exactly as an unresolved node kind or path already is
  — never fatal, and never silently dropped.
- **`Vec2` must be in the registry** for the editor's `pos` to survive a round
  trip. `DefaultPlugins` registers the glam types, so the running app is covered;
  a bare-`TypeRegistry` test that exercises annotations has to register it
  explicitly.

*Save output ordering.* A `HashMap` iterates nondeterministically, so save sorts
keys before writing. Storage does not need order, but a committed asset whose
diff reshuffles every save is noise nobody wants; sorting at the boundary costs
one line and keeps the file stable.

*What this replaces.* An earlier draft used `BTreeMap<String, String>` with the
editor parsing `"<x>,<y>"`. That avoided the registry entirely and kept the file
readable by hand, but the file is written by the editor now, and a stringly-typed
value with a bespoke parse helper is a convention the engine cannot check.

### 1b. The format breaks at version 4

There is no compatibility read for the old `pos` field. Keeping one would put
the editor's vocabulary back into the document in the same breath as taking it
out.

`FORMAT_VERSION` becomes 4 and the existing whole-file version check refuses a
version-3 document by version, naming both versions — which is a better failure
than a serde error about a missing `metadata` field at some line. The
`sway-document::v3` module is renamed to `v4` to match, since it is named after
the format version it implements; `sway-app`'s `sway_document::v3::` imports
move with it.

`NodeDoc` loses `pos: (f32, f32)` and gains the annotation map. That is what
keeps `sway-document` from knowing that an editor exists — the current `pos`
field, whose doc comment reads "where the editor draws this node", is the leak
this closes. Afterwards the document has no notion of a canvas position, only of
annotations it carries and does not read.

`demo.sway.ron` is the only version-3 document in the tree and is rewritten in
this change (Decision 11 renames its `Vec3` entries at the same time).

### 1b. The format breaks at version 4

There is no compatibility read for the old `pos` field. Keeping one would put
the editor's vocabulary back into the document in the same breath as taking it
out.

`FORMAT_VERSION` becomes 4 and the existing whole-file version check refuses a
version-3 document by version, naming both versions — which is a better failure
than a serde error about a missing `metadata` field at some line. The
`sway-document::v3` module is renamed to `v4` to match, since it is named after
the format version it implements; `sway-app`'s `sway_document::v3::` imports
move with it.

`demo.sway.ron` is the only version-3 document in the tree and is rewritten in
this change (Decision 11 renames its `Vec3` entries at the same time).

### 2. `GraphCommand` is deleted; the graph is mutated through its own methods

`GraphCommand`, `CommandOutcome`, `apply_graph_command` and
`apply_graph_commands` all go. Every variant was already a thin wrapper over a
`Graph` method that exists — `Delete` over `remove`, `Connect` over `connect`,
`Disconnect` over `disconnect`, `SetSlot` over `set_slot` — and after the rest of
this change only six variants would remain anyway (`Select` and `Move` are gone
with selection and placement).

The engine ends up with one mutation surface. Two methods are new, because two
commands did work no existing method did:

| Method | Returns | Note |
|---|---|---|
| `insert(Node)` | `NodeId` | exists |
| `create(&TypeRegistry, type_path)` | `Option<NodeId>` | new — needs the registry for `ReflectDefault` and the node-kind check |
| `remove(NodeId)` | `Option<Node>` | exists |
| `connect(Port, Port, i32)` | `Result<EdgeId, ConnectError>` | exists |
| `disconnect(EdgeId)` | `bool` | exists |
| `set_slot(EdgeId, i32)` | `bool` | exists |
| `set_field(NodeId, &str, &dyn PartialReflect)` | `FieldWrite` | new |

`FieldWrite { Written, Unchanged, Rejected }` replaces `CommandOutcome`'s four
variants. Each method now reports precisely what it can fail at, instead of every
caller matching one enum that folds "no such node", "no such kind", "path did not
resolve" and "connection refused" together.

*The immediate payoff.* `sway-runtime`'s gizmo and picker hold `ResMut<Graph>`
and today build a `GraphCommand` only to `apply_graph_command` it inline against
the graph they already own. Five call sites of ceremony become five method calls.

*What the enum was really for.* Deferral, not dispatch. A masonry widget cannot
borrow the world during event dispatch, so an edit made in a widget has to be
recorded and applied after dispatch returns. That is an editor problem, and the
enum belongs to the editor — the same relocation this change makes for selection,
canvas placement and viewport input. See Decision 4.

*Why not keep it as the public authoring API.* It is a closed enum in the generic
engine: adding a mutation means editing it, which is the identical complaint that
retires `FieldValue`. Being consistent about that is the point of the change.

*What is given up.* A command enum is a natural undo journal, and methods are
not. There is no undo anywhere in the tree today — no code, no doc, no spec — so
nothing is lost now. If undo lands later it wants a journal designed for it
(inverse edits, coalescing, a cursor), which is a different artifact from a
dispatch enum and is better built then than approximated now.

### 2a. Field writes carry a reflected value

`set_field` takes `&dyn PartialReflect`: resolve the path, compare with the
existing `reflect_equal`, `try_apply`. The `FieldValue` enum, `boxed_for` and
`int_as` (~90 lines) move to `sway-editor`, where the inspector already knows a
control produced an `f64` and the field wants a `u8`. Saturating-narrowing
behaviour is preserved verbatim — it is specified in the `editor` delta.

`PartialReflect` requires `Any + Send + Sync`, so a boxed value crosses the
editor's channel unchanged.

*Alternative rejected:* keeping `FieldValue` and adding a `Reflected(Box<..>)`
escape-hatch variant. That leaves the closed enum in the engine and adds a second
way to say the same thing.

### 3. `viewport_input` becomes `sway-viewport-input`

A crate with no dependency but `bevy_math` (for `Vec2`) holding
`ViewportInput`, `ViewportButton`, `ViewportKey`, `ViewportModifiers` and
`normalize_viewport_pos`. `sway-editor` produces the events, the new
`sway-editor-viewport` consumes them, neither depends on the other, and the
engine is out of it.

`ViewportInputRx` (the crossbeam `Receiver` resource) goes to
`sway-editor-viewport`, which owns the systems that drain it.

### 4. The editor owns its own deferred-edit vocabulary

`GraphRx`, the `PreUpdate` drain, and the payload they carry all move to
`sway-editor` as `EditorEdit` plus a `GraphEditPlugin`. `sway-app` adds it in
editor builds only, matching how it already inserts `GraphRx` conditionally.
`GraphPlugin` then does one thing: insert the resource and schedule the tick.

`EditorEdit` is a small enum — create, delete, set field, connect, disconnect,
set slot — and its applier is a `match` mapping each variant onto the `Graph`
method from Decision 2. It exists because a masonry widget cannot borrow the
world mid-dispatch, not because the engine needs a command vocabulary.

Everything that *can* reach `&mut Graph` skips it: the gizmo and picker call the
methods directly, and so does document load. Only the 37 widget call sites in
`sway-editor` go through `EditorEdit`.

This drops `crossbeam-channel` from `sway-graph` entirely.

*Why a channel at all, given one thread.* The winit `window_event` handler drives
both masonry and `app.update()`, so this is not a thread boundary — a `Vec`
drained by the shell would do. The channel stays because it is already wired,
already `Send` (so a future worker thread is not a redesign), and replacing it is
not what this change is about.

*Alternative rejected:* Bevy `Messages<EditorEdit>`. Writing one still needs
`&mut World`, which is exactly what the widget does not have.

### 5. `Compat` and `Target` collapse; `NodeParts` is dropped

`Compat { Direct, Optional, Variadic }` (stored on the edge) and
`Target { Direct, Optional, Index(usize) }` (computed at rebuild) are the same
three-way distinction twice. Keep `Compat` on the edge as the connect-time
verdict, and let rebuild compute the destination index inline — the variadic
index is derived from the slot sort at rebuild anyway, so it never needed to be
carried in an enum variant. `Target` becomes crate-private, or disappears into
a `(Compat, usize)` pair passed to `write`.

`NodeParts` / `PartType` type data duplicates what `TypeInfo::Struct::field()`
already answers. Replace with one free function
`part_type(registry, type_id, part) -> Option<&'static TypeInfo>` reading the
registration's `TypeInfo` directly. This removes a `FromType` impl, a panic
path at registration time, and one item of registered type data per node kind.
The D3 shape check (`inlets`/`state`/`outlets` present) moves into
`register_node_kind` as an explicit assertion, which is where a caller expects
to be told they got it wrong.

### 6. Visibility narrowing

`EvalOrder`, `GraphStep`, `PropagateStep`, `Link`, `Sorted`,
`topological_order`, `absolute_path`, `compatibility_of_values`, `path::*` and
`tick::run` become `pub(crate)`, except where the `test-support` feature needs
them. `graph::order` and `graph::path` become private modules. `lib.rs`
re-exports stay the flat list they are; the `graph::` module path stops being a
second public spelling of the same items.

`sway-graph` also drops `bevy_transform` (unused) and moves `bevy_time` to
`dev-dependencies` (used only by the test fixtures).

### 7. `test-support` feature

`graph::testing` is `#[cfg(test)]`, so `sway-nodes`, `sway-editor` and
`sway-runtime` each re-implement `trace_world` / `tick`. Gate it behind a
non-default `test-support` feature instead, exporting `trace_world`,
`tick_once`, `read_field`, `set_field` and the fixture kinds. Downstream crates
add `sway-graph = { workspace = true, features = ["test-support"] }` under
`[dev-dependencies]` and delete their copies.

This is the one place `bevy_time` stays a real (optional) dependency of the
engine.

### 8. Crate moves

| From | To | Why |
|---|---|---|
| `sway-nodes` (whole crate) | `sway-base-nodes` | Named for its domain: the base value/signal nodes every project starts from. |
| `sway-graph/src/viewport_input.rs` | `sway-viewport-input` | Shared vocabulary between two crates that cannot depend on each other. |
| `sway-runtime/src/viewport/` | `sway-editor-viewport` | Editor-only interaction (camera orbit, gizmo, picking). Nothing on stage runs it. |
| `sway-runtime/src/sprite_material.rs` + `nodes/sprite_material.rs` | one module | One concept, two files, split only by the two-node-model transition that is over. |
| `sway-runtime/src/frame_sequence.rs` + `nodes/frame_sequence.rs` | one module | Same. |

`sway-editor-viewport` depends on `bevy` (facade), `sway-graph`,
`sway-viewport-input` and — for `NodeEntities`, which picking resolves through —
`sway-runtime`. `sway-runtime` must therefore *not* depend on it; `sway-app`
adds it in editor builds, exactly as it adds `EditorViewportPlugin` today.

`sway-base-nodes` holds `Oscillator`, `Envelope`, `Math`, `Remap` and
`MakeVec3`, each in its own module with its pure helper folded in, and one
`BaseNodesPlugin`. Deleted on the way: `beat.rs` (nothing outside itself
references it), `NoteField`, and `tests/traces.rs` (golden traces of pure
functions from the removed wire model — the node kinds' own tests cover the same
arithmetic).

### 9. `Envelope` takes time as an inlet

Today `Envelope::evaluate` reads `Time<Fixed>` from `&World` to accumulate
`state.now`. Reshape it to `inlets.time: f32` and compare gate timestamps
against that, mirroring `Oscillator`, which already works this way and ignores
its `&World`.

Consequence: `sway-base-nodes` needs no `bevy_time`, and every base node is a
pure function. The cost is that an envelope needs a time source wired in — in
practice `MidiTime`, which is what the demo already feeds `Oscillator`. There is
no wall-clock node kind in the tree today; if one is wanted it belongs in a
domain crate that may name `Time<Fixed>`, not in the base set. Recorded as an
open question rather than built here.

`state.now` disappears; `state.gate_on` / `state.gate_off` become timestamps in
the inlet's own time base. A graph saved before this change keeps its inlets
(state is never serialized), so the only visible difference is that an envelope
whose `time` inlet is unconnected holds still instead of free-running.

### 10. Two plugins per node domain collapse to one

`sway-midi` exposes `MidiPlugin` (resources, clock, transport systems) and
`MidiGraphNodesPlugin` (node kinds) and `sway-app` adds both. Fold the second
into the first. `MidiPlugin` already takes the `Receiver` it needs, so nothing
about its construction changes. Same for `sway-runtime`: `RuntimeNodesPlugin`
and `ProjectionPlugin` become one `RuntimePlugin`; `register_runtime_node_kinds`
stays a public function so schema-only tests keep their cheap path.

### 11. The `Vec3` node kind is renamed `MakeVec3`

The node kind keeps its behaviour — three scalar inlets in, one vector outlet
out — and loses only its name. `sway-base-nodes::Vec3` currently collides with
`bevy::math::Vec3`, which its own outlet is made of; `value.rs` already has to
write `use bevy::math::Vec3 as MathVec3` to say what the node produces. A node
kind named for the type it constructs, in a codebase where that type is also in
scope, is a name that has to be worked around at every use.

`MakeVec3` names the operation instead. `Vec3In`/`Vec3Out` become
`MakeVec3In`/`MakeVec3Out`, and the module moves from `value.rs` to
`make_vec3.rs`.

*Why not delete it, as an earlier draft of this design proposed.* Nested inlet
paths do make it possible to drive one component of a vector inlet directly —
`sway-graph`'s `a_nested_destination_writes_only_that_component` test proves an
edge into `point.y` writes only `y`. But the two are not the same tool: an edge
into `translation.y` reaches inside one consumer, while `MakeVec3` produces a
vector that fans out to many. `docs/architecture.md` §10's settled decision
("a `Vec3 { x, y, z }` value node with driveable components is what produces
them") stands, and this change does not revise it.

*Document consequence.* A document keys node kinds by the last segment of the
type path, so the rename changes `type: "Vec3"` to `type: "MakeVec3"` on disk.
That is a second reason the format goes to 4 (Decision 1b), and the demo's
`vec3A` / `vec3B` entries are rewritten with it.

## Risks / Trade-offs

- **A wide refactor with no behavioural target of its own can silently change
  behaviour.** → Order the work so the engine's own tests run green before any
  downstream crate is touched, and finish by loading `demo.sway.ron` in the real
  app. The graph golden traces, the document round-trip and the projector tests
  are the specific gates.

- **`sway-graph`'s command tests are written against an enum that ceases to
  exist.** → `command.rs`'s 23 construction sites are all tests of behaviour the
  `Graph` methods still have. They are rewritten as direct method tests, which is
  a rewrite rather than a deletion: each one keeps its assertion and loses its
  wrapper.

- **Deleting the command enum removes the mechanism that kept structural change
  out of the tick.** → The mechanism was never the queue. The tick runs inside
  `World::resource_scope`, so the `Graph` is genuinely absent from the `&World` a
  node sees — nothing reachable during a tick can call a mutation method. The
  queue only decided *when* an editor's edits land; that is still `PreUpdate`,
  now by where the applier is scheduled rather than by what type it consumes.

- **Annotations depend on the type registry, so an annotation can now fail to
  load.** → It is reported and skipped like an unresolved kind or path, never
  fatal, and the node still loads. The failure mode is a surface annotating with
  an unregistered type, which is a bug in that surface and worth a diagnostic.

- **`HashMap` iteration order makes save output unstable.** → Save sorts keys
  before writing, so a committed asset's diff stays clean even though storage is
  unordered.

- **`sway-editor-viewport` depending on `sway-runtime` is a domain-to-domain
  edge, which the new rules forbid.** → It is not a node domain; it is an editor
  surface, and the dependency runs surface → runtime, the same direction
  `sway-app` runs. The rule is stated as "no domain crate depends on another
  domain crate" precisely so this stays legible.

- **Deleting `tests/traces.rs` removes the only golden-trace file in
  `sway-nodes`.** → It traces `envelope_tick`, `math_value`, `remap_value` and
  `oscillator_value` — pure functions of a model that no longer exists, and the
  node kinds that replaced them have their own tests. Removing dead traces is
  the point of the change, not a casualty of it.

- **Every existing project file stops loading.** → Accepted deliberately: a
  compatibility read would put the editor's vocabulary back into the document.
  The blast radius is one file (`demo.sway.ron`), rewritten here, and the
  version check turns the break into a clear message rather than a serde error.
  Anyone holding an unversioned copy of a project rewrites `pos: (x, y)` as
  `metadata: {"pos": "x,y"}` and bumps the version line.

- **`sway-document`'s module is named after the format version, so bumping it
  renames the module.** → `v3` → `v4` is mechanical (`sway-app` is the only
  external caller that spells the module path), and leaving the module at `v3`
  while it implements version 4 would be exactly the kind of drift this change
  exists to remove.

## Migration Plan

Sequenced so the workspace compiles at each group boundary:

1. **New crates, no callers** — create `sway-viewport-input`; move the module in;
   have `sway-graph` re-export it temporarily so nothing breaks yet.
2. **Engine surface** — metadata map, reflected `SetField`, selection removal,
   `Compat`/`Target` merge, `NodeParts` removal, visibility narrowing,
   `test-support` feature. Fix `sway-document` (including the `v3` → `v4`
   rename and the format bump), `sway-editor`, `sway-runtime` and `sway-app` in
   the same group; drop the temporary re-export.
3. **Node crates** — rename `sway-nodes`, rename `Vec3` to `MakeVec3`, reshape
   `Envelope`, delete the dead modules, collapse the double plugins in
   `sway-midi` and `sway-runtime`.
4. **Runtime split** — extract `sway-editor-viewport`, fold the split modules,
   delete the unreachable pipelines, fix the manifests.
5. **Assets and docs** — rewrite `demo.sway.ron` for version 4, annotations and
   the `MakeVec3` rename; add `docs/architecture.md` §11 plus the corrections to
   §5 and §8.

Rollback is per group and each is a self-contained commit. Group 2 is the one
that cannot be reverted in isolation once a project file has been saved through
it — the saved file is version 4 and an earlier build will refuse it. Nothing
else in the change touches an on-disk or external interface.

## Open Questions

- Is a wall-clock `Time` node kind wanted now that `Envelope` no longer reads
  `Time<Fixed>` itself? Today `MidiTime` is the only time source, which is
  sufficient for the MVP's MIDI-locked target. If one is wanted it is a new node
  kind in a crate that may name `Time<Fixed>` — additive, and it does not change
  anything specified here.
- `sway-geo` is dormant, has no node kinds and no plugin, and after this change
  no dependency on the engine. Whether it should be deleted or kept as the
  landing site for the post-MVP geometry work is a roadmap question, not a
  structure one.
