# M1b integration spike — findings

Consolidated, tracked answers to the four questions the design
(`docs/superpowers/specs/2026-07-27-m1b-integration-spike-design.md`, §10)
requires this milestone to record, plus what a later milestone (M7, the real
editor) would otherwise have to rediscover. Sourced from
`.superpowers/sdd/2026-07-27-m1b-integration-spike/progress.md` — the SDD
ledger, which records several corrections to first-pass explanations — and
the `task-*-report.md` files, which are gitignored scratch and not a durable
record on their own. Where a task report's original claim and the ledger's
later, reviewer-verified correction disagree, this document follows the
ledger.

## 1. Did Bevy and vello share one device?

**Yes, comprehensively, and end to end.** This was the milestone's first
question and its cleanest answer.

Task 1 established the compile-time identity gate: `cargo tree -i wgpu`
shows a single `wgpu 29.0.4` resolved across both `bevy` 0.19 and
`imaging_vello` 0.0.2 (`vello-0-9` feature). The committed test
(`bevy_and_vello_share_one_wgpu`) is deliberately empty-bodied — compile-time
type identity is the assertion; there is no runtime check that could express
it more strongly, and the human ruling on this (pre-flight, recorded in the
ledger) says explicitly not to flag that as a gap. A first review round did
flag a real, adjacent gap — `GpuContext::new()` itself had no committed test
exercising it, so the experimental-features workaround below was
asserted-not-proven — and that was closed with a non-vacuous test before
Task 1 was accepted.

The one real hazard `GpuContext::new` had to route around: this machine's
adapter (Apple M4 / Metal) advertises `EXPERIMENTAL_RAY_QUERY` /
`EXPERIMENTAL_MESH_SHADER` / `EXPERIMENTAL_COOPERATIVE_MATRIX`, and wgpu 29
panics at `request_device` if those are passed through as
`required_features` without an explicit `unsafe ExperimentalFeatures`
opt-in. `GpuContext::new` subtracts `wgpu::Features::all_experimental_mask()`
from the adapter's feature set before requesting a device. Neither Bevy nor
vello need experimental features at this milestone, so excluding them is the
correct call, not a compromise — but it is exactly the kind of thing that
only shows up by trying to construct a real device on real hardware, and
should not be "simplified away" by anyone who finds it and doesn't
understand why it's there.

The proof this actually works, not just resolves: a human confirmed
(2026-07-27) that `--windowed --demo point-cloud` shows the point-cloud
sphere on screen, traced end to end —

```
Bevy renders on sway-gpu's wgpu 29 device via RenderCreation::Manual
  -> into our ViewportTexture through ManualTextureViews/RenderTarget::TextureView
  -> retarget_cameras actually fires
  -> Frame/Compositor blit reaches the winit surface and presents
```

— through **our** texture and **our** compositor, not a separate path. Both
of the design's fallbacks were retired unused: the two-device-plus-CPU-copy
route (design §7, task 1) and the Syphon route (parent spec §2.8) were never
needed, because there was no failure to fall back from.

One honest qualifier on this otherwise clean result: that human sighting
confirmed Bevy's output reaching the screen through the shared device and
our compositor. It did **not** confirm vello's output through the same path
in the same sighting — at that point in the milestone, Task 3 had `--editor`
falling back to `ShowPresenter`, so Task 2's vello rectangle was unreachable
from that run. Whether vello's own pixels, composited alongside the
viewport, were ever separately confirmed by a human is answered honestly in
"What was not proven," below (they were not).

## 2. Which parts of masonry's host-embedding API were missing or wrong?

**`PaintLayerMode::External` worked as the primary path.** The `get_widget`
fallback the design pre-authorized for exactly this risk (design/brief:
*"if `External` does not carry usable bounds... give the viewport widget a
`WidgetId`... and read its window-space layout rect through
`RenderRoot::get_widget(id)` instead"*) was never taken — no fallback code
exists anywhere in the Task 5 diff. But "worked as the primary path" needs
qualifying immediately, because it did not work for free, and the way it
didn't is the richest single finding of this milestone.

### The `PaintLayerMode` reset bug

`masonry_core/src/passes/paint.rs:98` unconditionally resets
`state.paint_layer_mode = Inline` at the top of `paint_widget`, for *every*
widget, on *every* redraw. The only place `set_paint_layer_mode` can be
re-asserted is inside the request-gated block at lines 99–138, which only
runs if the widget's paint was actually requested that frame. So a one-shot
placeholder that calls `ctx.set_paint_layer_mode(PaintLayerMode::External)`
once gets an `External` layer in frame 1 — and then it silently vanishes
from every frame after, because nothing re-requests the paint that would
re-assert it.

This is not a bounds problem — the design anticipated the fallback would
most likely be needed because `External`'s bounds were wrong or absent, and
that never happened: `bounds = state.border_box()` (confirmed by reading
`paint.rs:269`, `push_external_layer(id, state.border_box())`) is always
correct whenever the widget paints at all. The actual gap was one layer up —
a general masonry repaint-scheduling property, not anything specific to
`External`. Upstream's own doc comment on `PaintLayerMode` is technically
accurate and easy to misread: *"This is reset to `Inline` at the start of
each paint pass for the widget"* reads, on a first pass, as "reset before
your `paint()` runs, then set by you" rather than "reset regardless of
whether your `paint()` runs at all this frame."

**The fix**: `ViewportPlaceholder` continuously requests anim frames
(`ctx.request_anim_frame(); ctx.request_paint_only();`), modeled on
masonry's own `Spinner` widget, and `EditorUi::redraw()` pumps
`WindowEvent::AnimFrame(elapsed)` with real wall-clock `Instant` deltas
before every redraw, guaranteeing the widget's `paint()` — and therefore its
`set_paint_layer_mode` call — runs every frame.

**This is the correct host pattern, not a workaround bolted on to cover a
gap.** A reviewer independently verified that `masonry_winit`'s own
reference host (`event_loop_runner::redraw`) does the *identical* thing —
same `AnimFrame` pump, same `Instant`-diff pattern, down to a matching TODO
comment in the upstream source. `PaintCtx::set_paint_layer_mode`'s own doc
comment says to set it "each time they paint." So the finding is not "we
found a masonry bug and patched around it" — it's "we found a
non-obvious invariant of a pre-integration API and, by reading the one
existing host, confirmed we implemented the intended pattern rather than
inventing our own."

### `LayoutCtx` has no `set_transform`

Confirmed against `contexts.rs`: the `impl_context_method!` block that
defines `set_transform` (line 1664) is instantiated only for `MutateCtx`,
`ActionCtx`, `EventCtx`, `UpdateCtx`, and `RawCtx` (the block registered at
line 1476) — not for the `MeasureCtx`/`LayoutCtx` block (line 595). This
matters because "apply pan/zoom during layout" is the intuitive place to
reach for it, and it simply is not available there. Pan/zoom has to be
applied from an event or mutate context instead; `GraphCanvas` carries a
`zoom: f64` field written from `EventCtx`, not `LayoutCtx`, for exactly this
reason. This is an asymmetry in the pre-integration API surface worth
flagging for M7: transform mutation and geometry *reading* live in
different context types, and nothing in the type signatures makes that
obvious ahead of time.

### `WidgetId::next()` is `pub(crate)`, not public

`masonry_core/src/core/widget.rs:650`. A host outside the `masonry_core`
crate cannot mint a `WidgetId` from a raw integer either — its one field is
also `pub(crate)`. The only public route is to build a real widget and read
the ID back off it: `NewWidget::new(w).id()`. This surfaced as an
unanticipated second compile error in Task 5 (beyond the one the brief
predicted), which is itself a small data point on how much of this API
still assumes an in-tree caller rather than an external host.

### `RenderRootSignal`s are dropped, deliberately, and that is a real gap for M7

`RenderRoot::new` takes a signal sink: `impl FnMut(RenderRootSignal)`.
Masonry emits these for cursor changes, IME, and window requests (resize,
title, exit, ...). This spike's sink is a no-op closure — `|_signal:
RenderRootSignal| {}` — because a hardcoded window with no interactive
widgets needing cursor feedback or IME needed none of them. That is fine for
a spike and was flagged as a deliberate simplification at the time it was
written (Task 4), not discovered after the fact. It is recorded here because
a real editor will need cursor changes at minimum — resize handles, a
different cursor while dragging a node or drawing an edge — and M7 should
not have to rediscover that the current code silently discards every
signal masonry emits for that purpose.

### What worked without surprises, for contrast

Two things the design flagged as risk turned out to need no special
handling at all, and are worth naming precisely because a "missing or
wrong" section can otherwise read as more negative than the API actually
was:

- **Pointer routing under a non-identity transform.** `find_widget_under_pointer`
  (`widget.rs:578-609`) tests against `ctx.bounding_box()`, computed in
  `compose_widget` (`compose.rs:33-34`) as
  `window_transform.transform_rect_bbox(paint_box)` — fully
  transform-dependent, and correct without any extra code once
  `set_transform` is called on the canvas. `to_local` (`contexts.rs:1282-1286`)
  inverts `window_transform` for the reverse direction. This is exactly what
  the design's §2.3 predicted ("pan/zoom is one call rather than hand-rolled
  pointer math") and it held.
- **Same-frame mutation resolution.** `RenderRoot::handle_pointer_event`
  (`render_root.rs:547-554`) runs `run_on_pointer_event_pass` then
  `run_rewrite_passes` inside the same call, and `run_rewrite_passes`
  (`829-853`) loops mutate → action → ... → layout → compose up to four
  times within that call. A `NodeBox` action from a click is fully applied —
  layout and hit-testing both updated — before `handle_pointer_event`
  returns. There is no frame of lag between a click and the state it
  produces being hit-testable again.

## 3. What the editor frame costs

**This had not been measured before this task.** It was measured for this
report by temporarily instrumenting `EditorPresenter::present`
(`crates/sway-app/src/presenter.rs`) with `std::time::Instant` around three
phases, logging a rolling average once a second — the same cadence
`log_fps` already uses — then running
`./target/debug/sway --editor --windowed --demo point-cloud` for several
seconds and reading the log. **The instrumentation was reverted before
committing**; `git diff` against `crates/sway-app/src/presenter.rs` is empty
in the commit this report ships with. The numbers below are the record of
that measurement, not a standing feature.

All numbers are from an **unoptimized debug build** (`cargo build`, not
`--release`) on the Apple M4 machine used throughout this milestone,
`--windowed --demo point-cloud`, steady state (after the cold-start
convergence window described in §4 below, which briefly gives `app.update()`
a misleadingly large number — the first 5-frame window measured
163–165ms for `app.update()` alone; that is the async-shader-load stall
from §4, not a representative frame). Two independent ~9-second runs agreed
to within noise:

| Phase | Time (steady state) |
|---|---|
| `app.update()` (Bevy) | ~3.0 ms |
| vello UI pass (`ui_renderer.render_scene`) | ~2.25 ms |
| compositor (`begin_frame` acquire + `composite` encode) | ~10.7 ms |
| `present()` (`queue.submit` + `surface_texture.present()`) | ~0.48 ms |
| **Sum** | **~16.4 ms**, consistent with the measured 60fps (`log_fps`, unchanged, logged alongside) |

The number in the "compositor" row needs the same honesty M1's findings
applied to its `--demo scatter` figure: **it is not a clean measurement of
GPU compositor-pass cost.** `WindowSurface::begin_frame` calls
`surface.get_current_texture()` before `Frame::composite` ever records a
draw call, and on this backend/present-mode (`Fifo`) that acquire call is
where the CPU actually blocks waiting for the next presentable swapchain
image — i.e. it is where vsync pacing happens, not inside `present()` as the
phase names might suggest. `Frame::composite` itself only *records* commands
into an already-open encoder; it does not submit them. `Frame::present`
calls `queue.submit` (submission returns once commands are enqueued, not
once the GPU finishes executing them) and then `surface_texture.present()`.
So: the ~10.7ms "compositor" figure is dominated by the vsync wait bundled
into surface acquisition, not by the cost of drawing two quads, and none of
these four numbers measure GPU execution time — they measure CPU wall-clock
time around each call, which for `submit`/`present` is not the same thing as
GPU work completing. A later milestone that wants real GPU-side timings
needs wgpu timestamp queries or a profiler, not `Instant::now()` around
these calls. What this measurement *does* establish honestly: `app.update()`
and the vello UI pass are both small and roughly comparable (~3ms and
~2.25ms) at this content scale (one placeholder-era canvas, no real graph
data yet), and the frame as a whole is vsync-bound at 60fps with headroom —
none of the CPU-side phases individually approach a 16.6ms budget.

## 4. Anything in spec §2.8 that turned out wrong?

Two things, both real, both already partly self-corrected by the design
document itself before implementation started (§2.1–§2.4), and one thing
the design got right that is worth confirming rather than assuming.

**§2.8's headline risk — wgpu version alignment between Bevy and vello —
was not the milestone's main risk, and did not need to be managed as one.**
§2.8 (parent spec) reads: *"Bevy and Vello pin these independently and do
not move in lockstep."* That was true of the crates as published. It was
already stale by the time this milestone's own design doc was written:
masonry `main`'s renderer split (`masonry_core`/`masonry` depend on neither
vello nor wgpu; the vello adapter lives in a separate `masonry_imaging`
crate; `imaging_vello`'s `vello-0-9` feature happens to resolve to the exact
same wgpu 29.0.4 as Bevy 0.19) had already dissolved the alignment problem
before Task 1 wrote a line of code. The design's own §2.1–§2.4 record this
correctly as reconnaissance. What the design did *not* get right is the
overall risk-weighting that followed from it: by structuring §1 as two
independently-failing questions and calling device-sharing "the one that
already has a cheap in-process fallback," the document still reads, on a
first pass, as treating Q1 as the more uncertain of the two. In practice Q1
worked on the first real end-to-end run and needed no fallback at all, while
the genuine friction (§2, above) turned up inside Q2, in a place §5 did not
name: not bezier rendering, hit-testing, or drag-to-connect (all of which
passed the Task 7 gate cleanly) but the `PaintLayerMode` host-integration
gap in `External` itself.

**§2.8's claim that pan/zoom transforms and hit-testing would be
hand-written was already half-retracted by this design's own §2.3, and the
retraction held.** `set_transform(Affine)` plus `window_transform`
inversion supplied both, exactly as §2.3 said. What remained genuinely
"ours," confirmed by what actually got built rather than assumed up front:
bezier edge rendering, curve hit-testing (never implemented — deliberately
out of scope per design §5), and drag-to-connect (implemented, using the
same pointer-capture machinery as box dragging, and part of what the Task 7
gate proved).

**One thing the design predicted correctly and is worth stating plainly
rather than silently confirming**: §2.4 named `External` as "explicitly
pre-integration" and said to expect gaps. It was right to expect them —
§2's `PaintLayerMode` finding is exactly that kind of gap — and right that
the gaps were the kind a host discovers by trying to be the first real
host, not the kind visible from reading the API alone.

## What a later milestone would otherwise rediscover

- **Apple M4 / Metal adapter experimental-features subtraction.** See §1.
  `GpuContext::new` must subtract `wgpu::Features::all_experimental_mask()`
  from `adapter.features()` before requesting a device, or `request_device`
  panics with `ExperimentalFeaturesNotEnabled`. Do not remove this as dead
  code without understanding it is load-bearing on this hardware.

- **`UiTexture` vs `ViewportTexture` need opposite texture usages, for
  opposite reasons.** `UiTexture` (`crates/sway-gpu/src/textures.rs`) is
  `STORAGE_BINDING | TEXTURE_BINDING`, deliberately *not*
  `RENDER_ATTACHMENT`: `vello::Renderer::render_to_texture` writes through a
  compute pipeline (a storage-texture write), never a render pass, and
  `RENDER_ATTACHMENT` is neither required nor sufficient — confirmed against
  `imaging_vello`'s own internal offscreen target, and by the wgpu
  validation error omitting `STORAGE_BINDING` produces at the first
  `create_bind_group` inside vello's renderer. `ViewportTexture` is the
  opposite: `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC`, because Bevy
  writes to it as an ordinary render target.

- **The colour-space scheme**, fixed once (before Task 1) specifically
  because getting it wrong produces washed-out or double-dark output that
  costs hours of squinting to diagnose:

  | Surface | Format | Holds |
  |---|---|---|
  | Viewport texture (Bevy's view) | `Rgba8UnormSrgb` | Bevy renders linear; hardware encodes on write |
  | Viewport texture (compositor's view) | `Rgba8Unorm`, same backing texture, `view_formats` lists both | samples the already-encoded bytes raw |
  | UI texture | `Rgba8Unorm` | forced by vello's `supported_texture_formats()`; vello writes encoded bytes |
  | Window surface | `Bgra8Unorm` (non-sRGB) | compositor writes encoded bytes straight through, no gamma math in `composite.wgsl` |

  Verified correct (Task 2), including a worked-example check of the
  `to_ndc` arithmetic. `ViewportTexture::new` needs both views built from
  one texture with `view_formats` declaring the second format at creation,
  or wgpu rejects the view.

- **`retarget_cameras` was sufficient; none of M1's demo files needed
  editing.** A single idempotent system (`Query<&mut RenderTarget,
  With<Camera>>`, compare-before-assign so it doesn't mark the component
  changed every frame) retargets every camera at every demo's spawn point to
  the shared `ManualTextureViewHandle`, registered in `PostStartup` and
  `Update`. This was possible because `RenderTarget::normalize` handles an
  unresolvable `Window(Primary)` target (with no primary window present) by
  returning `None` rather than panicking — confirmed by reading
  `bevy_camera::camera.rs:922`, not merely assumed.

- **`RenderRootSignal`s are dropped.** See §2. Recorded again here because
  it is exactly the kind of thing a later milestone would otherwise
  rediscover by wondering why cursor feedback doesn't work.

- **Real Bevy 0.19 API shapes that diverged from what a plan/brief written
  against an earlier mental model assumed** (verified against
  `bevy_render`/`bevy_camera` 0.19.0 source, not inferred from compiler
  errors alone):
  - `Camera` has **no `target` field**. `RenderTarget` is a separate
    required component, defaults to `Window(Primary)`, and does **not**
    derive `PartialEq` — `matches!` against it, don't `==` it.
  - `WgpuWrapper` is at `bevy::render::renderer::WgpuWrapper`
    (`bevy::render::WgpuWrapper` is private).
  - `ManualTextureViewHandle`/`RenderTarget` come from `bevy::camera::*`,
    not `bevy::render::camera::*`.
  - `RenderCreation::manual` argument order is `(device, queue,
    adapter_info, adapter, instance)`.
  - `DeviceDescriptor<L>` gained an `experimental_features:
    ExperimentalFeatures` field since whatever wgpu version older
    reference material assumed; use `..Default::default()` in the struct
    literal so future field additions don't break the build again.

- **The upscaling-blit convergence delay — corrected mechanism.** Pixels
  reach a `ManualTextureView` only via `bevy_core_pipeline`'s upscaling blit
  node. The *first* explanation recorded for why a cold run needs up to
  ~60 `app.update()` calls before the viewport texture shows real content
  (rather than the default `ClearColor`, 43/44/47/255) was "the upscaling
  pipeline compiles asynchronously." **That explanation was wrong and was
  corrected** in the ledger after a scoped re-review against
  `bevy_core_pipeline-0.19.0` source: `upscaling/mod.rs`'s
  `prepare_view_upscaling_pipelines` already calls
  `pipeline_cache.block_on_render_pipeline()` the first time a view's
  upscaling pipeline is created, specifically to prevent this race — but
  that only blocks once pipeline *creation* has actually started
  (`Creating(task)`); if the blit **shader asset** itself hasn't finished
  loading through Bevy's async asset pipeline yet, pipeline creation fails
  immediately with `ShaderNotLoaded`, nothing blocks, and the frame falls
  back to an ordinary per-frame retry. The observed behaviour (clear-and-
  skip-blit for a variable number of frames) is real and unchanged; only
  the stated cause was wrong. A test that renders one frame and asserts on
  pixels will be flaky for this reason; the committed readback test polls
  with a bounded cap (300 frames) instead of assuming a fixed frame count.
  Anyone extending or copying that test should keep the poll-with-cap
  pattern, not "wait N frames."

- **`WidgetId::next()` is `pub(crate)`.** See §2. Mint IDs via
  `NewWidget::new(w).id()` from outside `masonry_core`.

- **The deliberate windowed-only regression.** `--monitor` selection and
  fullscreen-on-an-external-display, both working at M0, stop working as of
  this milestone's shell rewrite, deliberately, until M6. Restoring them is
  monitor enumeration off the winit event loop plus
  `window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))))` —
  roughly 15 lines (design §9's own estimate; nothing in this milestone's
  implementation contradicts it). `Args.monitor`/`Args.windowed` are already
  parsed and merely unread (`#[allow(dead_code)]`); `log_monitors` is
  registered every `Update` but is a permanent no-op in this shell.

- **The pinned masonry rev.** `masonry`/`masonry_core`/`masonry_testing` are
  pinned to xilem `main` at `c5950bcb03d4f3d187a20d1159f6aa276fd056bf`
  (2026-07-03). This is unreleased and moving; every API shape recorded in
  this document and in the ledger is true *at that rev* and should be
  re-verified against whatever rev is current before M7 relies on any of
  it verbatim — this report cannot state what, if anything, has moved since,
  only that something plausibly has.

## Deferred minor findings (for final whole-branch review triage)

Collected from every task's reviewer-approved "deferred minor" list. None
blocked its task's acceptance; none were re-triaged here as anything other
than minor. Two findings that were originally deferred (Task 4's stale
`Dimensions::MAX` comment and its unclamped editor-path resize) were
subsequently fixed in Tasks 6 and 5 respectively and are **not** repeated
below as still-open.

1. `crates/sway-gpu/src/context.rs`'s module doc says device features are a
   "union of what Bevy and vello need," but the code actually requests the
   adapter's whole non-experimental feature set — the doc overclaims
   relative to what's implemented.
2. `crates/sway-gpu/src/context.rs`'s `use wgpu::{...}` import list is not
   alphabetically ordered.
3. `crates/sway-gpu/src/textures.rs` keeps an `#[allow(dead_code)] texture:
   Texture` field on `UiTexture` for speculative future readback that
   nothing currently uses.
4. `shell.rs`'s comment near `app.finish()`/`app.cleanup()` overclaims:
   it attributes a panic hazard to `RenderCreation::Manual` vs `Automatic`,
   but `create_render` calls `bevy_tasks::block_on(...)` synchronously
   inside `build()` regardless, on native — the real dividing line is
   native vs wasm32, not Manual vs Automatic. Worth a corrected comment (or
   a guard) before this shell code is reused past this spike.
5. `Args.monitor`/`Args.windowed` are parsed but unread
   (`#[allow(dead_code)]`); `log_monitors` is a permanent no-op but is still
   registered every `Update`. (Companion to the deliberate regression
   above — not a bug, but dead weight worth cleaning up alongside restoring
   `--monitor`.)
6. The `AnimFrame` pump that keeps `ViewportPlaceholder`'s `External` layer
   alive (see §2) locks the entire widget tree into perpetual anim+paint
   evaluation every frame. Harmless only because `shell.rs`'s loop has been
   unconditionally continuous since Task 3; revisit if the shell ever
   becomes event-driven for battery life.
7. `ViewportPlaceholder::new` has no `Default` impl (clippy would flag
   this).
8. No unit tests exist for the bezier edge-endpoint math in
   `crates/sway-editor/src/canvas.rs`, nor for `GraphCanvas::with_node`'s
   id-mismatch panic path.
9. `NodeBox`'s non-primary-button guard treats `button == None` (the touch
   case) as non-primary, unlike masonry's own convention elsewhere
   (`button.rs`/`text_area.rs`/`slider.rs` match `None | Some(Primary)`). A
   touch tap over a node would bubble to `GraphCanvas` and clear selection
   instead of selecting the node. Out of scope for a desktop mouse-driven
   editor spike, but real, and relevant if touch is ever in scope (see
   "What was not proven," below).

## What was not proven

Stated as plainly as the positive findings, per the standard M1's report
set:

- **No pixel-level verification of the composited UI over the live
  viewport ever happened.** The one human visual confirmation this
  milestone has (2026-07-27, `--windowed --demo point-cloud`) was of the
  Bevy sphere rendering through the shared device — it answered §1's
  device-sharing question and nothing else. Task 4's own "outstanding
  visual check" — masonry's panel actually rendering, the viewport inset at
  its intended rect, and the alpha composite of the transparent UI texture
  over the viewport quad — was recorded as outstanding when Task 4 landed
  and is **not** resolved anywhere later in the ledger. Everything after
  that point (Tasks 5–7) is verified by masonry headless test harnesses and
  code-reading against masonry's own source, not by a human looking at the
  window. That is a meaningfully weaker guarantee than it might sound: the
  colour-space table (above) and the alpha-blend flag on the UI quad are
  exactly the kind of thing that can be validation-clean and test-clean
  while still being visually wrong (a swapped sRGB/non-sRGB view pairing,
  for instance, produces no wgpu error at all).
- **Interaction was verified by tests, never by a human clicking.** The
  Task 7 gate test (node selection under a non-identity zoom transform) is
  not vacuous — a reviewer hand-computed the geometry and confirmed the
  test discriminates a genuine transform application from a coincidental
  pass — but it is `masonry_testing`'s synthetic pointer-event harness, not
  a real mouse. Dragging, panning, zooming, and drag-to-connect are all
  exercised the same way: correct by test and by source-reading, not by
  anyone's hand on a trackpad.
- **Touch input is unsupported**, and in one specific way, actively wrong:
  see deferred-minor finding 9 above. This was never in scope (design §5
  assumes a desktop mouse-driven editor) but is worth stating rather than
  leaving implicit.
- **The viewport-under-live-interaction combination was never exercised in
  one run.** Task 7's own report notes the run that confirmed
  `viewport_rect` still returns `Some` every frame involved *no* pointer
  interaction at all — dragging a node while the Bevy viewport is
  simultaneously animating underneath was reasoned about (the `AnimFrame`
  pump is structurally decoupled from pointer handling, so the risk is
  argued to be low) but never actually run and watched.
- **The frame-cost numbers in §3 are debug-build, CPU-wall-clock numbers on
  one machine, for placeholder-era content** (one static canvas, no real
  graph data, no real node count to speak of). They say the shell's own
  overhead is small relative to a 16.6ms budget at this scale; they say
  nothing about how any of these numbers scale with a real graph, and the
  "compositor" figure in particular measures something closer to
  vsync-wait-plus-encode than compositor GPU cost, as explained in §3.
  Nobody should quote a "10.7ms compositor pass" out of that context.
