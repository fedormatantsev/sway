## Context

See `proposal.md` — Why. The constraints that shape the approach:

- **The host owns textures, Bevy writes through them.** `sway-app` creates the wgpu device, one `sway_gpu::ViewportTexture`, and a `Compositor` that samples it into the window. Bevy renders headlessly into it through a `ManualTextureView` registered under a single `ManualTextureViewHandle` (`headless::VIEWPORT_HANDLE`), and `headless::retarget_cameras` points *every* camera at that one handle each `Update`. Two cameras therefore overwrite each other, which is why `sway-editor-viewport::camera::apply_active_camera` exists: it toggles `Camera::is_active` so only one draws.
- **Two clocks.** The graph ticks in `FixedUpdate` (`sway-graph`'s `GraphPlugin`; `TICK_HZ = 120.0` in its testing helpers). Rendering happens once per `app.update()`, which the shell calls once per `RedrawRequested`, paced by the vsync'd `surface.present()`. Nothing currently needs to tell the two apart; capture does.
- **The frame loop is ours.** `Shell::redraw` calls `app.update()` and then composites and presents, on the same thread, with the device in hand. There is no Bevy runner.
- **Readback rows are padded.** `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` is 256, and `copy_texture_to_buffer` will not pad for you. `headless.rs`'s existing test already does a correct padded readback of `ViewportTexture` (which carries `COPY_SRC` for exactly this reason) and is the model for the capture path.
- **Pipeline compilation is asynchronous.** That same test documents, from direct observation, that the first frames after startup can land the *global default clear colour* in the target rather than the camera's output, for as many as 60 updates on a cold shader cache. Any "render once and write the file" path that ignores this writes a grey rectangle and reports success.

## Goals / Non-Goals

**Goals:**

- One render target per camera, sized by the graph, with the window no longer deciding any camera's size.
- A capture path that writes on a fixed 60 fps timeline off the show's own clock, and that never makes the show fall behind that clock.
- A window capture an agent can invoke with one command and trust the exit status of.

**Non-Goals:**

- Video encoding. A capture is an image sequence; muxing it into a movie is a separate tool's job.
- Triggering capture from events. `recording` is a bool inlet precisely so that the event system, when it exists, needs no change here.
- Capturing the editor's own camera, or capturing at a resolution other than the camera's authored one.
- Pixel-diff testing of captured output (project rule: no pixel-diff tests).
- More than one window, or presenting two cameras at once.

## Decisions

### D1: A camera's target is a host-owned texture, not a Bevy `Image`

Each camera that something consumes gets its own `sway_gpu` colour target sized to its authored resolution, registered in `ManualTextureViews` under its own `ManualTextureViewHandle`. `retarget_cameras` stops assigning one shared handle and instead assigns each camera the handle of its own target. `VIEWPORT_HANDLE` survives as the editor camera's handle only.

*Alternative considered — `RenderTarget::Image`*, which Bevy 0.19 supports and which would make `bevy_render::gpu_readback::Readback` available for free. Rejected because the compositor samples wgpu textures the host owns, and the wgpu texture behind a `Handle<Image>` lives in the render world (`RenderAssets<GpuImage>`) where the host cannot reach it without a render-graph node. Every presented frame would need an extra blit out of Bevy's world into ours. Keeping targets host-side preserves today's zero-copy present and keeps the readback in the crate that already knows how to do it.

*Alternative considered — keep one shared target and render cameras in sequence.* Rejected: it reintroduces exactly the overwrite problem `apply_active_camera` works around, and makes "capture camera A while presenting camera B" impossible.

Targets are allocated lazily, for cameras something consumes: an output connection, a capture connection, or the editor previewing it. A camera node nothing looks at costs no VRAM. Deleting a camera, or editing its resolution, releases the old target and its handle.

### D2: `Camera` joins the existing marker-protocol pattern

`nodes/protocol.rs` already has the shape this needs: a ZST marker as an outlet plus a `#[reflect_trait]` the projector calls (material, image sequence, mesh source, hierarchy). Camera output becomes a fifth: a `CameraTarget` marker on `Camera`'s outlets, and a matching inlet on `Output` and `Capture`. Per that module's rules the edge is valueless and carries identity only — which is exactly what the `nodes` spec requires of the connection — and it still participates in the evaluation order, so a consumer is guaranteed to be projected after the camera that feeds it.

`Output` and `Capture` are **not** scene nodes. They declare no pose, no children and no `SceneNodeOut`, so the closed scene-node set in the `nodes` spec is untouched and `Graph::connect` refuses a mesh or a material into them by schema, the same way `Group` refuses geometry today.

### D3: Presentation is a resource the host reads, not a camera the host guesses

Projection publishes what the host needs to present: the target handle and authored resolution of the camera the `Output` node names, or nothing. `sway-app` reads that resource the way `presenter.rs` already reads `Graph`, `Selection` and `Transport`, and composites that texture into the window letterboxed to its aspect. No output node, or none with a camera, means no viewport quad — a case the compositor already handles (`present` composites `[ui_quad]` alone when there is no viewport rect).

`sway-editor-viewport::camera::ViewportCamera` stops being a two-state `Editor | Scene` enum and becomes a selection over the editor camera plus the document's camera nodes. `apply_active_camera` keeps its job — exactly one camera drawing into the pane — but is now driven by that selection rather than by a role marker, and `ViewportCameraRole` goes away with `tag_scene_cameras`, whose `Without<RenderLayers>` heuristic for excluding the gizmo overlay camera stops being needed once cameras are identified by the graph node that produced them.

### D4: The preview renders at the fitted pane size, and the camera still renders once

A previewed camera's target is sized to the largest rect of its authored aspect that fits the pane, not to its authored resolution — the preview should cost the pane's pixels. Framing is unaffected because a perspective projection frames by aspect ratio and field of view, not by pixel count, which is the whole reason aspect-only preview is coherent.

When a camera is *both* previewed and consumed by the graph (a recording capture in an editor session, or the show path), the target is allocated at the authored resolution and the preview samples that one target down into the pane. The camera renders once per frame either way, which is what the `nodes` spec's "one camera serves several consumers" requires.

Rounding the fitted rect to whole pixels perturbs the aspect ratio by less than a pixel's worth (a 641-pixel-wide pane fits 641×360 for a 16:9 camera, an aspect of 1.7806 rather than 1.7778). Accepted: it is below the threshold at which framing is observable, and the alternative — snapping the pane rect to exact-aspect sizes — would make the preview jitter as the pane is dragged.

### D5: Capture is driven by a slot clock in the host's frame loop

The capture node publishes intent at tick rate: for each capture node, the target handle of its camera, its path pattern, and whether `recording` is true. The host reads that in `Shell::redraw` after `app.update()` returns, and asks one question per frame — *has the run crossed into a new capture slot?* A slot is `1/60` of show time, counted from the run's start. Crossing one slot issues one readback; crossing none issues nothing; crossing several (a render loop slower than 60 Hz) reissues the most recent frame for each skipped slot, which is what keeps playback timing correct rather than compressing it.

The slot index *is* the file number, so numbering is a timeline rather than a count of what happened to be written, and a dropped slot leaves its number unused. That correspondence is the whole reason the `runtime` spec can require playback to match the show.

Show time is wall time, so a slot boundary is just a wall-clock instant — and because the show itself now runs at a fixed 60 (D5a), a slot boundary and a rendered frame ordinarily coincide. That is what makes the repeat-the-last-frame path in the `runtime` spec a fallback for a scene too heavy to render at rate, rather than the ordinary case. The slot check stays a wall-clock comparison rather than a frame count, so that the timeline is right even when the frame loop is not.

The drain has to live in the host loop rather than in a Bevy system: with `PipelinedRenderingPlugin`, frame N's render runs alongside frame N+1's main schedule, so a main-world system reading a target back reads an indeterminate frame. The host loop, by contrast, runs after `app.update()` has returned and the frame's commands are submitted.

The slot counter lives with the host-side drain, not in the node's state — reset on the false→true edge of `recording`. The node's `state` stays empty and its tick stays a pure function of its inlets, as the `nodes` spec requires of base nodes.

*Alternative considered — Bevy's `gpu_readback::Readback` component.* Convenient (insert while recording, remove when not, one `ReadbackComplete` per frame) and it sidesteps the pipelining race by living in the render world. Rejected for the same reason as D1: it takes a `Handle<Image>` and our targets are not Bevy images. It also fires per *rendered* frame, which is precisely the cadence this design must not have.

### D5a: The frame loop is paced to 60, unconditionally

Today the loop is paced by the vsync'd `Fifo` present alone (`Shell::redraw` ends in `window.request_redraw()`, and `surface.present()` blocks), so the show's rate is whatever the attached display refreshes at. That becomes an explicit 60 fps pace in the shell, independent of capture and independent of the display.

Doing it unconditionally rather than only while recording keeps one frame rate in the system instead of two. A rate that changed when a capture started would make every timing observation — a dropped-slot count, a "is this fast enough" judgement, a golden trace — depend on whether someone happened to be recording.

The pace is a wall-clock deadline per frame, not a count of refreshes: 144 Hz is not a multiple of 60, and a nominal 60 Hz panel is usually 59.94.

`Fifo` still blocks underneath, so by default the deadline is a floor rather than a guarantee — on a 30 Hz display the show renders at 30 and the repeat-frame path in the `runtime` spec covers the difference. A `--no-vsync` flag lifts that floor: `WindowSurface::new` (`surface.rs:60`) hardcodes `PresentMode::Fifo` today, and the flag makes it prefer `Mailbox`, falling back to `Immediate`, falling back to `Fifo` with a diagnostic — `SurfaceCapabilities::present_modes` is the authority and Metal does not offer the same set everywhere. `Mailbox` first because it stops blocking without tearing; `Immediate` tears, which is a fair trade for a capture run and a poor one for a show, so the flag is opt-in and off by default.

With the flag on, the host's own deadline is the only thing pacing the loop, which is exactly the configuration a capture wants: the slot clock and the frame loop then agree on one clock instead of two.

### D6: Readback and encoding are asynchronous, and the show never waits for them

Keeping up with the external clock outranks completing a capture, so nothing on the capture path may block the frame loop.

A slot issues a `copy_texture_to_buffer` into a buffer taken from a small pool and calls `map_async`; the frame then composites and presents without waiting. Each subsequent frame does a non-blocking `device.poll`, collects whatever mappings have completed, hands their bytes plus their slot index to a writer thread over a bounded channel, and returns the buffers to the pool. The writer thread unpads the rows, encodes, and writes.

Saturation is handled by dropping, never by waiting: if the pool has no free buffer, or the writer's channel is full, that slot is dropped and counted. The run reports the count when it ends. A recording that loses frames is a worse recording; a show that misses the external clock is a worse show, and the `runtime` spec now settles that trade in the show's favour.

This inverts an earlier reading of this design, in which readback was synchronous and a stalling frame loop was the mechanism that guaranteed a gapless sequence. Under an external clock that mechanism is exactly backwards — the stall would be the show falling behind the MIDI it is supposed to be following.

`ViewportTexture` already carries `COPY_SRC`; camera targets are created the same way.

### D7: Encoding lives in `sway-app`, readback in `sway-gpu`

`sway-gpu` gains "read this target back as unpadded RGBA8 rows" — a texture concern, next to `ViewportTexture` and the padded-copy logic the headless test already proves out. `sway-app` gains "write these pixels to this path as a PNG" via the `image` crate, because file formats and filesystem errors are host concerns and `sway-gpu` should not learn about either. This keeps the new external dependency (`image`) in the host crate.

### D8: `--capture-window` settles by observation, not by a frame count

The flag runs the ordinary shell with the ordinary presenter, and after each presented frame asks: has every asset resolved and has the graph projected at least once (`architecture`: evaluation waits for assets), and are the last two window readbacks byte-identical? When both hold, write the file and exit successfully. A bounded cap on frames — generous, in the spirit of the existing test's `MAX_UPDATES = 300` — bounds the wait; hitting it is a diagnostic and a failure exit, never a written file.

Stability comparison is what defends against the asynchronous-pipeline-compilation trap documented in `headless.rs`: the wrong-clear-colour frames are stable frame-to-frame *only after* the upscaling pipeline is ready, because before that the destination is cleared to a different colour than after. Requiring assets resolved *and* a projection run *and* two identical frames rejects every failure mode that test found.

The file is written to a temporary path in the destination's directory and renamed into place, so a failure leaves no partial file, as the `app` spec requires.

### D9: `resolution` defaults to 1920×1080

An HDMI show is the target, so that is the useful default rather than a small one. Documents written before this change have no `resolution` field; the loader must supply the default rather than reject the document.

## Risks / Trade-offs

- **Documents written before this change may fail to load** if `sway-document`'s reflect-driven RON path treats a missing field as an error rather than taking `Default`. → Check this first, before any node work; if it does not tolerate a missing field, the fix belongs to the document layer (a default-filling load path) and not to a hand-edit of every existing project file.
- **VRAM grows with camera count** — a document with four 4K cameras allocates four 4K targets. → Lazy allocation (D1) means only consumed cameras cost anything, and the `runtime` spec already requires an over-large target to be reported and skipped rather than silently downsized.
- **A recording may quietly be incomplete.** Dropping is the specified response to saturation (D6), so a run can finish with holes in its numbering and nothing on screen to say so. → The end-of-run diagnostic reports the drop count, and the numbering is slot-based so a hole is detectable after the fact rather than invisible.
- **A 4K capture at 60 fps is roughly 2 GB/s of readback.** No pool size and no writer thread makes that sustainable to disk; such a run will drop most of its slots. → Acceptable for now — the drop count tells the truth about it — but it is the reason the capture rate should become an inlet rather than staying at 60 forever, and the reason an encoder that compresses on the GPU is the eventual answer.
- **Capping the show at 60 is a visible change on a high-refresh display**, and it lands on everyone, not just on people who capture (D5a). → Deliberate for now: HDMI output is the target and one frame rate is worth more than a smoother editor. Giving the editor a schedule of its own is the real answer and is recorded under Future Improvements.
- **The pace and the vsync'd present are two clocks that beat against each other.** `Fifo` blocks on the display's refresh, so a 60 fps deadline on a 59.94 Hz panel drifts in and out of phase and will occasionally miss a slot. → The slot check is a wall-clock comparison rather than a frame count, so drift costs an occasional repeated frame instead of accumulating error in the timeline; `--no-vsync` removes the second clock entirely for runs where the sequence matters more than the screen.
- **Row unpadding is easy to get subtly wrong** — a width whose byte stride is already 256-aligned (1920×4 = 7680) hides the bug that a width like 1000 (4000 bytes) exposes as a skewed image. → Test the unpadding with a deliberately unaligned width, not with 1920.
- **Losing `ViewportCameraRole` touches picking and the gizmo.** `tag_scene_cameras`'s `Without<RenderLayers>` filter currently keeps the gizmo overlay camera out of the toggle, and `pick.rs`/`gizmo.rs` query cameras. → Identify cameras by the node that produced them, and re-check both call sites when the role marker goes.
- **The editor preview and the show path now disagree in resolution by design**, so a scene that looks right in a 640×360 preview is only guaranteed to *frame* right at 1920×1080 — anything resolution-dependent (a thin line, a shader that reads texel size) will not match. → This is inherent in the aspect-only choice; capture at authored resolution is the escape hatch for checking it.

## Migration Plan

The change is additive at the node level and breaking at the document level in one respect: a document with a camera but no `Output` node presents nothing where it previously filled the window. Existing project files therefore need an `Output` node added and wired to their camera — a one-line edit per document, and the diagnostic for "nothing wired to the output" should say so plainly enough that the fix is obvious without reading the spec.

There is no rollback concern beyond reverting the branch: nothing here writes to a document unless the author saves.

## Future Improvements

Recorded here so they are not rediscovered as bugs. None is in scope for this change.

- **The editor should have a render schedule of its own.** D5a gives the whole process one 60 fps pace, which is right for the show and arbitrary for the editor: the editor's UI wants to repaint when something changed and when the pointer moves, and its viewport wants to run at the show's rate only when the author is actually watching the scene move. Splitting them would let the editor idle cheaply, repaint the UI at the display's rate, and drive the scene at the show's — three rates that are genuinely different concerns and are conflated today.
- **The capture rate should be an inlet rather than a constant.** 60 is hardcoded now; 24 and 30 are both obviously wanted, and the node is already shaped so that adding the inlet changes nothing else.
- **Encoding should move off the CPU for high-resolution runs.** A 4K 60 fps run is roughly 2 GB/s of readback, which no writer thread makes sustainable. Compressing on the GPU before readback is the eventual answer, and it is what would make capture at show resolution routine rather than best-effort.
