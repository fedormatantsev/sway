## Context

See proposal.md — Why.

The demo infrastructure spans three crates (`sway-app`, `sway-runtime`,
`sway-nodes`) plus two test files, one asset file, and the legacy
`docs/superpowers/` planning tree. No demo symbol is referenced outside its
crate's own tests or `sway-app/src/main.rs`.

The `docs/superpowers/` directory references are surfaced in two places in
`docs/architecture.md` (lines 4 and 34). The useful content in those
referenced files (MVP scope, milestones, out-of-scope items) is already
summarised inline in `architecture.md §10` and throughout the doc; the
external references are now dead pointers.

## Goals / Non-Goals

**Goals**
- `cargo build` and `cargo test` pass with zero new warnings after the change.
- No `spawn_demo_*`, `Demo`, `--demo`, or `sprite_depth_spike` symbols remain
  in non-test production code.
- `docs/superpowers/` is deleted; `docs/architecture.md` compiles without
  broken internal links.
- Any still-accurate MVP/roadmap prose currently reachable only via
  `docs/superpowers/` links is preserved in `docs/architecture.md` or a new
  `docs/notes.md`.

**Non-Goals**
- Rewriting `point_cloud.rs`, `scatter.rs`, or `sprite_layer.rs` themselves —
  those modules may contain production renderers (even if the *demo spawner* is
  removed).
- Introducing replacement demo infrastructure.
- Deleting the `cube.gltf` or `sprites/` assets if they are referenced by the
  real document path (they are; keep them).

## Decisions

**D1: Remove entire `sprite_depth_spike` module, not just its public spawner.**

Rationale: `sprite_depth_spike.rs` is self-described as "Throwaway by design".
It exports `SpriteDepthPlugin` and `spawn_depth_spike_scene`, both of which are
referenced only through the `Demo::SpriteDepth` dispatch arm. No production
system or document path uses them. Removing the module simplifies `lib.rs` and
removes the embedded shader asset.

Alternative considered: strip only the public API, keep the module for
reference. Rejected — leaving dead modules creates confusion about production
boundaries.

**D2: Remove `tests/demo_renders.rs` and `tests/demo_document.rs` together.**

Both tests exercise `demo.sway.ron` and demo-spawner functions. Once those are
gone the tests will not compile. Deleting them is the only option short of
rewriting them against new fixtures, which is out of scope here.

`tests/cube_asset.rs` and `tests/rfd_pollable.rs` are unrelated and stay.

**D3: Delete `docs/superpowers/` as a single directory removal.**

All files in it are historical artefacts of the Superpowers planning workflow.
The two references from `architecture.md` point to design documents whose
conclusions are already captured inline. No source file links to any
`docs/superpowers/` path.

**D4: Inline the two broken `architecture.md` references; add a `docs/notes.md`
only if the prose does not fit inline.**

Line 4's sentence ("Ongoing roadmap and open work live in …") should be removed
or replaced with a pointer to OpenSpec changes. Line 34's MVP roadmap reference
should be replaced with a short inline note — `§10` already lists Out-of-MVP
items so the cross-reference adds nothing.

## Risks / Trade-offs

- **Risk: a `Demo`-arm path contains a production feature we haven't noticed.**
  → Mitigation: `grep` for every symbol removed before deletion; confirm no
  non-demo caller exists (`grep` evidence already collected in the proposal).

- **Risk: `sprite_depth_spike` holds a shader asset that gets referenced by
  some test golden file.**
  → Mitigation: the only test that references it is
  `sway-runtime/tests/sprite_depth_interpenetration.rs`. Check that test after
  deletion; delete it alongside the module if it no longer compiles.

## Migration Plan

1. Delete `docs/superpowers/` (git rm -r).
2. Update `docs/architecture.md` — remove/replace the two broken references.
3. Remove `Demo` enum + `--demo` CLI handling from `sway-app/src/main.rs`;
   make the normal document-load path unconditional.
4. Remove `spawn_demo_*` functions from `sway-runtime`; remove
   `sprite_depth_spike` module entirely.
5. Delete `sway-app/tests/demo_renders.rs`, `sway-app/tests/demo_document.rs`,
   and `sway-app/assets/demo.sway.ron`.
6. Remove `pub use sprite_depth_spike::SpriteDepthPlugin` and dead module
   decl from `sway-runtime/src/lib.rs`; update the crate doc comment.
7. Remove dead `pub use` re-exports for demo spawners from `sway-runtime/src/lib.rs`.
8. Clean up stale M1/demo-era comments in `headless.rs`, `node_box.rs`, and
   `sway-nodes` where `grep` flagged them.
9. Run `cargo build --workspace` and `cargo test --workspace`; fix any
   compilation errors.
10. No rollback complexity — all changes are deletions; a revert is a `git
    revert`.

## Open Questions

None.
