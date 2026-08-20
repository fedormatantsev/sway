## 1. `sway-viewport-input` (new crate)

- [x] 1.1 Create `crates/sway-viewport-input` (deps: `bevy_math` only) and add it to the workspace members and `[workspace.dependencies]`.
- [x] 1.2 Move `ViewportInput`, `ViewportButton`, `ViewportKey`, `ViewportModifiers` and `normalize_viewport_pos` into it, with their tests.
- [x] 1.3 Leave `ViewportInputRx` behind for now (it moves in group 5) but re-home it on the new types.
- [x] 1.4 Have `sway-graph` re-export the new crate's items temporarily so downstream crates still compile; add `sway-viewport-input` to `sway-editor`, `sway-runtime` and `sway-app`.
- [x] 1.5 `cargo test -p sway-viewport-input` and `cargo check --workspace`.

## 2. `sway-graph`: node metadata replaces canvas position

- [x] 2.1 Replace `Node::pos`/`set_pos` with `metadata: HashMap<String, Box<dyn PartialReflect>>` plus `metadata()`/`metadata_mut()`; update `Node::new`/`Node::of` signatures.
- [x] 2.2 Confirm an annotation write does not dirty the node, and that an annotation reads back as the type it was written with; add both tests.
- [x] 2.3 `sway-document`: replace `NodeDoc::pos` with the annotation map, serialized through the untyped `ReflectSerializer`/`ReflectDeserializer` against the registry. Sort keys on save so an unchanged document saves byte-identically. Remove every mention of an editor or a canvas position from the crate's types, doc comments and module docs — after this task `sway-document` has no notion that an editor exists.
- [x] 2.4 `sway-document`: an annotation whose type is not registered is reported through `LoadDiagnostics` and skipped, leaving the rest of the node loaded.
- [x] 2.5 `sway-document`: bump `FORMAT_VERSION` to 4 and confirm the existing whole-file version check refuses a version-3 file naming both versions.
- [x] 2.6 `sway-document`: rename the `v3` module directory to `v4` and update `sway-app`'s `sway_document::v3::` imports.
- [x] 2.7 `sway-document`: tests for annotation round-trip preserving type, an unrecognised key surviving, an unregistered type being reported and skipped, an entry with no annotations, no key having a field of its own, a double save being byte-identical, and a version-3 file being refused by version.
- [x] 2.8 `sway-editor`: write canvas placement as a `Vec2` annotation under `"pos"`, directly rather than as a deferred edit. Confirm `Vec2` is registered on every path that loads or saves a document, including bare-`TypeRegistry` tests.
- [x] 2.9 `cargo test -p sway-graph -p sway-document -p sway-editor`.

## 3. `sway-graph`: the mutation API replaces the command enum

- [x] 3.1 Add `Graph::create(&TypeRegistry, type_path) -> Option<NodeId>`, carrying over the registered-node-kind check and `ReflectDefault` construction from the old `Create` arm.
- [x] 3.2 Add `Graph::set_field(NodeId, &str, &dyn PartialReflect) -> FieldWrite` with `FieldWrite { Written, Unchanged, Rejected }`: resolve the path within `inlets`, `reflect_equal`, `try_apply`, dirty only on a real write.
- [x] 3.3 Delete `GraphCommand`, `CommandOutcome`, `apply_graph_command`, `apply_graph_commands` and `GraphRx`; rewrite `command.rs`'s 23 test sites as direct method tests, each keeping its assertion.
- [x] 3.4 Delete `FieldValue`, `boxed_for`, `int_as` and `write_field` from `sway-graph`.
- [x] 3.5 Remove `Graph::selection`/`set_selection` and their tests.
- [x] 3.6 `GraphPlugin` reduces to inserting the `Graph` resource and scheduling `tick_graph` in `GraphTickSet`.
- [x] 3.7 Drop `crossbeam-channel` from `sway-graph`'s manifest.
- [x] 3.8 `cargo test -p sway-graph` — the engine is green before any consumer is touched.

## 4. Consumers move onto the mutation API

- [x] 4.1 `sway-editor`: add an `EditorEdit` enum (create, delete, set field, connect, disconnect, set slot) and a `GraphEditPlugin` owning the receiver and the `PreUpdate` applier that maps each variant onto a `Graph` method.
- [x] 4.2 `sway-editor`: add the selection resource; move the drop-on-delete behaviour (a deleted node clears the selection) there, driven off `Graph::removed()`.
- [x] 4.3 `sway-editor`: convert the 37 `GraphCommand::` sites (canvas 19, inspector 12, scene_tree 3, palette 2, reflect_ui 1) to `EditorEdit`.
- [x] 4.4 `sway-editor`: move the coercion logic into the inspector — control value → field's declared type, with saturating integer narrowing preserved — and add tests for out-of-range clamping and for a value arriving already typed.
- [x] 4.5 `sway-runtime`'s gizmo and picker: drop the command construction entirely and call `graph.set_field(..)` / the other methods on the `ResMut<Graph>` they already hold (5 sites).
- [x] 4.6 `sway-app`: add `GraphEditPlugin` in editor builds where it inserted `GraphRx`.
- [x] 4.7 `cargo test -p sway-editor -p sway-runtime` and `cargo check --workspace`.

## 5. `sway-graph`: surface reduction

- [x] 5.1 Collapse `Target` into `Compat` plus a computed index; keep `Compat` as the edge's connect-time verdict and make the merged form crate-private.
- [x] 5.2 Delete `NodeParts`/`PartType` type data; add `part_type(registry, type_id, part) -> Option<&'static TypeInfo>` and move the D3 shape check into `register_node_kind` as an explicit assertion.
- [x] 5.3 Update `sway-document`, `sway-editor` and `sway-runtime` at every `NodeParts` call site.
- [x] 5.4 Make `EvalOrder`, `GraphStep`, `PropagateStep`, `Link`, `Sorted`, `topological_order`, `absolute_path`, `compatibility_of_values`, `tick::run` and the `path`/`order` modules crate-private; trim `lib.rs` and `graph/mod.rs` re-exports to what callers use.
- [x] 5.5 Drop `bevy_transform` from `sway-graph`; move `bevy_time` to an optional dependency.
- [x] 5.6 Add a non-default `test-support` feature exporting `trace_world`, `tick_once`, `read_field`, `set_field` and the fixture node kinds from `graph::testing`.
- [x] 5.7 Move `ViewportInputRx` from `sway-graph` to where it is drained (group 6), and delete `sway-graph`'s temporary `sway-viewport-input` re-export.
- [x] 5.8 `cargo test -p sway-graph --all-features` and `cargo check --workspace`.

## 6. `sway-base-nodes` (renamed from `sway-nodes`)

- [x] 6.1 Rename the crate directory, package and workspace entry to `sway-base-nodes`; update every dependent manifest and `use`.
- [x] 6.2 Fold `math.rs` into the `math` node module and `envelope.rs` into the `envelope` node module; delete `beat.rs`, `NoteField` and the crate-root `pub use *` re-exports.
- [x] 6.3 Rename the `Vec3` node kind to `MakeVec3` (`Vec3In`/`Vec3Out` → `MakeVec3In`/`MakeVec3Out`, `value.rs` → `make_vec3.rs`) and drop the now-unnecessary `use bevy::math::Vec3 as MathVec3` alias. Behaviour is unchanged.
- [x] 6.4 Reshape `Envelope`: `inlets.time: f32`, gate timestamps against that base, `state.now` removed, `&World` unused. Update its tests.
- [x] 6.5 Delete `tests/traces.rs`; check the arithmetic it covered is covered by the node kinds' own tests and add what is missing.
- [x] 6.6 Replace the module's `GraphNodesPlugin` with a crate-root `BaseNodesPlugin` registering `Oscillator`, `Envelope`, `Math`, `Remap`, `MakeVec3` and their part types; keep the unique-short-name test.
- [x] 6.7 Delete the crate's copied test harness and depend on `sway-graph`'s `test-support` feature instead.
- [x] 6.8 Drop `bevy_time` from the manifest; confirm the crate no longer reads anything outside the graph.
- [x] 6.9 `cargo test -p sway-base-nodes`.

## 7. One plugin per domain

- [x] 7.1 `sway-midi`: fold `MidiGraphNodesPlugin` into `MidiPlugin`; delete the separate export.
- [x] 7.2 `sway-runtime`: fold `RuntimeNodesPlugin` and `ProjectionPlugin` into one `RuntimePlugin`, keeping `register_runtime_node_kinds` public for schema-only tests.
- [x] 7.3 `sway-app`: collapse the plugin list accordingly.
- [x] 7.4 `cargo test -p sway-midi -p sway-runtime`.

## 8. `sway-editor-viewport` (extracted from `sway-runtime`)

- [x] 8.1 Create `crates/sway-editor-viewport` (deps: `bevy`, `sway-graph`, `sway-viewport-input`, `sway-runtime`, `crossbeam-channel`) and register it in the workspace.
- [x] 8.2 Move `sway-runtime/src/viewport/{mod,camera,gizmo,pick}.rs` into it, along with `ViewportInputRx` and `EditorViewportPlugin`.
- [x] 8.3 Point the gizmo's field writes at the new reflected `SetField`.
- [x] 8.4 `sway-app`: depend on the new crate and add `EditorViewportPlugin` from it in editor builds.
- [x] 8.5 `cargo test -p sway-editor-viewport -p sway-runtime`.

## 9. `sway-runtime` internal layout and dead code

- [x] 9.1 Fold `sprite_material.rs` and `nodes/sprite_material.rs` into one module; remove `ensure_sprite_material_pipeline`'s double-add guard if the collapse makes it unnecessary.
- [x] 9.2 Fold `frame_sequence.rs` and `nodes/frame_sequence.rs` into one module.
- [x] 9.3 Delete `point_cloud.rs`, `scatter.rs`, `sprite_layer.rs` and their exports; note the removal in the roadmap section of `docs/architecture.md` §10.
- [x] 9.4 Delete the crate's copied test harness in favour of `sway-graph`'s `test-support` feature.
- [x] 9.5 `cargo test -p sway-runtime`.

## 10. Manifest hygiene

- [x] 10.1 Remove unused dependency edges: `sway-midi → sway-nodes`, `sway-geo → sway-graph`, `sway-runtime → sway-base-nodes`, `sway-editor → sway-base-nodes` (dev).
- [x] 10.2 Delete the stale manifest comments that justified them.
- [x] 10.3 Re-check every crate's manifest against its `use` statements and remove anything else unreferenced.
- [x] 10.4 `cargo check --workspace --all-targets`.

## 11. Demo asset

- [x] 11.1 Rewrite `crates/sway-app/assets/demo.sway.ron`: `version: 4`, `pos: (x, y)` → `metadata: {"pos": "x,y"}` on all 25 nodes, and `type: "Vec3"` → `type: "MakeVec3"` on `vec3A`/`vec3B`.
- [x] 11.2 Update the file's header comment where it describes the format or names node kinds.
- [x] 11.3 Load it through `load_from_path` in a test and assert the load is diagnostic-clean.

## 12. Documentation

- [x] 12.1 Add `docs/architecture.md` §11 "Source structure": the engine rule, the node-domain rule, the one-plugin rule, the dependency direction, and the shared-vocabulary crate rule.
- [x] 12.2 Update §8's crate layout table for `sway-base-nodes`, `sway-viewport-input`, `sway-editor-viewport` and the removed pipelines.
- [x] 12.3 Update §5's ownership table: selection moves to the editor, viewport input moves out of `sway-graph`.
- [x] 12.4 Update §10: the `Vec3` value node keeps its role under the name `MakeVec3`, and base nodes take time as an inlet.
- [x] 12.5 Replace every `GraphCommand` reference with the mutation API: §1 ("the authoring surface is `GraphCommand`"), §2 ("everything outside the graph writes it through `GraphCommand`" and the command list), §5's ownership table, §7's "they are commands applied in `PreUpdate`", and §10's settled-decisions entry.
- [x] 12.6 Update §2's node table and §7's pipeline listing where they mention `pos` or `Select`, and record the format-4 break where §7 describes the document.

## 13. End-to-end verification

- [ ] 13.1 `cargo test --workspace --all-targets`.
- [ ] 13.2 Run the app against `crates/sway-app/assets/demo.sway.ron` in editor mode: the graph loads with no diagnostics, nodes are where they were saved, the scene renders, and MIDI-driven motion still runs.
- [ ] 13.3 Save from the editor and reload; confirm the file round-trips and node placement is preserved.
- [ ] 13.4 Confirm the graph golden traces produce the same numbers as before the change.
