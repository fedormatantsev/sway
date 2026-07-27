# M1b — Integration spike — Design

**Date:** 2026-07-27
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §2.8, §5 (M1b)

## 1. What this milestone answers

One question: **can one process, one wgpu device, and one winit event loop carry
both a live Bevy viewport and a masonry editor UI?** Everything else in this
document is instrumentation for that.

The parent spec (§5) frames M1b as a single go/no-go gate whose failure mode is
"stop and reconsider against the Syphon route". Reconnaissance done while
writing this design shows the gate is really **two independent questions that
fail differently**, and separating them is the main thing this document adds.

**Question 1 — can the two renderers share a device?** If they cannot, the
fallback is two devices and a per-frame CPU copy: degraded, editor-only, and
still shippable. **This is not a no-go.** Syphon answers this question, which
turns out to be the one that already has a cheap in-process fallback.

**Question 2 — can masonry carry a node editor at all?** Transform-correct
pointer routing to per-node child widgets, arbitrary bezier painting, and drag
state held in the widget tree. There is no cheap fallback here, and **this is
the real no-go**: failing it invalidates §2.8's choice of editor toolkit, not
its compositing story.

The task order in §7 exists to answer the cheap-to-disqualify parts first.

## 2. What reconnaissance found

The parent spec's §2.8 risk reads: *"Masonry draws through Vello; Bevy drives
wgpu directly… Bevy and Vello pin these independently and do not move in
lockstep."* That was true of the crates as published, and is materially out of
date as of masonry `main`.

### 2.1 The version landscape, measured

| Crate | Resolves to wgpu |
|---|---|
| `bevy` 0.19 (our pin) | **29.0.4** |
| `masonry` 0.4.0 (latest release) → `vello` 0.6 | 26 |
| xilem `main` workspace → `vello` 0.8 | 28 |
| `imaging_vello` 0.0.2, default feature `vello-0-9` → `vello` 0.9 | **29.0.4** |

`winit` is **0.30.13** on both sides already and was never the problem.

No *released* (bevy, masonry) pair shares a wgpu major. But masonry `main` has
been restructured such that the question mostly stops applying.

### 2.2 Masonry's renderer split

On `main`:

- **`masonry_core` and `masonry` depend on neither vello nor wgpu.** Paint
  output is an `imaging::record::Scene` — a backend-neutral command stream.
- The renderer lives in a separate crate, `masonry_imaging`, whose own docs say
  it *"does not own window integration, surfaces, or compositor policy"* and
  that it exposes *"host-neutral texture rendering helpers for writing into
  caller-provided WGPU targets"*. Its vello adapter is 52 lines and its entry
  point, `new_target_renderer(device, queue)`, takes an **existing** device and
  queue.
- `imaging_vello` picks its vello — and therefore its wgpu — by feature flag
  (`vello-0-7` / `vello-0-8` / `vello-0-9`). `vello-0-9` is the default and
  resolves to wgpu 29.0.4, which is bevy 0.19's exact version.
- `masonry_core` carries `PaintLayerMode::External` and
  `VisualLayerKind::External { bounds }` — a placeholder layer for
  externally-rendered content, documented as *"current hosts do not realize
  these placeholders yet… this mode exists so the core paint model can represent
  external boundaries before host integration lands."* That is exactly the
  Bevy-viewport-inside-a-widget hole, already cut.
- `RenderRoot` exposes `redraw() -> (VisualLayerPlan, Option<TreeUpdate>)`
  alongside `handle_pointer_event`, `handle_window_event`, `handle_text_event`
  and `size()` — a complete host-embedding API. `masonry_winit` is one host
  among possible hosts, not a requirement.

We do **not** use `masonry_imaging` itself: it pins wgpu 28 through the xilem
workspace. We depend on `masonry` + `masonry_core` (neither of which touches
wgpu) and drive `imaging_vello` with `vello-0-9` directly from `sway-gpu`.

### 2.3 Bevy's half of the handshake

- `RenderCreation::Manual(RenderResources)` accepts an externally-created
  device, queue, adapter, adapter info and instance. All five are public tuple
  structs (`RenderDevice`, `RenderQueue`, `RenderAdapter`, `RenderAdapterInfo`,
  `RenderInstance`), constructible from our own wgpu objects.
- `ManualTextureViews` + `RenderTarget::TextureView(ManualTextureViewHandle)`
  render a camera into a texture **we** created, which is what lets the
  compositor own the viewport texture rather than digging it out of the render
  world.
- `set_transform(Affine)` on a masonry widget, composed into `window_transform`
  and inverted for hit-testing, means pan/zoom is one call rather than
  hand-rolled pointer math. §2.8's warning that "pan/zoom transforms… are all
  hand-written" is half-retracted: the transform and its hit-testing are
  masonry's; bezier edge rendering, curve hit-testing and drag-to-connect
  remain ours.

### 2.4 The cost of this route

`masonry` and `masonry_core` come from git `main`, pinned to rev
`c5950bcb03d4f3d187a20d1159f6aa276fd056bf` (2026-07-03). This is unreleased and
the API is moving. §2.8 already accepts masonry churn on the grounds that the
editor never runs on stage, and that reasoning is unchanged. The `External`
layer in particular is explicitly pre-integration, so we are the first host to
realize it and should expect to find gaps.

## 3. Architecture

```
sway-app (bin)        winit EventLoop + ApplicationHandler + Window; the frame loop
  └ sway-gpu          wgpu 29 Instance/Adapter/Device/Queue, the Surface, the
                      offscreen viewport + UI textures, the compositor pass, and
                      the imaging_vello renderer. The ONLY place any of these are
                      created (§2.8).
  └ sway-runtime      the Bevy App, headless: DefaultPlugins minus WinitPlugin,
                      WindowPlugin { primary_window: None },
                      RenderPlugin { render_creation: Manual(our device) },
                      camera target = RenderTarget::TextureView(handle)
  └ sway-editor       masonry RenderRoot + widget tree. No wgpu. No vello.
```

**`sway-editor` depending on neither wgpu nor vello is the load-bearing
structural fact.** It is what masonry's renderer split buys, and it means a
vello or wgpu bump touches `sway-gpu` and nothing else — §2.8's "confine all
device creation to `sway-gpu`, so a divergence is one file's problem", obtained
rather than merely intended.

Bevy is driven by explicit `app.finish()`, `app.cleanup()`, then `app.update()`
per frame. No runner plugin, no `App::run()`. Winit events reach masonry via
`ui-events-winit` → `RenderRoot::handle_pointer_event` /
`handle_window_event` / `handle_text_event`.

`sway-gpu` requests the **union** of the wgpu features and limits Bevy needs and
those vello needs. This is a named task rather than an afterthought: it is the
most likely place the shared device fails, and its failure is a clean early
signal rather than a mysterious later one.

## 4. The frame

Two presenters over one runtime path, per §2.8.

### Editor presenter

```
1. root.redraw()                  → VisualLayerPlan
2. find the External layer        → viewport rect in window space
3. resize the viewport texture if that rect changed
4. app.update()                   → Bevy renders into the viewport texture
5. imaging_vello renders the plan's scene layers into ui_texture,
   base_color = TRANSPARENT (the External subtree contributes no pixels)
6. compositor pass into the surface texture:
      quad 0: viewport texture, at the External bounds
      quad 1: ui_texture, fullscreen, alpha-blended over it
7. surface.present()
```

**Masonry redraws before Bevy updates** (steps 1–4) so that a viewport resize
costs no frame of lag: the rect is known before the frame that must fill it.

**The UI goes to its own transparent texture rather than interleaving vello
passes.** Vello writes every pixel of its target and cannot blend over existing
content, so "root layer, then viewport, then overlay layers" is not expressible
as two vello passes into one surface. One transparent UI layer composited over
one viewport quad gets overlays-above-viewport right for free, and needs one
compositor pass with two quads.

Both renderers submit to the same queue, so ordering is guaranteed by submission
order and needs no explicit synchronisation.

### Show presenter

```
1. app.update()  → viewport texture
2. compositor pass: one fullscreen viewport quad
3. present
```

Same compositor, no masonry, no vello. Bevy could render straight into the
acquired surface texture and skip the blit; it does not, because §2.8 already
prices the blit at well under a millisecond and one compositor path is worth
more than that.

### A measurement artefact this removes

Owning the loop means vsync comes from `surface.present()`. The M1 findings
record a `--demo scatter` figure of ~1600 fps that was not a frame rate at all —
that demo spawned no camera, Bevy skipped swapchain acquisition entirely, and
the number described an unthrottled app loop. Under this shell every frame ends
in a present, so that class of artefact cannot recur.

## 5. The canvas

Throwaway. M7 replaces all of it; the goal here is to answer question 2 of §1.

One `GraphCanvas` widget owns pan/zoom via `set_transform(Affine)` on its
content, with **a real child widget per node box** — its own `WidgetId`, its own
drag and selection state — and bezier edges painted by the canvas into its own
scene.

Per-node child widgets rather than one widget painting everything is the point.
The parent spec's case for masonry (§2.8) is that *"a graph is already a
retained structure with stable identity per node, port, and edge"*, so a single
custom widget holding boxes as plain data would be an immediate-mode canvas
wearing a masonry hat, and would prove nothing about the claim under test.

**Drag-to-connect is in scope**; per-edge hit-testing is not. Drag-to-connect
uses the same pointer-capture machinery as box dragging and is where masonry
fails if it fails. Edge widgets would need non-rectangular hit areas and
overlapping bounds — the least likely thing to cooperate, for a question M7 can
answer once there are real edges.

Deliberately absent: ports as widgets, inspectors, real graph data. The boxes
are a `Vec<(Id, Point)>`.

## 6. Crate and file layout

```
crates/sway-gpu/            NEW
  src/lib.rs                device/queue/adapter/instance creation, feature+limit union
  src/surface.rs            winit surface config, resize
  src/textures.rs           viewport + UI textures, resize policy
  src/compositor.rs         the two-quad blit pass and its shader
  src/vello.rs              imaging_vello renderer bound to our device
  assets/shaders/composite.wgsl

crates/sway-editor/         NEW
  src/lib.rs                RenderRoot construction, event conversion entry points
  src/canvas.rs             GraphCanvas: pan/zoom, edge painting, drag-to-connect
  src/node_box.rs           the per-node child widget

crates/sway-runtime/
  src/headless.rs           NEW: builds the Bevy App against an external device

crates/sway-app/
  src/main.rs               MODIFIED: winit ApplicationHandler, frame loop, presenter choice
  src/presenter.rs          NEW: editor and show presenters
```

`sway-graph` does not exist yet (M2) and is untouched. `graph.rs` in `sway-app`
stays where it is, per M1's constraint.

**The crates are permanent; `sway-editor`'s contents are not.** `sway-gpu`, the
presenters, and the headless Bevy construction are written as the real thing —
§2.8 names `sway-gpu` as the mitigation that confines version coupling to one
place, so it should exist properly the first time. `canvas.rs` and `node_box.rs`
are the throwaway of §5.

## 7. Task order and failure branches

Ordered so the cheapest disqualifying answer arrives first.

**1. Dependency resolution and one shared device.** Pin xilem `main` at rev
`c5950bc`; verify `masonry_core` and `imaging_vello` agree on the `imaging`
crate version; create a device satisfying both Bevy's and vello's feature and
limit requirements. *Ends in:* a vello-painted rectangle and a Bevy triangle in
one window, on one device.
*If this fails:* fall back to two devices and a per-frame CPU copy (Bevy renders
to a texture, readback, upload into the UI renderer's device). Both devices stay
on wgpu 29 — the likely failure here is an irreconcilable feature or limit set,
not a version mismatch — so this fallback costs a round trip, not the dependency
route. Editor-only cost; the show path never composites. **Continue — this is
not a no-go.**

**2. Presenter and compositor.** Both presenters, with M0's cube and M1's
`--demo` spikes as the content under test. Reusing the existing demos is
deliberate: they are a direct regression signal that manual device creation did
not break the render spikes.

**3. Masonry widget tree, static.** Real per-node widgets, real layout, painted
bezier edges, composited around a live viewport.

**4. Interaction.** Pan/zoom, box dragging, drag-to-connect.
*If this fails:* **no-go.** Report against §2.8 and reconsider the editor
toolkit — not the compositing route, which tasks 1–3 will already have settled.

## 8. Testing

Rendering is verified by eye, per parent spec §4. Three things are testable
without a GPU and are not optional:

- **One `wgpu`, one `winit`.** A test (or `cargo tree -d` in CI) asserting no
  duplicate `wgpu` or `winit` in the resolved graph. §2.8 asks for exactly this,
  so that a duplicated wgpu is a red build rather than a baffling runtime type
  error. It is also the automated form of task 1's exit condition.
- **External-layer geometry.** Given a `VisualLayerPlan` containing an
  `External { bounds }` layer under a non-identity `VisualLayer::transform`,
  assert the window-space viewport rect. This is pure arithmetic over masonry's
  output and is where an off-by-a-transform bug would otherwise be found by
  squinting at a misplaced viewport.
- **Canvas hit-testing under zoom.** `masonry_testing` drives the widget tree
  headlessly: a pointer press at a window-space point under a non-identity
  canvas transform must reach the intended node widget. This is question 2 of §1
  reduced to an assertion, and it should exist before the interaction task
  rather than after it.

Everything else — that the viewport shows the right thing, in the right place,
at frame rate — is looked at.

## 9. Deliberate regression

The show path becomes a plain window. `--monitor` selection and
fullscreen-on-an-external-display, both proven at M0, stop working until M6.

Restoring them is monitor enumeration off the winit event loop plus
`window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))))` — roughly
15 lines. Recorded here so that M6 inherits a known task rather than a
rediscovered bug, and so nobody reads the missing flag as an accident.

## 10. What this milestone must produce besides code

Written into `docs/superpowers/reports/`, in the shape M1's findings took:

1. **Did Bevy and vello share one device?** If not, what failed — feature/limit
   union, resource construction, or something else — and what the two-device
   fallback cost per frame.
2. **Which parts of masonry's host-embedding API were missing or wrong**, given
   that `External` layer realization is explicitly pre-integration upstream.
3. **What the editor frame costs**, split between `app.update()`, the vello UI
   pass, and the compositor pass — the first honest number for how much of a
   frame the editor shell spends before any real content exists.
4. **Anything in parent spec §2.8 that turned out wrong**, stated as plainly as
   §2.1–§2.3 above state what was already found wrong before implementation
   started.
