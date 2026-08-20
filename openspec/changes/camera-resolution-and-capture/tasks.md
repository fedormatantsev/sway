## 1. Groundwork (`sway-document`)

Do this first: it is the one thing that can invalidate the whole approach (design — Risks, first bullet).

- [ ] 1.1 Add a test that loads a serialized `Camera` written before this change — no `resolution` field — and expect it to load with the default rather than error. `cargo test -p sway-document`
- [ ] 1.2 If 1.1 fails, make the v4 load path fill a missing field from `Default` rather than rejecting the document, and keep the test as the regression. Do not work around it by hand-editing project files.

## 2. `sway-gpu`

- [ ] 2.1 Add a camera render target beside `ViewportTexture` in `textures.rs`: colour only, `COPY_SRC`, both the sRGB view Bevy writes through and the sample view the compositor reads — the same pair `ViewportTexture` already carries, sized from an arbitrary resolution rather than from a window.
- [ ] 2.2 Make `WindowSurface::new` choose its present mode instead of hardcoding `PresentMode::Fifo` (`surface.rs:60`): take a "don't wait for the refresh" preference, pick `Mailbox` → `Immediate` → `Fifo` from `SurfaceCapabilities::present_modes`, and return which one it got so the caller can report a fallback (D5a).
- [ ] 2.3 Add an asynchronous readback: a small buffer pool, `copy_texture_to_buffer` + `map_async` on request, and a non-blocking `device.poll` collector that yields completed readbacks as unpadded RGBA8 rows. No blocking poll anywhere on this path (D6).
- [ ] 2.4 Test the row unpadding with a deliberately unaligned width (e.g. 1000 px = 4000 bytes/row, which pads to 4096) — 1920 is 256-aligned already and hides the bug. `cargo test -p sway-gpu`
- [ ] 2.5 Test that the pool drops rather than blocks when every buffer is in flight, and reports the drop to its caller.

## 3. `sway-runtime` — node kinds

- [ ] 3.1 Add a `CameraTarget` marker and its outlet part to `nodes/protocol.rs`, following the existing material/mesh/sequence pattern, plus the port-name constants. Extend `every_protocol_marker_is_valueless` to cover it (D2).
- [ ] 3.2 Give `Camera` a `resolution` inlet defaulting to 1920×1080 (D9) and a `CameraTarget` outlet. Test the default and that pose inlets are unaffected.
- [ ] 3.3 Add the `Output` node: one `camera` inlet, non-variadic, no pose, no children, no `SceneNodeOut`. Mirror `a_group_declares_no_geometry_and_no_material_port` to pin the absent ports, so a mesh or material connection is refused by schema.
- [ ] 3.4 Add the `Capture` node: `camera`, `path` and `recording` inlets, `recording` defaulting to false. Same absent-port test as 3.3.
- [ ] 3.5 Add the path-pattern helper: a `#` run expands to the zero-padded slot index; a pattern with no `#` run is refused. Unit-test expansion, padding width, and the refusal.
- [ ] 3.6 Register all three kinds in the runtime plugin so the palette, document and inspector pick them up reflectively with no editor-side change. `cargo test -p sway-runtime`

## 4. `sway-runtime` — targets and projection

- [ ] 4.1 Replace `retarget_cameras`' single `VIEWPORT_HANDLE` with a handle per camera: allocate a target sized to the camera's `resolution`, register it in `ManualTextureViews`, and point that camera at it. `VIEWPORT_HANDLE` stays as the editor camera's handle only (D1).
- [ ] 4.2 Allocate lazily — only for a camera something consumes (output, capture, or the editor previewing it) — and release the target and its handle when the camera is deleted or its resolution changes.
- [ ] 4.3 Make a zero-component resolution, and a resolution the device cannot allocate, render nothing and report once naming the camera (and, for the second, the limit). Test the once-only-ness, not just the diagnostic.
- [ ] 4.4 Publish what the host must present — the target handle and resolution of the camera the `Output` node names, or nothing — as a resource, the way `Graph` and `Transport` are already read by the presenter (D3).
- [ ] 4.5 Publish capture intent per capture node: camera target, expanded path pattern, and whether `recording` is true.
- [ ] 4.6 Report once, not per frame, for a capture with no camera or an empty path, and for a document whose output names no camera. The migration depends on that last message being clear (design — Migration Plan). `cargo test -p sway-runtime`

## 5. `sway-editor-viewport`

- [ ] 5.1 Turn `ViewportCamera` from the `Editor | Scene` toggle into a selection over the editor camera plus each camera node, and fall back to the editor camera when the selected one leaves the document (D3).
- [ ] 5.2 Drive `apply_active_camera` from that selection, keeping exactly one camera active per frame; delete `ViewportCameraRole` and `tag_scene_cameras` with it.
- [ ] 5.3 Re-check `pick.rs` and `gizmo.rs` against the removed marker — the gizmo overlay camera was excluded by `tag_scene_cameras`' `Without<RenderLayers>` filter and now needs excluding by identity instead (design — Risks).
- [ ] 5.4 Add the letterbox fit as a pure function: the largest rect of a given aspect that fits a pane, centred, rounded to whole pixels. Unit-test the exact case (640×480 pane, 16:9 → 640×360 centred), the rounding case (641-px pane), and a pane narrower than the aspect.
- [ ] 5.5 Size a previewed camera's target to that fitted rect when the editor is its only consumer, and to the authored resolution when a graph consumer needs those pixels, with the preview sampling the larger target down (D4). Test both sizings; test that the camera renders once either way. `cargo test -p sway-editor-viewport`

## 6. `sway-editor`

- [ ] 6.1 Replace `ViewRequest::ToggleCamera` and its transport-bar control with a camera list — the editor camera plus every camera node — since a two-state toggle can no longer express the selection. `cargo test -p sway-editor`

## 7. `sway-app`

- [ ] 7.1 Add `--no-vsync` and `--capture-window <path>` to `parse_args`, alongside the existing `--midi` / `--list` / `--editor`; pass the vsync preference through to `WindowSurface::new` and report a fallback (2.2).
- [ ] 7.2 Pace `Shell::redraw` to a fixed 60 fps against a wall-clock deadline, unconditionally, with no rendering-ahead to make up a late frame (D5a).
- [ ] 7.3 Composite the presented camera letterboxed into the window using the fit function from 5.4, and composite no viewport quad at all when nothing is wired to the output.
- [ ] 7.4 Add the capture drain after `app.update()` in `Shell::redraw`: advance the slot clock, issue one readback per crossed slot, reissue the last frame for skipped slots, drop and count on saturation, and report the count when a run ends (D5, D6).
- [ ] 7.5 Add the writer thread: a bounded channel of (slot index, pixels), PNG encoding via the `image` crate, temp file then rename. Add `image` to `sway-app` only (D7).
- [ ] 7.6 Implement `--capture-window`: render until assets have resolved, the graph has projected once, and two consecutive window readbacks are byte-identical; then write and exit successfully. A bounded frame cap ends the wait with a diagnostic and a failure exit, never a file (D8).
- [ ] 7.7 Make the exit status alone distinguish success from failure, and leave no partial file behind on either path. `cargo test -p sway-app`

## 8. Verification

- [ ] 8.1 Run the focused suites for every crate touched: `sway-document`, `sway-gpu`, `sway-runtime`, `sway-editor-viewport`, `sway-editor`, `sway-app`.
- [ ] 8.2 Add an `Output` node wired to the camera in each project document under the repo, and confirm each still presents (design — Migration Plan).
- [ ] 8.3 Manually run the editor: preview a camera, confirm the letterbox matches its authored aspect and that editing the resolution at constant aspect changes nothing on screen.
- [ ] 8.4 Manually record a run to a temp directory: confirm the file count matches the elapsed seconds times 60 within the reported drop count, that numbering is slot-based, and that a second run restarts at zero.
- [ ] 8.5 Run `--capture-window` end to end into a temp path: confirm one file, the window's own dimensions, a success exit, and a failure exit with no file for an unwritable path.
- [ ] 8.6 `cargo clippy --workspace --all-targets` and `cargo test --workspace` once the focused suites pass.
