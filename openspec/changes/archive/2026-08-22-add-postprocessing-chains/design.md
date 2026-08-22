## Context

See `proposal.md` — Why. Constraints that shape the approach:

- Camera targets are **host-owned** `sway_gpu` colour textures registered as `ManualTextureView`s. Bevy writes the 3D pass through them; the compositor and capture read them after `app.update()` returns (camera-resolution-and-capture D1, D5). Effect targets MUST be the same kind of object, or present and capture need a second path.
- The `CameraTarget` protocol already carries identity only. `Output` and `Capture` already take that marker on a non-variadic `camera` inlet. Chaining is a new producer, not a new protocol.
- Bevy 0.19 ships `DepthOfField` and `ColorGrading` as **components on the camera entity**, applied to that camera's `ViewTarget`. Attaching them would overwrite the camera's published target and make branching (`Camera → Output` and `Camera → FilmGrain → Capture`) impossible. Film grain is not a Bevy 0.19 effect at all.
- Depth for DoF lives in Bevy's render world (`ViewDepthTexture`). A host blit after `app.update()` cannot sample it without an extra copy out of the render graph.
- `CameraTargets` is already a `NodeId →` allocation map. Effect nodes are `NodeId`s too.

## Goals / Non-Goals

**Goals:**

- One colour target per consumed camera-target producer, including post-process nodes, so a consumer sees exactly the node it is wired to.
- DoF, color grade, and grain as graph nodes whose inlets are ordinary driveable fields.
- Present, capture, and editor preview all consume those targets through the machinery that already exists for cameras.

**Non-Goals:**

- HDR / floating-point targets (effects run on today's `Rgba8UnormSrgb`).
- LUT grading, bloom, vignette, chromatic aberration, bokeh DoF, motion blur.
- Post-process on the editor's own camera.
- A new crate or a new protocol marker.
- Pixel-diff tests.

## Decisions

### D1: Reuse `CameraTarget`; give effects a camera-only outlet

Post-process nodes declare `source: CameraTarget` and an outlet that is only `camera: CameraTarget`. They MUST NOT reuse `CameraTargetOut`, which also carries `SceneChild` — that would make them scene nodes and admit child / pose connections the spec forbids.

`Output` and `Capture` do not change their inlet type. `publish_camera_consumers` already resolves `source_of(..., protocol::CAMERA)` to a `NodeId` and looks that id up in `CameraTargets`. Once effect nodes are in the same map, presenting or capturing a grain node is the same lookup. Diagnostic wording changes from "no camera connected" to "no camera target connected"; the complaint keys can stay.

*Alternative considered — a new `PostProcess` protocol.* Rejected: it would force `Output` / `Capture` to accept two inlet types, or a union marker, for no extra legality. Type equality already allows `Camera → Output` and `FilmGrain → Output`.

*Alternative considered — a variadic `postprocess` inlet on `Camera`.* Rejected: order would be an inlet-slot concern rather than a visible chain, and branching (present unprocessed, capture grained) would be inexpressible.

### D2: One host-owned target per consumed producer, same registry

Extend `CameraTargets` (name can stay; it is already "targets keyed by the node that produced them") so an effect node that something consumes gets a `CameraTarget` of the source resolution, a `ManualTextureViewHandle`, and the same lazy allocate / release rules as cameras.

Desired size is the source **camera's** desired size: authored resolution when a graph consumer needs those pixels, pane-fit when the editor preview is the only consumer of that chain (existing D4). Walking backward from an effect to the camera that starts the chain is a finite `source_of` loop along `protocol::CAMERA`.

A producer is consumed if Output, Capture, or the editor preview names it, or if a downstream effect that is itself consumed names it. Unwired or diagnosed effects allocate nothing.

*Alternative considered — in-place processing of the camera target.* Cheaper, and enough if every consumer always wanted the full chain. Rejected because the spec requires the source target to stay unchanged under branching.

### D3: Effect passes are Bevy render-graph fullscreen draws, not host blits and not camera components

Each consumed effect is a fullscreen pass that samples the source `ManualTextureView` and writes the destination `ManualTextureView`.

- **`DepthOfField`** runs in `Core3d` after the source camera's main pass, sampling that camera's colour *and* `ViewDepthTexture`, writing the DoF node's target. Gaussian only. Parameters map onto Bevy's `DepthOfField` uniform layout so we can reuse `dof.wgsl` rather than invent a second CoC model. The Bevy `DepthOfField` **component is never inserted** on the scene camera.
- **`ColorGrade`** and **`FilmGrain`** run after their source (camera or previous effect) has written, colour only. Grade packs the six inlets into Bevy's `ColorGradingUniform` (exposure / temperature / tint / hue / post-saturation from `ColorGradingGlobal`; contrast applied to every section; lift/gamma/gain left at identity). Grain is a small custom WGSL: luminance-weighted noise scaled by `intensity`, hashed with the show frame index.

Passes are extracted from the graph after projection: a resource of `(source_handle, dest_handle, kind, uniforms)` rebuilt when topology or inlet values change. Empty `evaluate` on the node kinds — they are not tick-time image processors.

*Alternative considered — attach `DepthOfField` / `ColorGrading` to the camera entity.* Rejected: Bevy writes those into the camera's `ViewTarget`, which is the camera node's published image.

*Alternative considered — host blits in `Shell::redraw` after `app.update()`.* Matches capture/present, but DoF cannot see depth without copying `ViewDepthTexture` out of the render world every frame. Keeping DoF in `Core3d` avoids that copy; grade and grain stay in the same graph so a chain is one frame, not a mix of Bevy-then-host that would race pipelined rendering.

### D4: Sampleable depth only on cameras that feed a consumed `DepthOfField`

A camera whose `camera` outlet feeds a `DepthOfField` that will allocate (D2) gets `DepthPrepass` (or equivalent sampleable depth) so `ViewDepthTexture` exists for D3. Other cameras keep today's colour-only target. Resolution change already rebuilds the colour target; depth is rebuilt with it.

`DepthOfField` whose source is not a `Camera` node: no target, diagnostic once (`CameraDiagnostics` style). The connection stays type-legal — legality is still path types, not a special-case protocol. The node is what refuses to run.

### D5: Preview selection is any camera-target producer

`ViewportCamera` becomes the editor camera plus a `NodeId` that names either a `Camera` or a post-process node. The picker lists every such node. Letterbox aspect is the source camera's authored aspect (the same walk as D2). Previewing an effect is a consumer of that effect, which pulls the whole chain into allocation, at pane size when nothing else needs authored pixels.

The editor camera still has no chain.

`PresentedCamera.node` is already "the node whose target we composite". It may now be an effect node. Aspect for letterboxing walks back to the camera as in D2; the host does not need to know which kind it is presenting.

### D6: Grain stability is free

Capture already repeats the last read-back buffer when a slot has no new render. Grain is in that buffer. Do not re-roll grain on a host-side copy, and do not re-read a target that has been redrawn. No extra seed channel.

Show frame index for the grain hash increments once per rendered frame of the show (the 60 fps host pace), not per graph tick.

### D7: Defaults and crate home

| Node | Defaults |
|---|---|
| `ColorGrade` | identity: exposure 0, contrast 1, saturation 1, temperature 0, tint 0, hue 0 |
| `FilmGrain` | intensity `0.1` (visible, not crushing) |
| `DepthOfField` | Bevy `DepthOfField::default()` (Gaussian, focal distance 10 m) |

Node kinds live in `sway-runtime` beside `Camera` / `Output` / `Capture` (same domain, same plugin). No new crate. Shaders live next to the module, as `sprite_material.wgsl` does.

## Risks / Trade-offs

- [Bevy's `dof.wgsl` assumes the camera `ViewTarget` ping-pong, not a distinct `ManualTextureView`] → Mitigation: bind the camera colour as the shader's colour input and the effect target as the colour attachment; keep Gaussian (single extra target, no bokeh dual-output). If the shader cannot be reused without the ping-pong, fall back to a thin wrapper pass that copies the same uniforms into a fullscreen blit we own.
- [Pipelined rendering: sampling a source target the same frame it is written] → Mitigation: schedule each effect pass in `Core3d` after the source camera's main pass (and after a previous effect pass on the same camera) so they are one submitted frame, the way Bevy's own post-process stack is. Do not blit from the host between frames.
- [VRAM: one 1080p `Rgba8` target per effect in a live chain] → Mitigation: allocate only consumed producers (D2). A three-node chain at 1920×1080 is three colour targets plus one depth; accepted for v1. HDR would multiply this; it is a non-goal.
- [MSAA / depth format mismatch on DoF] → Mitigation: scene cameras stay single-sample, matching today's `CameraTarget`. DoF samples that. No MSAA writeback path in this change.
- [LDR DoF and grade are a worse look than HDR] → Mitigation: accepted; floating-point targets are a later change and would resize every camera target, not just the chain.

## Migration Plan

No document format bump. A document that already wires `Camera → Output` loads and presents as it does today: `Camera` is still a legal camera-target producer, and no effect node means no extra pass and no extra target.

Palette gains `DepthOfField`, `ColorGrade`, and `FilmGrain`. Existing tests that count node kinds or assert `OutputIn` field names stay valid (`camera` remains the inlet name).

Rollback is revert: unused effect nodes in a saved document would fail to load on a build that lacks the kinds, so this change should land before any project in the repo starts depending on them. Do not add the nodes to `demo.sway.ron` until the passes work.

## Open Questions

None that change the specs or this approach. Grain's exact hash (monochrome vs RGB noise) is a shader-local choice; pick monochrome luminance noise unless a look test says otherwise during apply.
