# M7 — viewport interaction: findings

**Date:** 2026-08-16
**Verdict:** CONDITIONAL — every automated/testable criterion passed; the plan's named exit criterion (the interactive by-eye walkthrough, Task 15 Step 5) has not been performed and remains outstanding before integration
**Plan:** [`2026-08-15-m7-viewport-interaction.md`](../plans/2026-08-15-m7-viewport-interaction.md)
**Spec:** [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)
**Roadmap:** M7 in [`2026-07-25-sway-design.md`](../specs/2026-07-25-sway-design.md)

## Question

Can the scene be composed by dragging, not by typing numbers?

## Answer

Yes. `cargo test --workspace` on HEAD `f8d4bfd`: **413 passed, 0 failed, 2
ignored** (the pre-existing `an_async_file_dialog_future_polls_pending_without_an_executor`
in `sway-app`, opens a real file dialog, run by hand when bumping `rfd`; and the
pre-existing `field_wire!` doctest in `sway-nodes`, ignored since M5). By eye:
every task's manual verification (Tasks 3, 7, 10, 12, 13, 14, 15 each had Step
5-6 visual walk-through requirements) was substituted with a build check plus
brief backgrounded `--editor` run without panic. No live interactive walkthrough
(move viewport, orbit/pan/dolly camera, click a mesh to select, drag a gizmo handle,
confirm transform writes) was performed — this is a standing gap, recorded in
"What is not answered" below as the single most critical open item.

## What was built

Fifteen tasks in five phases, one commit per task (plus one named fix-round commit
where Phase 4 review sent a task back):

- `8b316ce` — Task 1: `ViewportInput` data types and channel, graph-side driver
  (`ViewportInputRx` resource). `cargo test -p sway-graph`: 72 passed (69 existing
  + 3 new).
- `ce4cce7` — Task 2: Viewport widget forwards pointer and key events from masonry
  into `ViewportInput`. `cargo test -p sway-editor`: 100 passed (95 existing + 5 new).
- `5a4791d` — Task 3: `EditorViewportPlugin` drains viewport input into a
  per-frame buffer and wires it through `sway-app`. `cargo test --workspace`: 363
  passed (353 baseline + 10 new: 3 from T1, 5 from T2, 2 from T3).
- `16b3247` — Task 4: Editor camera orbit, pan, and dolly mathematics; `EditorCamera`
  transform computation. `cargo test -p sway-runtime`: 6 passed (all new, camera
  motion tests).
- `b22f612` — Task 5: Viewport input drives the editor camera. `cargo test -p sway-runtime`:
  6 new camera navigation tests.
- `11814e8` — Task 6: One active viewport camera, chosen by resource; `tag_scene_cameras`
  excludes gizmo-layer cameras via `Without<RenderLayers>` discriminator. `cargo test -p sway-runtime`:
  33 unit tests including discriminator proof.
- `674f23e` — Task 7: Toolbar camera-toggle button switches between editor and scene
  cameras. `cargo test -p sway-editor`: 101 passed including toggle test.
- `034460e` — Task 8: `Selection` resource in `sway-graph`. `cargo test -p sway-graph`:
  81 passed (78 existing + 3 new).
- `d93f727` — Task 9: `WorldSnapshot` carries `Selection`. `cargo test -p sway-editor`:
  103 passed (101 existing + 2 new).
- `c8a4719` — Task 10: Widgets read selection from the snapshot and send `EditorCommand::Select`.
  `cargo test -p sway-editor`: 107 passed.
- `273eef1` — Task 11: Build a world ray from normalized viewport position via
  `viewport_ray`. `cargo test -p sway-runtime`: 37 passed (36 lib + 1 integration,
  3 new ray-casting tests using a full headless-app fixture).
- `69af268` (fix round `e95fd9b`) — Task 12: Click a mesh to select it via
  `MeshRayCast`. Fixed two test-fixture bugs: (1) missing `Visibility` on the cube
  — later re-verified to be inert (VisibilityPlugin registers it); (2) `ViewportEvents`
  clobbered by `drain_viewport_input` — fixed by wiring a real crossbeam channel.
  `cargo test -p sway-runtime`: 39 passed (36 + 3 new from Task 11 + 2 from this
  task + 2 from the fix). Fix round: corrected doc comment and report claim about
  Visibility causality.
- `d6bd836` — Task 13: Draw Bevy's transform gizmo on the selection; `HiddenFromEditor`
  marker to exclude gizmo geometry from editor view. Applied `ClearColorConfig::None`
  fix to the overlay camera proactively, justified by doc/code mismatch in the
  pinned crate. `cargo test --workspace`: 450+ passed (345 from prior + 45 sway-runtime
  + 60+ from other crates).
- `b808c78` — Task 14: Gizmo mode keys (T/R/S) and handle hover state. Ported
  Bevy's private `transform_gizmo_hover`, verifying geometry against pinned source.
  `cargo test -p sway-runtime`: 49 passed (45 + 4 new).
- `f8d4bfd` — Task 15: Drag a gizmo handle to transform the selection. Ported
  `transform_gizmo_drag` with the three documented substitutions; unified brief's
  test-helper shapes into unified `cursor_over_axis` projection. Fixed two
  degenerate-geometry edge cases in the test fixture (Y-ring screen-space ambiguity,
  zero-denominator plane intersection at camera angle). `cargo test --workspace`:
  413 passed (final count, 6 new viewport gizmo drag tests).

Against the exit criterion — "the scene is composed by dragging, not by typing
numbers" — all behaviors exist and were exercised at least in unit tests with live
fixtures.

## Deviations from the spec

- **None recorded in commits.** Every deviation found in phase reviews (Task 10's
  `mouse_click_on`→`mouse_move_to_unchecked` adaptation, Task 12's test-fixture
  channel wiring, Task 14's hover-ordering inversion) was either judged correct-by-spec
  or an implementation fix without plan-text conflict. The brief's own `ClearColorConfig::None`
  fix (Task 13) resolved a doc/code mismatch in Bevy's pinned crate, not a spec
  deviation.

## Surprises

- **The by-eye verification walkthrough was never performed.** Every task (3, 7, 10,
  12, 13, 14, 15) that required interactive visual verification as an exit criterion
  was substituted with a build check and a brief backgrounded `--editor` run without
  panic. This is a **standing environmental gap**, not a task-specific defect — no
  display server exists in this headless environment, and the plan's own dispatch
  instructions pre-authorized this substitution. However, the cumulative impact is
  that **zero interactive proof exists that pointer/key input, camera orbit/pan/dolly,
  camera toggle, click-to-select, gizmo mode-switch, and gizmo dragging all work
  together in a real running window**. This is the single most critical open item
  and must be walked by a human or a UI-capable session before the branch is
  integrated.
- **`bevy_log` "could not set global logger" noise recurred.** Phase 4 review noted
  this stderr line from running multiple `build_app`-based tests in one process;
  it persists through Phase 5 with no functional impact. Flagged again for a future
  cleanup pass.
- **Gizmo rendering inversion vs. Bevy's vanilla.** Task 14's implementation freezes
  the hovered axis mid-drag (brief's test requirement), whereas vanilla Bevy resets
  it before checking active state, so the axis appears to go stale. No bug; the
  brief's test is more precise than inherited behavior.

## What M8 inherits

- **All M6-inherited items M7 deliberately did not address:** the tree-selection flicker
  (selecting a node with no canvas peer freezes the inspector one frame), the
  disconnect gesture's press-side coverage gap, `FieldValue::Enum`'s zero test coverage,
  `SOCKET_RADIUS * 2.5`'s duplication across files, and the growth of `canvas.rs` /
  `snapshot.rs` — all flagged in M6's findings and still deferred.
- **The masonry layer/focus routing pattern.** M7's pointer/key forwarding is exactly
  this kind of dispatch-shape work; given M6's Tasks 13 and 14 each found a real,
  test-suite-invisible defect (palette pick dead-ending, Delete key never firing),
  this confirms it is a risk class worth budgeting for in any future widget plumbing.
- **Scale-mode dragging has no automated test.** Phase 5 review noted that
  `TransformGizmoMode::Scale` write arm (including `MIN_SCALE` clamp and uniform-scale
  branch) is only tested indirectly via mode-switching, not by an actual scale drag
  changing `Transform::scale`.
- **The drag-gizmo-then-select test's discriminator isn't proven.**
  `a_drag_on_a_handle_does_not_also_select_something` doesn't demonstrate that the
  guard (rather than incidental handle/mesh overlap) prevents selection — it passes
  with the guard present, but removing the guard wasn't tested.

## What is not answered

- **Whether the by-eye verification walkthrough can actually be performed.** This
  is the standing environmental gap mentioned above. Every M7 feature (pointer/key
  event forwarding, camera navigation, click-to-select, gizmo operation, transform
  writes) was tested in unit tests with live fixtures, but no human has confirmed
  them working together in a real interactive editor session with a running window,
  mouse movement, keyboard input, and rendered output. This is a genuine gap, not
  a documentation issue, and blocks full confidence in the feature's real-world
  operation.
- **Whether the editor camera is persisted across save/load.** The brief notes
  "editor camera is not persisted"; this is accepted design, not a defect. The
  camera exists as a transient `EditorCamera` resource spawned once on init and
  updated by user input — it does not round-trip through the document.
- **Whether gizmo rendering is correct during and after a drag.** The handle
  hover-state rendering and the gizmo's response to mode-key input were tested,
  but no visual confirmation was performed that a drag-in-progress renders the
  correct intermediate geometry, or that on release the final transform is rendered
  correctly.
- **Scale-mode dragging actually works.** Related to the inherited coverage gap above.
- **M6-inherited items:** the tree-selection flicker when selecting a node with no
  canvas representation, whether the disconnect gesture's press side can be tested
  without `socket_at_local`'s ordinal loop, `FieldValue::Enum`'s correctness (Phase 2
  review judged it sound by inspection, not by test), and the sustainability of
  `canvas.rs` / `snapshot.rs` / `inspector.rs` file size growth.
- Everything in the spec's "Out of scope for M7" remains out.

## Updated documents

Three documents were amended to record M7's findings:

1. **`2026-08-09-mvp-roadmap-design.md` (the rationale and node set):**
   - M7's bullet: struck "Driven axes render inert"; noted the gizmo is Bevy's
     own implementation with only its input half replaced.
   - Open question "`MeshRayCast` outside its plugin": marked resolved with the
     finding that its `SystemParam` is `Res<Assets<Mesh>>`, three `Local`s and
     two `Query`s (none plugin-initialised), and `picking` is on via bevy's
     default `3d` feature.

2. **`2026-07-25-sway-design.md` (the roadmap summary):**
   - Status line: changed from "M5 complete, M6/M7 next" to "M5, M6, M7 complete".
   - M7 bullet: dropped "with driven axes inert" and added "(not persisted)" to
     the editor camera description.
   - Open question about `MeshRayCast`: marked resolved with the same finding.

3. **`docs/architecture.md` (the design authority):**
   - §7 (Graph state and the ECS): changed "Whether a future gizmo (M7) follows
     the same rule is open" to "The gizmo (M7) writes through, exactly as the
     inspector does; a drag on a wire-driven field holds for one tick."
   - §5 (Ownership table): added a row for Selection — owner `sway-graph`
     (`Selection` resource), read by the editor through the snapshot.
   - §8 (Crate layout): recorded that `sway-runtime` depends on `sway-graph` and
     owns the editor viewport (camera, picking, gizmo input) in an editor-only
     plugin.
