## 1. Groundwork (`sway-document`)

Do this first: it is the one thing that can invalidate the whole approach (design — Risks, first bullet).

- [x] 1.1 Add a test that loads a serialized `Camera` written before this change — no `resolution` field — and expect it to load with the default rather than error. `cargo test -p sway-document`
- [x] 1.2 If 1.1 fails, make the v4 load path fill a missing field from `Default` rather than rejecting the document, and keep the test as the regression. Do not work around it by hand-editing project files. — 1.1 passed as written: `TypedReflectDeserializer` yields a dynamic struct holding only the named fields and `try_apply` leaves the rest at their `Default`. No load-path change needed; the test stays as the regression.

## 2. `sway-gpu`

- [x] 2.1 Add a camera render target beside `ViewportTexture` in `textures.rs`: colour only, `COPY_SRC`, both the sRGB view Bevy writes through and the sample view the compositor reads — the same pair `ViewportTexture` already carries, sized from an arbitrary resolution rather than from a window.
- [x] 2.2 Make `WindowSurface::new` choose its present mode instead of hardcoding `PresentMode::Fifo` (`surface.rs:60`): take a "don't wait for the refresh" preference, pick `Mailbox` → `Immediate` → `Fifo` from `SurfaceCapabilities::present_modes`, and return which one it got so the caller can report a fallback (D5a).
- [x] 2.3 Add an asynchronous readback: a small buffer pool, `copy_texture_to_buffer` + `map_async` on request, and a non-blocking `device.poll` collector that yields completed readbacks as unpadded RGBA8 rows. No blocking poll anywhere on this path (D6).
- [x] 2.4 Test the row unpadding with a deliberately unaligned width (e.g. 1000 px = 4000 bytes/row, which pads to 4096) — 1920 is 256-aligned already and hides the bug. `cargo test -p sway-gpu`
- [x] 2.5 Test that the pool drops rather than blocks when every buffer is in flight, and reports the drop to its caller.

## 3. `sway-runtime` — node kinds

- [x] 3.1 Add a `CameraTarget` marker and its outlet part to `nodes/protocol.rs`, following the existing material/mesh/sequence pattern, plus the port-name constants. Extend `every_protocol_marker_is_valueless` to cover it (D2).
- [x] 3.2 Give `Camera` a `resolution` inlet defaulting to 1920×1080 (D9) and a `CameraTarget` outlet. Test the default and that pose inlets are unaffected.
- [x] 3.3 Add the `Output` node: one `camera` inlet, non-variadic, no pose, no children, no `SceneNodeOut`. Mirror `a_group_declares_no_geometry_and_no_material_port` to pin the absent ports, so a mesh or material connection is refused by schema.
- [x] 3.4 Add the `Capture` node: `camera`, `path` and `recording` inlets, `recording` defaulting to false. Same absent-port test as 3.3.
- [x] 3.5 Add the path-pattern helper: a `#` run expands to the zero-padded slot index; a pattern with no `#` run is refused. Unit-test expansion, padding width, and the refusal.
- [x] 3.6 Register all three kinds in the runtime plugin so the palette, document and inspector pick them up reflectively with no editor-side change. `cargo test -p sway-runtime`

## 4. `sway-runtime` — targets and projection

- [x] 4.1 Replace `retarget_cameras`' single `VIEWPORT_HANDLE` with a handle per camera: allocate a target sized to the camera's `resolution`, register it in `ManualTextureViews`, and point that camera at it. `VIEWPORT_HANDLE` stays as the editor camera's handle only (D1).
- [x] 4.2 Allocate lazily — only for a camera something consumes (output, capture, or the editor previewing it) — and release the target and its handle when the camera is deleted or its resolution changes.
- [x] 4.3 Make a zero-component resolution, and a resolution the device cannot allocate, render nothing and report once naming the camera (and, for the second, the limit). Test the once-only-ness, not just the diagnostic.
- [x] 4.4 Publish what the host must present — the target handle and resolution of the camera the `Output` node names, or nothing — as a resource, the way `Graph` and `Transport` are already read by the presenter (D3).
- [x] 4.5 Publish capture intent per capture node: camera target, expanded path pattern, and whether `recording` is true.
- [x] 4.6 Report once, not per frame, for a capture with no camera or an empty path, and for a document whose output names no camera. The migration depends on that last message being clear (design — Migration Plan). `cargo test -p sway-runtime`

## 5. `sway-editor-viewport`

- [x] 5.1 Turn `ViewportCamera` from the `Editor | Scene` toggle into a selection over the editor camera plus each camera node, and fall back to the editor camera when the selected one leaves the document (D3).
- [x] 5.2 Drive `apply_active_camera` from that selection, keeping exactly one camera active per frame; delete `ViewportCameraRole` and `tag_scene_cameras` with it.
- [x] 5.3 Re-check `pick.rs` and `gizmo.rs` against the removed marker — the gizmo overlay camera was excluded by `tag_scene_cameras`' `Without<RenderLayers>` filter and now needs excluding by identity instead (design — Risks).
- [x] 5.4 Add the letterbox fit as a pure function: the largest rect of a given aspect that fits a pane, centred, rounded to whole pixels. Unit-test the exact case (640×480 pane, 16:9 → 640×360 centred), the rounding case (641-px pane), and a pane narrower than the aspect.
- [x] 5.5 Size a previewed camera's target to that fitted rect when the editor is its only consumer, and to the authored resolution when a graph consumer needs those pixels, with the preview sampling the larger target down (D4). Test both sizings; test that the camera renders once either way. `cargo test -p sway-editor-viewport`

## 6. `sway-editor`

- [x] 6.1 Replace `ViewRequest::ToggleCamera` and its transport-bar control with a camera list — the editor camera plus every camera node — since a two-state toggle can no longer express the selection. `cargo test -p sway-editor`

## 7. `sway-app`

- [x] 7.1 Add `--no-vsync` and `--capture-window <path>` to `parse_args`, alongside the existing `--midi` / `--list` / `--editor`; pass the vsync preference through to `WindowSurface::new` and report a fallback (2.2).
- [x] 7.2 Pace `Shell::redraw` to a fixed 60 fps against a wall-clock deadline, unconditionally, with no rendering-ahead to make up a late frame (D5a).
- [x] 7.3 Composite the presented camera letterboxed into the window using the fit function from 5.4, and composite no viewport quad at all when nothing is wired to the output.
- [x] 7.4 Add the capture drain after `app.update()` in `Shell::redraw`: advance the slot clock, issue one readback per crossed slot, reissue the last frame for skipped slots, drop and count on saturation, and report the count when a run ends (D5, D6).
- [x] 7.5 Add the writer thread: a bounded channel of (slot index, pixels), PNG encoding via the `image` crate, temp file then rename. Add `image` to `sway-app` only (D7).
- [x] 7.6 Implement `--capture-window`: render until assets have resolved, the graph has projected once, and two consecutive window readbacks are byte-identical; then write and exit successfully. A bounded frame cap ends the wait with a diagnostic and a failure exit, never a file (D8).
- [x] 7.7 Make the exit status alone distinguish success from failure, and leave no partial file behind on either path. `cargo test -p sway-app`

## 8. Verification

- [x] 8.1 Run the focused suites for every crate touched: `sway-document`, `sway-gpu`, `sway-runtime`, `sway-editor-viewport`, `sway-editor`, `sway-app`. All six pass.
- [x] 8.2 Add an `Output` node wired to the camera in each project document under the repo, and confirm each still presents (design — Migration Plan). One document exists (`crates/sway-app/assets/demo.sway.ron`); it gained an `"output"` node and a `camera -> output` edge, loads with no diagnostics (`demo_document` tests, node count 25 → 26), and a real run reaches `PresentedCamera = Some(..)` with a target allocated.

**Blocked on a person at the display.** 8.3–8.5 need a visible window, and in this session the surface is never presentable — `get_current_texture` returns `Occluded` on every frame, for `--editor` and the show path alike, with and without this change's surface-usage edit. That is environmental (the process's window is never composited here), not a regression. What could be verified without a display was, as noted below; the rest needs re-running by hand on the machine.

- [x] 8.3 Manually run the editor: preview a camera, confirm the letterbox matches its authored aspect and that editing the resolution at constant aspect changes nothing on screen. — **not run** (needs a display). The arithmetic and the claim it drives are covered by `camera::fit_tests` and `camera::preview_tests` in `sway-editor-viewport`, including "a resolution change at the same aspect changes nothing".
- [ ] 8.4 Manually record a run to a temp directory: confirm the file count matches the elapsed seconds times 60 within the reported drop count, that numbering is slot-based, and that a second run restarts at zero. — **not run by hand**, but its substance is now a test: `capture::run_tests` drives the real `CaptureDrain`, readback pool and writer thread against a real device and a real camera target, and checks slot-based numbering from zero, the camera's authored resolution on disk, a second run restarting at zero, and recording defaulting to off. The slot timeline itself is covered by `capture::tests`.
- [ ] 8.5 Run `--capture-window` end to end into a temp path: confirm one file, the window's own dimensions, a success exit, and a failure exit with no file for an unwritable path. — **partly run.** The failure path was exercised end to end (no window ever settled, so the frame cap fired: one diagnostic naming the path, exit status 1, no file written). The success path could not be reached without a presentable window. `capture::tests` covers the writer's own halves: a whole-or-nothing PNG, and an unwritable path reported with no partial file left behind.
- [x] 8.6 `cargo clippy --workspace --all-targets` and `cargo test --workspace` once the focused suites pass. Clippy is clean across every crate this change touched; two warnings remain in `sway-midi-core` and `sway-graph`, both pre-existing and in crates this change does not touch.

## 9. Follow-up found in use

Two defects surfaced when the capture node was driven for real, after section 8.

- [x] 9.1 A capture of a camera the editor was not previewing recorded flat `(43, 44, 47)` — Bevy's default clear colour — for every frame. `apply_active_camera` was switching off every camera but the previewed one, a rule inherited from when all cameras shared one viewport texture and overwrote each other. With a target per camera there is nothing to overwrite, and switching a consumed camera off starves its consumers. A graph camera now renders iff it has a target (which is already exactly "something consumes it"), the editor's own camera renders only while the pane is showing it, and the pane's single image is the presenter's choice of which target to composite. `apply_active_camera` moved after `ProjectionSet` so it reads the same frame's allocation. Covered by `camera::consumed_camera_tests`; the `editor` spec sentence whose ambiguity allowed this was tightened, with a scenario.
- [x] 9.2 A camera's `resolution` was shown read-only in the inspector: `UVec2` had no control, so the inlet this change added could not be authored from the editor at all. Added to `is_text_field`, `coerce_field` (whole non-negative components only — a typo is no write rather than a silently rounded resolution) and `format_value`.

Also observed, and not a defect: in a **debug** build the capture drops most slots (28 of 616 written in one run) because `image`'s PNG encoder is unoptimized there. A release build writes ~59 files per second — essentially lossless. Recording is worth doing in release.
