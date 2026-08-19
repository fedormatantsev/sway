## Why

The graph is grounded in the ECS: a node is an entity, a connection is a Bevy
`Relationship` component, and authoring is plain ECS mutation. Architecture §4
and §5 justified that with two claims — the editor would read the world directly
with "no second schema", and there would be "no `connect` API". Both are dead in
the tree. `sway-editor/src/snapshot.rs` is 1092 lines building a per-frame
`WorldSnapshot`; `sway-graph/src/command.rs` is 1044 lines of `EditorCommand`
plus its applier. That is 2136 lines of adapter whose only job is to undo the
ECS grounding at each boundary, and the costs it was meant to buy off are all
still being paid:

- 19 wire types, each a `Relationship` + `RelationshipTarget` + macro expansion,
  to express what is `(node, port) -> (node, port)`.
- A type-registry lock, a `FromReflect` stack copy and two `ReflectComponent`
  lookups per propagate step, per tick (`dispatch.rs`).
- The standing false-cycle question, which exists only because `Entity` is
  forced to be the sort vertex.
- `claim.rs` and a four-pass reconcile in `apply.rs`, because `Entity` is not a
  stable identity and `DocId` has to shadow it.
- Node state stored as a component, so it is inspectable and in the world,
  contrary to architecture §2.

**Why now:** M9 (events) is the last unbuilt subsystem, and it does not fit the
current model at all — `Behaviour::evaluate` takes inlets, state and outlets,
and `TriggerIn<W>` is a fourth thing with no parameter to arrive in. Doing this
before M9 means events land in a model that has room for them; doing it after
means building `sway-events` twice.

## What Changes

- **BREAKING: the graph moves out of the ECS into a `Resource`.** Nodes and
  edges are a plain data structure, not entities and relationship components.
  The tick is an exclusive system over that resource via `World::resource_scope`.
- **A node is one reflected Rust type with three parts** — `inlets`, `state`,
  `outlets` — as nested sub-structs. Absent parts are `()`. One concrete type
  per node kind replaces today's split, where the inlet type implements
  `Behaviour` and state and outlets are separate types resolved by `TypeId`.
- **BREAKING: `evaluate(&mut self, world: &World)`.** `&mut self` reaches all
  three parts; `&World` lets a node read external resources. `MidiTime` reads
  the MIDI snapshot itself, so no MIDI machinery enters `sway-graph` and no
  pre-tick injection phase exists.
- **BREAKING: all 19 wire types are deleted.** An edge is
  `(src NodeId, outlet path) -> (dst NodeId, inlet path, slot)`. The path names
  a declared field of the part. A compound inlet is wired as a whole; to drive
  one component, that component is its own inlet (`translation` / `rotation` /
  `scale` on scene nodes, `"x"` / `"y"` / `"z"` on a `Vec3` node). `Vec3XFrom`
  / `Vec3YFrom` / `Vec3ZFrom` become those inlet names rather than nested
  destinations on a transform.
- **`Option<T>` and `Vec<T>` inlets are first-class.** An unwired `Option<T>` is
  `None` and the node decides; a `Vec<T>` inlet accepts many edges, ordered by
  the edge's `slot` — a sort key, not an array index, so the `Vec` is derived
  from the edge set rather than pre-sized. This retires the MVP exclusion on
  variadic inlets, which existed because `Relationship` had no fan-in.
- **Edges never carry `Handle<T>`.** Handles live only in node state, which is
  never serialized. Asset and scene connections are **markers** — ZST outlets
  paired with a `#[reflect_trait]`: `SceneMaterial`/`MaterialNode`,
  `ImageSequence`/`ImageSequenceNode`, `MeshSource`/`MeshNode`, `SceneChild`.
  A marker edge propagates nothing and exists to declare legality and ordering.
- **The world becomes derived.** Projector systems read the graph and maintain
  `Assets<T>` entries and scene entities. Projection is asymmetric and
  hand-written per node type: a material node owns an `Assets<M>` entry and no
  entity; a scene node owns an entity. Handles are allocated structurally at
  node creation; asset contents update per tick.
- **BREAKING: `snapshot.rs` is deleted.** The editor's read pass reads `&Graph`
  directly and walks `TypeInfo`; no `Arc`, no mutex and no copy, because the UI
  read and the tick never overlap in `EditorPresenter::present`. Widgets still
  cache what they paint, which is masonry's retained model, not a schema.
- **BREAKING: all authoring writes go to the graph, never to the world.** The
  gizmo emits the same `SetField` command the inspector does. Picking maps
  `Entity -> NodeId` for selection only; no value flows world to graph.
- **BREAKING: the project is a directory that is the asset root, one project per
  `App`.** The graph is loaded as a `GraphAsset` from inside it, used to
  initialize the resource, and saved by serializing the resource back to the
  asset's path. Save-As to another directory is removed.
- **The graph's own hot reload is removed.** `AssetEvent::Modified` is ignored
  for `GraphAsset`, deleting `LastApplied` and `should_skip`. The file watcher
  stays on for content, so texture and mesh reload survives.
- **A load gate.** The tick and the projectors do not run until every asset is
  loaded. The MIDI drain is never gated, so the pulse clock cannot lose time.
- **Retires roadmap D4** (`#[require]` companions and `ComponentDocRegistry`):
  a node type is a complete struct, so there is nothing to supply.
- **Resolves the false-cycle open question** (architecture §4, roadmap open
  questions). `evaluate` reads every inlet and writes every outlet, so at node
  granularity the coupling the sort assumes is the coupling that exists.

Out of scope: `sway-events` itself (M9 builds on this model, it is not part of
it), `EnvironmentMap` (M8's other half), restoring an authored value on
disconnect, and the authored-versus-wired inlet distinction.

## Capabilities

### New Capabilities

- `architecture`: the layering and ownership model this change establishes — the
  graph as a `Resource` initialized from a `GraphAsset`, the world as a derived
  artifact maintained by projectors, one project per `App` with the project
  directory as the asset root, and the single-direction authoring rule. This is
  the cross-cutting domain named in `openspec/config.yaml`; it does not exist
  yet and this change is the first that needs it.

### Modified Capabilities

- `graph`: every requirement changes. `Wire is a relationship on the consumer`,
  `Reflection is the wire catalog`, `Default wire evaluation is a reflected
  field copy`, `Behaviour is inlets, state, and outlets` and `Authoring watches
  include behaviours` are replaced by the node/edge model, the nested three-part
  node, `evaluate(&mut self, &World)`, and marker edges. `Evaluation order`
  survives in substance but its vertex becomes `NodeId`.
- `document`: every requirement changes. `Wire keys are full type paths` and
  `Only inlets are document components` are replaced by node entries and an
  edge list with field paths; `Format version 2` becomes version 3.
- `editor`: every requirement changes. `Inlet identity is the wire type path`,
  `Connect and disconnect name a type path` and `Edges carry the wire type path`
  are replaced by `(NodeId, field path)` addressing. `Inspector shows inlets
  only` survives in substance against the new node shape.
- `nodes`: `A mesh node carries no material until one is wired` and `A material
  wire supplies the material component it writes into` change — the material is
  a marker edge and the material node attaches itself through `MaterialNode`,
  rather than a wire type inserting `MeshMaterial3d<M>` on connect.
- `runtime`: `A sprite material is a node wired to a mesh` and `A sprite
  material takes its colour and depth runs from wires` change to marker edges.
  The frame-number, displacement, occlusion and tint/opacity requirements
  survive in substance and are restated against the new inlet shape.

## Impact

- `sway-graph` — replaced, not edited. `wire.rs`, `dispatch.rs`, `watch.rs`,
  `registry_components.rs`, `behaviour.rs` and `register.rs` go; `order.rs`
  survives with `Entity` swapped for `NodeId`; `command.rs` becomes the graph
  command stream.
- `sway-document` — `claim.rs` and the four-pass `apply.rs` reconcile go. Load
  is `TypedReflectDeserializer` per node inlets struct; save is the serializer.
- `sway-editor` — `snapshot.rs` is deleted outright; widgets are populated from
  a reflected read of `&Graph`. `canvas.rs`, `inspector.rs` and `palette.rs`
  re-address against `(NodeId, field path)`.
- `sway-nodes`, `sway-runtime` — every node type is rewritten into the nested
  three-part shape. `field_wire!` and all its expansions go. `sway-runtime`
  gains the projector systems.
- `sway-app` — `build_app` takes a project directory; the shell rebuilds the
  `App` on project open, keeping the winit window and the wgpu device, which
  `RenderCreation::manual` already permits.
- `sway-midi` — unchanged in substance; `MidiTime` becomes a node that reads
  its resources through `&World`.
- No new third-party dependencies. `slice::get_disjoint_mut` requires nodes to
  be stored in a `Vec` addressed by generational index.
