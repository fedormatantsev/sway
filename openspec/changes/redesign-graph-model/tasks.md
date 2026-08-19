Migration order follows design.md — Migration Plan. The new model lands beside
the old one; nothing is deleted until group 9. The demo document must render
correctly at the end of groups 5, 6 and 7.

## 1. sway-graph — the graph model

- [x] 1.1 Add `NodeId` as a generational index and `Graph` holding `Vec<Node>` plus a free list, so a deleted id never resolves to a later node (`graph`: A graph is nodes and edges).
- [x] 1.2 Add the `Node` container with `inlets` / `state` / `outlets` as three nested reflected parts, `()` for an empty part, plus node kind and editor position (`graph`: A node is inlets, state, and outlets).
- [x] 1.3 Add `Edge { src: (NodeId, path), dst: (NodeId, path), slot }` and the edge list on `Graph`.
- [x] 1.4 Add node-kind registration: a `#[reflect_trait]` for evaluation plus the reflected type of each part, resolvable from the type registry.
- [x] 1.5 Add per-node dirty tracking — a dirty set written by commands, propagation and evaluation, drained by a consumer (`graph`: Changes are tracked per node). Cover that an equal write reports nothing.
- [x] 1.6 `cargo test -p sway-graph` covers 1.1–1.5.

## 2. sway-graph — legality, commands and invariants

- [x] 2.1 Add path resolution over a node's parts via `bevy_reflect::GetPath`, with `inlets.` / `outlets.` prepended by the resolver so stored paths stay short (`graph`: An edge addresses fields by path).
- [x] 2.2 Add the legality rule — `D == S`, `D == Option<S>`, `D == Vec<S>` — decided at connect time from reflected type info.
- [x] 2.3 Add the graph command set (create, delete, set field, move, connect, disconnect, select) replacing `EditorCommand`.
- [x] 2.4 Enforce the invariants `Relationship` used to provide: reject self-connections, replace rather than duplicate on a single-connection inlet, and drop every edge naming a deleted node (`graph`: The graph rejects connections that would break its invariants).
- [x] 2.5 `cargo test -p sway-graph` covers 2.1–2.4, including that an illegal connection is refused without evaluating anything.

## 3. sway-graph — rebuild and tick

- [x] 3.1 Port `order.rs` from `Entity` to `NodeId`, keeping deterministic tie-breaking and cycle-append. Delete the false-cycle caveat from its docs — node granularity resolves it (`graph`: Evaluation order).
- [x] 3.2 Build variadic inlets during rebuild: collect edges per `(node, inlet path)`, sort by slot with `NodeId` breaking ties, size the `Vec` to the edge count and fill in order (`graph`: Inlets may be optional or variadic).
- [x] 3.3 Emit no propagate step for a valueless edge, but keep it as a sort constraint (`graph`: An edge may carry no value).
- [x] 3.4 Implement the propagate step with `slice::get_disjoint_mut` over the node `Vec`, guarded by a reflect-equality check so equal values do not dirty.
- [x] 3.5 Implement the evaluate step and drive the tick through `World::resource_scope`, so a node holding `&World` cannot reach the graph (`graph`: Node evaluation reads inlets and writes state and outlets).
- [x] 3.6 Golden-trace tests at a fixed delta over a fixture graph; assert a two-hop chain resolves in one tick and a cycle still ticks. `cargo test -p sway-graph`.

## 4. sway-nodes — value nodes

- [x] 4.1 Port `Vec3`, `Math` and `Remap` to the nested three-part shape.
- [x] 4.2 Port `Oscillator`, `Lfo` and `Envelope`, moving phase and similar memory into the `state` part.
- [x] 4.3 Port `MidiTime` as an ordinary node that reads the transport resource through `&World` during evaluation — no injection phase and no MIDI type named by `sway-graph` (`graph`: an external time source is an ordinary node).
- [x] 4.4 Port the existing trace tests to the new node shapes. `cargo test -p sway-nodes`.

## 5. sway-runtime — projection and protocols

- [x] 5.1 Add the projector layer: a `NodeId -> Entity` map, a spawn/update/despawn pass driven by the dirty set, and projector ordering by graph order (`architecture`: The graph is the authored model and the world is derived).
- [x] 5.2 Add the protocol markers and their reflected traits — `SceneMaterial`/`MaterialNode`, `ImageSequence`/`ImageSequenceNode`, `MeshSource`/`MeshNode`, `SceneChild`.
- [x] 5.3 Allocate handles structurally at node creation and update asset contents per tick for dirty producer nodes, so a connection is never waiting on a handle that does not exist yet.
- [x] 5.4 Port `MeshAsset`, `PlaneMesh` and `FrameSequence` as producer nodes that own their assets and expose them through their protocol traits (`nodes`: A node that owns an asset does not pass it along a connection).
- [x] 5.5 Port `PbrMaterial` and `SpriteMaterial` as material nodes that attach their own typed material to every connected scene node (`nodes`: A material node attaches itself to what it is connected to).
- [x] 5.6 Add the scene nodes `MeshNode`, `Group`, `Camera`, `DirectionalLight`, `PointLight`, with `Group` carrying transform and children only and refusing geometry (`nodes`: The scene node set is fixed).
- [x] 5.7 Project `children` edges into parenting so transform propagation stays Bevy's, inserting a parent only where a child connection exists.
- [x] 5.8 `cargo test -p sway-runtime` covers 5.1–5.7, including that deleting a node despawns its entity and releases its asset.

## 6. sway-document — format version 3

- [x] 6.1 Define the version 3 shape: nodes keyed by stable id, an edge list naming two ids, two paths and a slot (`document`: A document is nodes and edges keyed by stable ids).
- [x] 6.2 Mint stable ids once at node creation and build the `id -> NodeId` map at load. No reconcile pass and no claim pass.
- [x] 6.3 Serialize inlets only, per node kind, via `TypedReflectSerializer` / `TypedReflectDeserializer` on the inlets type (`document`: A document stores inlets only).
- [x] 6.4 Report and skip an unknown node kind, an edge naming a missing id, and an edge naming a missing path — without preventing the rest of the document from loading.
- [x] 6.5 Reject a version other than 3, and a document declaring the same id twice, as whole parse errors.
- [x] 6.6 Rewrite `demo.sway.ron` in the version 3 shape, splitting each geometry-plus-transform entity into a producer node and a scene node, and folding the three separate `cube.gltf` references into one shared mesh node (`nodes`: Geometry, material and placement are separate nodes).
- [x] 6.7 Round-trip tests: load, save, reload is identical; deleting a node leaves every other id untouched. `cargo test -p sway-document`.

## 7. sway-editor — read path and commands

- [x] 7.1 Delete `snapshot.rs`. Populate widgets from a reflected read of `&Graph` during the presenter's step 0, with no `Arc`, mutex or copy (design D11).
- [x] 7.2 Derive inspector controls from each field's reflected type; show a field with no control read-only rather than omitting it (`editor`: The editor reads the graph without a parallel model).
- [x] 7.3 Re-address sockets to `(NodeId, field path)`, discovered from the node kind's declared inlets and outlets so an unconnected inlet still has a socket.
- [x] 7.4 Re-address the canvas: edges carry two paths and a slot, and attach to the sockets whose keys are those paths.
- [x] 7.5 Build the palette from registered node kinds, replacing `ComponentDocRegistry` and retiring roadmap D4.
- [x] 7.6 Point drag-to-connect at the graph command set, surfacing refusal for illegal types, self-connections and replacement on a single-connection inlet.
- [x] 7.7 Allow reordering the edges on a variadic inlet by changing one edge's slot (`editor`: Edges carry two field paths and an ordering key).
- [x] 7.8 Make the inspector accept edits to connected fields (`editor`: Inspector shows inlets only — a connected field is still editable).
- [x] 7.9 `cargo test -p sway-editor`.

## 8. sway-app — project lifecycle

- [x] 8.1 Make `build_app` take a project directory and set it as the asset root, so every path a graph names resolves relative to it (`architecture`: A project is a directory).
- [x] 8.2 Rebuild the `App` on project open, keeping the window and the wgpu device, and re-establish the viewport texture through `set_viewport_view`.
- [x] 8.3 Remove Save As to another directory; Save writes back to the file the project was opened from.
- [x] 8.4 Ignore `AssetEvent::Modified` for the graph asset, and delete `LastApplied` / `should_skip`. Leave `watch_for_changes_override` on so content still hot-reloads (`architecture`: Reloading a project is an explicit action).
- [x] 8.5 Gate the tick and the projectors on every asset reporting loaded; leave the MIDI drain ungated so the pulse clock stays continuous (`architecture`: Evaluation waits for assets; input capture does not).
- [x] 8.6 Point the gizmo at the graph command set instead of writing `Transform` directly, and resolve picking `Entity -> NodeId` for selection only (`architecture`: Authoring writes reach the world only through the graph).
- [x] 8.7 Update `crates/sway-app/tests/demo_document.rs` for the version 3 demo. `cargo test -p sway-app`.

## 9. Removal

- [x] 9.1 Delete `wire.rs`, `dispatch.rs`, `watch.rs`, `registry_components.rs`, `behaviour.rs` and `register.rs` from `sway-graph`, and `test_wires.rs`.
- [x] 9.2 Delete the `field_wire!` macro and every expansion of it across `sway-nodes` and `sway-runtime`, and `wire_testing.rs`.
- [x] 9.3 Delete `claim.rs` and the four-pass reconcile in `sway-document`.
- [x] 9.4 Remove `EditorPos` and `HiddenFromEditor` as components now that position is a node field.
- [x] 9.5 `cargo build --workspace` clean, no dead-code warnings from the removal.

## 10. Verification

- [x] 10.1 `cargo test --workspace` passes.
- [x] 10.2 `cargo clippy --workspace --all-targets` clean; `cargo fmt --all --check` clean.
- [ ] 10.3 Run the editor on the version 3 demo: create a node, connect it, edit a field, drag a gizmo handle, save, reopen — all without leaving the editor.
- [ ] 10.4 Confirm by eye that the demo renders as it does today: cubes animating from MIDI time, both sprite layers interpenetrating `cubeC` and each other.
- [x] 10.5 Update `docs/architecture.md` to the new model, replacing §2 (wire contract), §4 (order/rebuild/tick), §6 (scene composition) and §7 (graph state), and remove the false-cycle open question from §4 and §10.
