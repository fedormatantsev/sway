## Context

See `proposal.md` — Why. The constraints that shape the approach:

- `sway-graph` must not depend on `bevy_render`, MIDI types, or the document
  format (architecture §5). That survives this change.
- `sway-app` owns winit and the wgpu device through `sway-gpu`, and
  `sway_runtime::headless::build_app` receives the device via
  `RenderCreation::manual`. The `App` is already constructed by a closure held
  in `shell.rs` and called *after* the device exists — so the `App` is a
  disposable object around a device that outlives it.
- Bevy's asset root is fixed when `AssetPlugin` builds. There is no supported
  way to repoint it at runtime.
- Bevy assets are load-only; there is no `AssetServer::save`.
- The tick must stay deterministic enough for golden-trace tests at a fixed
  delta (architecture §9).

## Goals / Non-Goals

**Goals:**

- One uniform node model that expresses value nodes, source nodes, asset
  producers and scene nodes without categories or special cases.
- Connectivity is data, not Rust types: adding a node kind never adds a wire
  type.
- The graph is testable without a Bevy `App`.
- The document is readable and hand-editable, and diffs cleanly under git.

**Non-Goals:**

- Live graph patching during a show. Unchanged from architecture §1.
- Restoring an authored value on disconnect, and the authored-versus-wired
  inlet distinction. Both explicitly deferred.
- `sway-events`. This change makes room for M9; it does not build it.
- Parallel evaluation of the tick. The walk stays serial by design.

## Decisions

### D1 — The graph is a `Resource`, initialized from an `Asset`

`GraphAsset` is the on-disk artifact, loaded through the normal asset pipeline
alongside the images and meshes it references. A startup system uses it to build
the live `Graph` resource, which holds a `Handle<GraphAsset>` as a backlink so
save can resolve a path via `AssetServer::get_path`.

The asset is a **loading mechanism only**. It is not kept in sync with the
resource and is not consulted after initialization.

*Alternatives considered.* Making `Assets<Graph>` the live model: every edit
goes through `get_mut`, which fires `AssetEvent::Modified` and marks the whole
asset changed, giving no granular change detection, and `ResMut<Assets<Graph>>`
contends with everything else touching assets. Keeping the asset live as a
second copy for dirty detection: a `bool` on the resource set by the command
applier is cheaper and more direct than comparing two graphs.

Save is not an asset-pipeline operation — `serialize(&graph)` then
`fs::write(root.join(path))`. The asset half of the round-trip is
one-directional and the backlink does identity duty, not IO duty.

### D2 — One project per `App`; the project directory is the asset root

`AssetPlugin { file_path: <project dir> }` is set when the `App` is built, so
the graph, the sprite folders and the meshes are all root-relative and the
project directory is portable. This retires the objection recorded at the top of
`sway-document/src/file.rs` — a dialog-picked absolute path cannot round-trip
through the `AssetServer` — by making a project a *directory*, not a file.

Opening a project drops the `App` and calls `build_app` again with the new root.
The winit window, the wgpu device and the masonry UI state all live outside the
`App` and survive. `set_viewport_view` must re-establish the viewport texture,
which it already does on resize.

*Alternatives considered.* An indirect `AssetSource` holding an
`Arc<RwLock<PathBuf>>` root, swappable at runtime: works, but every handle must
be dropped on swap because cached assets are keyed by relative path and the same
key would mean a different file. A fixed workspace root with projects as
subdirectories: removes the problem entirely, but gives up opening a folder
anywhere on disk.

*Consequence.* Save-As to a different directory is removed — the running `App`'s
asset root would be wrong for the destination. Copying a project is a
file-manager operation.

*Fallback.* If `App` teardown proves leaky, re-exec the process with the new
project path. The two differ only in who calls `build_app`.

### D3 — A node is one reflected type with three nested parts

```rust
#[derive(Reflect)] struct OscIn    { frequency: f32, amplitude: f32 }
#[derive(Reflect)] struct OscState { phase: f32 }
#[derive(Reflect)] struct OscOut   { out: f32 }

#[derive(Reflect)]
struct Oscillator { inlets: OscIn, state: OscState, outlets: OscOut }
```

Absent parts are `()`, which `bevy_reflect` implements `Reflect` for. Every node
has the same shape, so nothing in the tick, the serializer or the editor
unwraps an `Option`.

*Alternatives considered.* A flat struct with roles marked by custom field
attributes (`#[reflect(@Inlet)]`). Less boilerplate across ~20 node types and
shorter paths, but the deciding argument is ser-de: nested gives
`TypedReflectSerializer` / `TypedReflectDeserializer` applied to a whole type —
the machinery `apply.rs` already uses — where flat requires building a filtered
`DynamicStruct` on every save and merging a partial one on every load, custom
code sitting on the format path forever. Nested also removes the constraint that
an inlet and an outlet cannot share a name, and lets a node's logic be a free
`fn(&In, &mut State, &mut Out)` testable with no `World` at all.

The path-length objection does not apply: an edge's source is always an outlet
and its destination always an inlet, so the resolver prepends `inlets.` /
`outlets.` and the document stores `"frequency"`, not `"inlets.frequency"`.

A derive macro that generates the nested form from a flat declaration is
additive and can be introduced later. The workspace has no proc-macro crate
today, so it is not part of this change.

### D4 — `evaluate(&mut self, world: &World)`

`&mut self` reaches inlets, state and outlets. `&World` lets a node read
external resources, which is how `MidiTime` reads the MIDI snapshot without
`sway-graph` naming a MIDI type and without a pre-tick injection phase.

`Graph` is a resource inside `World`, so the tick must use
`World::resource_scope` to hold `&mut Graph` and `&World` at once. The tick
therefore stays an **exclusive system**. That is acceptable — the walk is serial
by design (architecture §4) — and it buys a useful invariant for free: inside
`resource_scope` the `World` genuinely does not contain `Graph`, so a node
cannot reach the graph through `&World`. No reentrancy, and no node can read
another node's outlet behind the edge list's back. This is intentional, not
incidental.

*Alternatives considered.* A narrowed `TickCtx` carrying typed context: keeps
the tick non-exclusive and evaluation pure, but either makes `sway-graph` depend
on `sway-midi` or requires a type-erased side channel, and adds a node category
for external-state sources.

*Consequence.* Evaluation is no longer a pure function of `(inlets, state)`, so
architecture §9's bit-identical golden traces hold **by discipline**: world
reads must stay confined to resources the trace controls. A node reading entity
state or `Time<Real>` breaks reproducibility. This is a rule for node authors,
recorded here so it is not rediscovered through a flaky test.

### D5 — An edge is `(NodeId, path) -> (NodeId, path, slot)`

An edge names a **declared field** of the source outlets and of the destination inlets. A compound inlet is connected as a whole: `Transform` to `Transform`, `Vec3` to `Vec3`. Driving one component of a compound is not a nested path on that inlet — the component is its own declared inlet, or a node constructs the compound from parts.

`SetField` still resolves through `bevy_reflect::GetPath`, so an inspector or gizmo can edit `translation` as a `Vec3` (and, if needed, a nested path such as `"x"` on a `Vec3` node). That is authoring a value, not an extra inlet.

Scene nodes therefore flatten Bevy's `Transform` into `translation`, `rotation` and `scale`. The projector builds the entity `Transform` from those three. The demo's `vec3A.out → cubeA.translation` is then a legal top-level edge and a canvas socket.

Paths are stored short: the resolver prepends `inlets.` / `outlets.`, so the document says `"translation"`, not `"inlets.translation"`.

`propagate_field_copy` in `wire.rs` hand-rolls single-level field access and is
superseded entirely.

Legality is checked once at connect time by matching reflected types:

```
outlet type S -> inlet type D is legal iff
  D == S            direct
  D == Option<S>    optional inlet
  D == Vec<S>       variadic inlet, and multiple edges may land here
```

`slot` is a **sort key, not an array index**. At rebuild, edges landing on one
`Vec<T>` inlet are collected, sorted by slot with `NodeId` breaking ties, and
the `Vec` is resized to the edge count and filled in that order. Gaps are
therefore free — author slots 10, 20, 30 and inserting between two layers is a
single write with no renumbering cascade. Non-variadic inlets sit at slot 0 and
take the same code path.

*Alternatives considered.* Encoding the index in the path (`"layers[2]"`, which
`GetPath` accepts): one mechanism instead of two, but the `Vec` would have to be
pre-sized and sparse slots would need holes. As a separate sort key the `Vec` is
derived from the edge set, which is the property that matters.

### D6 — Edges never carry `Handle<T>`; asset and scene connections are markers

Handles live only in node state, which is never serialized — so "handles are not
serialized" is implied rather than stated as a rule.

A **protocol** is a pair: a ZST marker type used as an outlet, and a
`#[reflect_trait]` the projector calls. A node joins a protocol by declaring an
outlet of the marker type and implementing the trait.

| protocol | marker | trait |
|---|---|---|
| material | `SceneMaterial` | `MaterialNode::attach(&self, &mut EntityCommands)` |
| image sequence | `ImageSequence` | `ImageSequenceNode::texture(&self) -> &Handle<Image>` |
| mesh source | `MeshSource` | `MeshNode::handle(&self) -> &Handle<Mesh>` |
| hierarchy | `SceneChild` | — (the projector needs only the `NodeId`) |

This removes the last type erasure. A material node is the only thing that ever
knows its concrete `M`, and it inserts `MeshMaterial3d<M>` itself — no
`UntypedHandle` and no `TypeId -> insert fn` registry, which an earlier
handle-carrying draft required.

Marker edges propagate nothing (a ZST copy is a no-op) but **stay in the sort**,
which gives projector ordering for free — needed for producer chains such as
`FrameSequence -> SpriteMaterial`, where the published array texture only exists
once the folder resolves.

*Consequence.* Marker inlets are pure schema: `children: Vec<SceneChild>` has no
useful runtime value and exists to declare that the port exists, its type, and
that it is variadic. Projectors read the edge index, not the field. This must be
stated in the specs so the field is not later "optimized away" as unused.

*Known limit.* A node that *selects* between assets cannot be a value node,
because the thing being switched never flows. It becomes a marker node with
several inputs and a projector that picks. Nothing in the MVP needs it.

### D7 — The world is derived; projectors are hand-written per node type

Projection is deliberately **not** a generic mirror. Each projector knows what
it targets: a material node owns an `Assets<M>` entry and no entity; a scene
node owns an entity. Graph shape and world shape differ, and `ChildOf` is
inserted only when a `children` edge exists.

Handles are allocated **structurally** — a material node reserves its
`Handle<M>` when the node is created, and `AssetServer::load` returns a handle
synchronously. Asset *contents* update per tick for dirty producer nodes. Both
projectors then run post-tick with no ordering constraint between them, and a
handle inlet is never empty; only its content is ever pending.

The scene node set is closed: `MeshNode`, `Group`, `Camera`, `DirectionalLight`,
`PointLight`. `Group` is a distinct kind carrying translation, rotation, scale and children only,
rather than a `MeshNode` with an `Option`-shaped mesh — explicit on the canvas
and in the palette, at the cost of duplicating those fields. The two
implementations stay separate: no shared trait or base struct for the duplicated
`translation` / `rotation` / `scale` / `children` fields, because the duplication is a few lines and a
shared supertype would be the first crack in the closed set.

**Authoring is single-direction.** Commands write authored fields; external
systems write source-node outlets and producer handles; the tick writes
propagated inlets, state and pure outlets. Nothing else writes the graph, and no
value flows world to graph. The gizmo emits the same `SetField` command the
inspector does — `translation`, `rotation` and `scale` on the scene node, not
a nested path through a `Transform`. Picking returns an
`Entity` only to look up a `NodeId` for selection.

### D8 — `Vec<Node>` addressed by generational `NodeId`

Propagate needs `&outlets` of one node and `&mut inlets` of another
simultaneously. `slice::get_disjoint_mut([i, j])` gives that with no allocation;
a `SlotMap` would force cloning the source value into a temporary per edge per
tick, which is what `dispatch.rs` does today and is not worth inheriting.

`get_disjoint_mut` returns `Err` on duplicate indices, so **self-edges must be
rejected at connect time**. That is one of several invariants `Relationship` was
providing silently and that now belong to the connect command:

| invariant | was | now |
|---|---|---|
| at most one source per inlet | `Relationship` | connect command, unless the inlet is `Vec<T>` |
| rewire eviction | `Relationship` | connect command |
| consumer despawn drops its wires | `bevy_ecs` | node removal drops its edges |
| no self-edges | `Relationship` | connect command |
| fan-out | `RelationshipTarget` | free — it is the edge list |

Architecture §2 claimed every such invariant was a property of Bevy
`Relationship`. They return as roughly forty lines of validation in one place,
against the nineteen wire types and `dispatch.rs` they were charging for.

### D9 — The document uses stable ids; `NodeId` is runtime-only

```ron
nodes: {
  "lfoA":  (type: "Oscillator", pos: (-460, 40), inlets: (period: 8.0, ...)),
  "vec3A": (type: "Vec3", pos: (-220, 40), inlets: (x: -0.8, ...)),
}
edges: [ (from: ("lfoA", "out"), to: ("vec3A", "y"), slot: 0) ]
```

Load builds a `HashMap<FileId, NodeId>`; ids are minted once at node creation.

*Alternatives considered.* Dense indices, where a node's file position *is* its
`NodeId.index` and generation starts at 0 — an exact round-trip with no id space
at all, but deleting one node shifts every later index, so a one-node deletion
rewrites most of the file and destroys the git history of a format meant to be
read and occasionally hand-edited.

This is not a return to `claim.rs`. Ids are minted in one place at creation, not
reconciled against a parallel entity world every load.

### D10 — Load gate on; graph hot reload off; content watcher on

The tick and the projectors do not run until every asset reports
`LoadState::Loaded`. The **MIDI drain is never gated** — gating it would let the
pulse clock lose time and resume wrong.

`watch_for_changes_override` stays `Some(true)`, but `AssetEvent::Modified` is
ignored for `GraphAsset`. That deletes `LastApplied` and `should_skip` (the
suppression hack for the reload a Save triggers) while keeping texture and mesh
hot reload, which `frame_sequence.rs` explicitly depends on to mutate a
published array texture in place rather than thrashing every consumer.

*Alternative considered.* Disabling the watcher entirely. `watch_for_changes` is
per-`AssetPlugin`, not per-type, so this would strand the reasoning in
`frame_sequence.rs` and remove live iteration on sprite sheets.

### D11 — `snapshot.rs` is deleted; the editor reads the reflected graph

`EditorPresenter::present` is single-threaded and strictly sequential
(`sway-app/src/presenter.rs:150`): the world is read at step 0, masonry lays out
and paints from widget state at step 1, and `app.update()` runs at step 4. The
UI read and the tick never overlap.

So **no `Arc`, no mutex and no deep copy.** A borrow of `&Graph` only has to
survive step 0, not `app.update()`. The pass reads the resource directly and
pushes into widgets through `WidgetMut`.

The constraint that produced `snapshot.rs` was never concurrency — it is that
masonry widgets are `Box<dyn Widget>` with no lifetime parameter and therefore
cannot hold a borrow. That push-into-widgets pattern stays, and widgets keep
caching what they paint, which is masonry's retained model rather than a second
schema. What is deleted is the 1092 lines of hand-written view types between the
two: `NodeView`, `InletView`, `EdgeView`, `InspectorField`, `FieldKind`,
`TreeRow`. The inspector walks `TypeInfo` on the node's `inlets` struct; the
canvas walks nodes and edges directly. `FieldKind` becomes reflection's own
type information rather than a parallel enum.

*Alternative considered.* Widgets holding `Arc<Graph>`, with the tick using
`Arc::make_mut`. Copy-on-write is genuinely free while the refcount is 1, so
dropping the `Arc` at end of frame costs nothing — but retaining it once,
anywhere, silently deep-copies the whole graph on every tick with no compile
error and no obvious symptom. The sequencing makes it unnecessary, so it is not
worth the failure mode.

## Risks / Trade-offs

- **Per-node dirty tracking must be built.** `ResMut<Graph>` marks the whole
  resource changed, so projectors have no `Changed<T>` to filter on. → The
  graph carries its own dirty set, written by propagate, evaluate and the
  command applier, and drained per projector. The never-write-an-equal-value
  rule (architecture §7) is unchanged and is what keeps the set small.
- **`&World` in `evaluate` weakens golden-trace determinism.** → Recorded as a
  node-authoring rule in D4; world reads stay confined to trace-controlled
  resources.
- **The tick stays exclusive.** → Accepted; the walk was always serial, and the
  reentrancy invariant it buys is worth more than the parallelism it forgoes.
- **Rebuilding the `App` per project may leak or fail to re-establish render
  state.** → The viewport path already exists for resize; if teardown proves
  leaky, fall back to re-exec, which differs only in who calls `build_app`.
- **Marker inlets look like dead fields.** → Stated explicitly in the specs as
  port declarations.
- **Big-bang risk: this replaces `sway-graph` and rewrites every node type.**
  → See Migration Plan; the new model lands beside the old one and nodes
  migrate in batches behind a working demo document.
- **`sway-runtime` projectors give it knowledge of node types.** It already
  depends on `sway-nodes` (added by `add-sprite-material`), so this follows an
  established boundary rather than opening a new one.

## Migration Plan

1. Build the new `sway-graph` — `Graph`, `Node`, `Edge`, `NodeId`, rebuild,
   tick — beside the existing one. `order.rs` ports with `Entity` swapped for
   `NodeId`. No node types yet; tests use fixtures.
2. Port the value nodes (`Vec3`, `Math`, `Remap`, `Oscillator`, `Lfo`,
   `MidiTime`). These need no projector and prove `evaluate(&mut self, &World)`
   and value edges end to end.
3. Add the projector layer and the protocol markers, then port the producers
   (`MeshAsset`, `PlaneMesh`, `PbrMaterial`, `SpriteMaterial`, `FrameSequence`)
   and the scene nodes (`MeshNode`, `Group`, `Camera`, lights).
4. Port the document: stable ids, node entries, edge list, format version 3.
   Rewrite `demo.sway.ron` in the new format. Delete `claim.rs` and the
   four-pass reconcile.
5. Re-address the editor: delete `snapshot.rs` and populate widgets from a
   reflected read of `&Graph` (D11); canvas, inspector and palette move to
   `(NodeId, field path)`; the gizmo emits `SetField`.
6. Switch `build_app` to take a project directory and wire up the load gate.
7. Delete the old `sway-graph` surface — `wire.rs`, `dispatch.rs`, `watch.rs`,
   `registry_components.rs`, `behaviour.rs`, `register.rs` — and `field_wire!`
   with every expansion.

Rollback is per-step until step 7; the demo document renders correctly at the
end of steps 3, 4 and 5, which is the check at each boundary.
