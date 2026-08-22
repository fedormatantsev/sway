## 1. `sway-runtime` — node kinds

- [x] 1.1 Add a camera-only outlet part to `nodes/protocol.rs` (`camera: CameraTarget`, no `SceneChild`). Do not reuse `CameraTargetOut`. Extend `every_protocol_marker_is_valueless` if a new marker appears; otherwise pin that the new outlet part is valueless by size. (D1)
- [x] 1.2 Add `DepthOfField`, `ColorGrade`, and `FilmGrain` node kinds in a new `nodes/postprocess` module: `source: CameraTarget`, the camera-only outlet, no pose, no children, no `SceneNodeOut`. Defaults per D7. Mirror `an_output_declares_a_camera_port_and_nothing_a_scene_node_has` so a mesh, material, child, or pose connection is refused by schema.
- [x] 1.3 `DepthOfField` inlets: `focal_distance` and `aperture` (plus `source`). `ColorGrade` inlets: `exposure`, `contrast`, `saturation`, `temperature`, `tint`, `hue`. `FilmGrain` inlets: `intensity`. Empty `evaluate`. Test defaults, including `ColorGrade` identity and `FilmGrain` intensity `> 0`.
- [x] 1.4 Register all three kinds in the runtime plugin so the palette, document, and inspector pick them up reflectively. Test that `Camera → ColorGrade → Output` and `Camera → Output` are both legal, and that `ColorGrade → DepthOfField` is type-legal (the runtime refusal is later). `cargo test -p sway-runtime`

## 2. `sway-runtime` — target allocation

- [x] 2.1 Walk camera-target chains: given a producer `NodeId`, resolve its source camera by following `protocol::CAMERA` / `source` until a `Camera` node (or none). Test a three-node chain and a missing source.
- [x] 2.2 Extend `desired_sizes` so a consumed post-process node requests the same size as its source camera (authored vs pane-fit, existing D4). A producer is consumed if Output, Capture, or `EditorCameraPreview` names it, or if a consumed downstream effect names it. Unwired and diagnosed effects request nothing.
- [x] 2.3 Allocate and release effect targets through the existing `CameraTargets` map (D2). Test: `Camera → ColorGrade → Output` allocates two targets of the camera's size; disconnecting Output releases both; editing the camera resolution resizes both. Test that `Camera → Output` plus `Camera → FilmGrain → Capture` keeps the camera target as well as the grain target.
- [x] 2.4 Once-only diagnostics: post-process node with no source; `DepthOfField` whose source is not a `Camera`. No target in either case. `cargo test -p sway-runtime`

## 3. `sway-runtime` — effect passes

- [x] 3.1 Publish a reconstructed-each-frame resource of consumed effect passes: source handle, dest handle, kind, uniforms packed from the node's inlets. Include show-frame index for grain, incremented once per `Update` (D6).
- [x] 3.2 Enable sampleable depth on a camera that feeds a consumed `DepthOfField`; leave other cameras as they are. Test the component is present only in that case.
- [x] 3.3 Gaussian DoF fullscreen pass in `Core3d` after the source camera's main pass: sample camera colour + depth, write the DoF node's `ManualTextureView`. Do not insert Bevy's `DepthOfField` component on the scene camera (D3). If `dof.wgsl` cannot bind a distinct destination, use the wrapper blit called out in Risks.
- [x] 3.4 Color-grade fullscreen pass: pack the six inlets into Bevy's `ColorGradingUniform` (D3) and blit source → dest. Identity defaults MUST copy the source (spec: defaults are a no-op). No pixel-diff: assert the pass is scheduled and the dest handle is the grade node's.
- [x] 3.5 Film-grain fullscreen pass: luminance noise scaled by intensity, hashed with the show-frame index. Intensity `0` MUST skip or copy. Chain `DepthOfField → ColorGrade → FilmGrain` so each pass reads the previous dest. `cargo test -p sway-runtime`

## 4. Present, capture, preview

- [x] 4.1 Point `publish_camera_consumers` at the connected producer, not "the camera". `PresentedCamera.node` / `CaptureIntent.camera` become that producer id; resolution comes from its allocated target. Update diagnostic wording to "camera target". Existing `Camera → Output` tests must still pass.
- [x] 4.2 Test: Output wired to `FilmGrain` publishes the grain node's handle; Capture wired to `ColorGrade` publishes the grade node's handle and the source camera's authored resolution; branching `Camera → Output` and `Camera → FilmGrain → Capture` publishes two different handles. `cargo test -p sway-runtime`
- [x] 4.3 `ViewportCamera` / the preview picker list every `Camera` and every post-process node, plus the editor camera. Fall back to the editor camera when the selected node leaves the document. Previewing an effect consumes it (feeds 2.2). Letterbox aspect walks to the source camera. The editor camera still has no chain. `cargo test -p sway-editor-viewport`
- [x] 4.4 Update the editor camera-list control to show post-process nodes. `cargo test -p sway-editor`

## 5. Verification

- [x] 5.1 Confirm `demo.sway.ron` still loads with no diagnostics and still presents (`Camera → Output` unchanged). Do not add effect nodes to it in this change (design — Migration Plan).
- [ ] 5.2 Manually: wire `Camera → DepthOfField → ColorGrade → FilmGrain → Output`, preview each producer, confirm the chain is visible at each step and that Output / a capture of the last node match. Drive aperture and exposure from the inspector. — needs a display.
