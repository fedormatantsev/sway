## 1. Delete legacy planning directory

- [x] 1.1 Run `git rm -r docs/superpowers/` to remove all plans, specs, and reports

## 2. Update docs/architecture.md

- [x] 2.1 Remove the sentence on line 4 that references `docs/superpowers/specs/2026-07-25-sway-design.md`; replace the intro paragraph with a pointer to OpenSpec changes for ongoing work
- [x] 2.2 Remove the `docs/superpowers/specs/2026-08-09-mvp-roadmap-design.md` reference on line 34; the MVP scope is already captured inline in §10, so replace or drop the cross-reference

## 3. Remove demo CLI and dispatch from sway-app

- [x] 3.1 Delete the `Demo` enum from `sway-app/src/main.rs`
- [x] 3.2 Remove the `demo: Option<Demo>` field from `Args` and the `"--demo"` branch from `parse_args`
- [x] 3.3 Remove `let demo = args.demo;` and the camera-collision comment block above `match demo`
- [x] 3.4 Replace `match demo { None => { … }, Some(…) => { … } }` with the document-load plugin block unconditionally (the current `None` arm content)
- [x] 3.5 Remove `args.windowed` and `args.monitor` fields and their `parse_args` branches if they are now fully unused (currently flagged `#[allow(dead_code)]`)

## 4. Remove demo test files and assets from sway-app

- [x] 4.1 Delete `crates/sway-app/tests/demo_renders.rs`
- [x] 4.2 Delete `crates/sway-app/tests/demo_document.rs`
- [x] 4.3 Delete `crates/sway-app/assets/demo.sway.ron`; create `assets/project.sway.ron` (empty v3 document) as the new startup default

## 5. Remove demo spawners from sway-runtime

- [x] 5.1 Delete `pub fn spawn_demo_point_cloud` and any dead helpers from `crates/sway-runtime/src/point_cloud.rs`
- [x] 5.2 Delete `pub fn spawn_demo_sprite_layers` and `pub fn spawn_demo_camera` from `crates/sway-runtime/src/sprite_layer.rs`; remove dead `use` imports
- [x] 5.3 Delete `pub fn spawn_demo_scatter` and any dead helpers from `crates/sway-runtime/src/scatter.rs`
- [x] 5.4 Delete `crates/sway-runtime/src/sprite_depth_spike.rs` entirely (module + embedded shader assets)
- [x] 5.5 Delete `crates/sway-runtime/tests/sprite_depth_interpenetration.rs` (it imports from `sprite_depth_spike` which no longer exists)

## 6. Update sway-runtime public surface

- [x] 6.1 Remove `pub mod sprite_depth_spike;` from `crates/sway-runtime/src/lib.rs`
- [x] 6.2 Remove `pub use sprite_depth_spike::SpriteDepthPlugin;` from `lib.rs`
- [x] 6.3 Update the crate-level doc comment in `lib.rs` (currently reads "Provisional render spike code for M1…") to reflect current scope
- [x] 6.4 Remove any remaining `pub use point_cloud::spawn_demo_point_cloud`, `pub use sprite_layer::spawn_demo_*`, `pub use scatter::spawn_demo_scatter` re-exports if present in `lib.rs`

## 7. Clean up stale comments in source

- [x] 7.1 Remove "The M1 demo files (and …)" comment in `crates/sway-runtime/src/headless.rs` or update it to refer to the current code path
- [x] 7.2 Remove the "demo document" / `--editor` demo reference in `crates/sway-editor/src/node_box.rs` line 327 or update the doc comment to reflect current reality
- [x] 7.3 Check `crates/sway-nodes/src/nodes/value.rs` and `osc.rs` for comments referencing the demo document and update them to refer to the actual document format, not the deleted `demo.sway.ron`

## 8. Verify the build

- [x] 8.1 Run `cargo build --workspace` — fix any compilation errors
- [x] 8.2 Run `cargo test --workspace` — fix any test failures
- [x] 8.3 Run `cargo clippy --workspace` — resolve any new warnings introduced by the deletions
