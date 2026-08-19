## Why

The codebase carries M1/M8 render-spike demo code that was explicitly labelled
"throwaway by design" and "expect most of this to be rewritten at M5". M5 is
complete; the demo infrastructure is now dead weight — unused structs, CLI
flags, test files, and spawning functions that exist only to drive one-off
visual spikes. The `docs/superpowers/` directory is the legacy planning home;
new work lives under OpenSpec and those files are no longer the system's source
of truth. Architecture references pointing at superseded files confuse new
readers.

## What Changes

- **Remove `Demo` enum and `--demo` CLI flag** from `sway-app/src/main.rs`:
  the enum, the `parse_args` `"--demo"` branch, the `args.demo` field, and
  the entire `match demo { … }` dispatch block. The normal document-load path
  (current `None` arm) becomes unconditional.
- **Remove demo-only public API from `sway-runtime`**: `spawn_demo_point_cloud`,
  `spawn_demo_sprite_layers`, `spawn_demo_camera`, `spawn_demo_scatter`, and
  `spawn_depth_spike_scene`. The modules (`point_cloud`, `sprite_layer`,
  `scatter`, `sprite_depth_spike`) stay if they contain non-demo production
  code; only the `pub fn spawn_demo_*` / `spawn_depth_spike_scene` functions
  and their dead dependencies are removed.
- **Remove `sway-app` demo tests**: `tests/demo_renders.rs` and
  `tests/demo_document.rs` — they test a document (`demo.sway.ron`) and
  rendering paths that no longer exist in the codebase.
- **Remove `assets/demo.sway.ron`** from `sway-app/assets` (verified unused
  once the tests are gone).
- **Delete `docs/superpowers/`** entirely. Plans and reports are historical
  artefacts of the Superpowers-era workflow; specs inside it are superseded by
  OpenSpec.
- **Update `docs/architecture.md`**: remove the two cross-references to
  `docs/superpowers/…` files; extract and preserve any still-accurate roadmap
  or MVP content inline (or in a new `docs/notes.md` if the passage is large
  enough to warrant its own file).
- **Clean up stale comments in source**: `sway-runtime/src/lib.rs` module
  docstring calls the crate "provisional render spike code for M1"; update it
  to reflect current reality. Remove inline references to demo scaffolding in
  `headless.rs`, `node_box.rs`, and wherever `grep` finds "M1 demo" / "demo
  document" comments that no longer apply.

## Capabilities

### New Capabilities

_None_ — this is a pure deletion/cleanup change with no new observable behaviour.

### Modified Capabilities

This change does not alter any spec-level requirements. It removes dead code
paths that were never part of any spec. `skip_specs: true` is appropriate.

## Impact

- `sway-app`: `main.rs` loses the `Demo` enum, `args.demo` field, and
  demo-dispatch match; `tests/demo_renders.rs` and `tests/demo_document.rs` are
  deleted; `assets/demo.sway.ron` is deleted.
- `sway-runtime`: public `spawn_demo_*` symbols removed from `point_cloud.rs`,
  `sprite_layer.rs`, `scatter.rs`; module `sprite_depth_spike` removed or
  stripped of its public surface if any production code exists there (currently
  none).
- `docs/`: `superpowers/` directory deleted; `architecture.md` updated.
- No public API changes to library crates that other consumers depend on (all
  removed symbols are demo-only entrypoints).
- `cargo build` and `cargo test` must pass after the change with no new
  warnings.
