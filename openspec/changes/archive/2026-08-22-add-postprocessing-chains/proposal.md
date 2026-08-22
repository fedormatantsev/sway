## Why

A camera's render target is currently the unprocessed 3D pass: whatever Bevy writes, that is what the output, a capture, and the editor preview all show. Live looks need depth of field, film grain, and color grading, and those looks need to be authored in the graph — wired, MIDI-driveable, and inspectable at each step — rather than buried as camera-entity components the author cannot see or reorder.

## What Changes

- Three new node kinds — **`DepthOfField`**, **`ColorGrade`**, **`FilmGrain`** — each consume a camera target and produce a camera target, so they chain: `Camera → DepthOfField → ColorGrade → FilmGrain → Output`.
- Each effect node owns a render target of the source resolution. A consumer of that node (output, capture, another effect, or the editor preview) sees that node's processed frames, not the source's. The 3D camera still renders once; each effect is a pass that reads the previous target and writes its own.
- **`Output` and `Capture` accept any camera-target producer**, not only a `Camera` node. Existing `Camera → Output` documents keep working: a camera is still a camera-target producer.
- Effect parameters are ordinary inlets, so MIDI and other graph values can drive focus, exposure, grain, and the rest without new machinery.
- The editor viewport can preview any camera-target producer, so a chain can be inspected at the camera, after DoF, after grading, or after grain.
- `DepthOfField` needs the camera's depth buffer, so it MUST be wired directly to a `Camera`. Wiring it after a colour-only pass is refused with a diagnostic.
- Color grading is parametric (exposure, contrast, saturation, temperature, tint, hue). LUT files, bloom, vignette, chromatic aberration, bokeh DoF, HDR render targets, and effects on the editor's own camera are out of scope.

## Capabilities

### New Capabilities

<!-- none — post-processing is graph-authored scene/runtime behaviour, not a new domain -->

### Modified Capabilities

- `nodes`: three post-process node kinds that consume and produce the camera-target protocol; `Output` and `Capture` accept any producer of that protocol, not only `Camera`.
- `runtime`: each effect node is a pass onto its own target; a camera that feeds `DepthOfField` keeps a sampleable depth buffer; captured and presented frames are the connected producer's target.
- `editor`: the viewport preview list includes every camera-target producer, not only camera nodes.
- `app`: the window and HDMI path present the camera-target the output node names, which may be an effect node rather than a camera.

## Impact

- `sway-runtime`: new post-process node kinds and their projection; a sampleable depth buffer on cameras that feed `DepthOfField`; a target registry keyed by any camera-target producer, not only cameras; fullscreen passes for color grade and film grain (grain is not a Bevy 0.19 effect).
- `sway-gpu`: additional colour targets per effect node (same size and format as today's camera targets); readback and present already copy whatever target they are handed.
- `sway-editor` / `sway-editor-viewport`: preview picker lists cameras and post-process nodes; letterbox aspect still comes from the source camera's authored resolution.
- `sway-document`: new node kinds round-trip through existing reflection; no format version bump. Documents that already wire `Camera → Output` load unchanged.
- Testing: node schema, chain legality, target sizing, and "consumers see the connected node's target" are unit-tested. No pixel-diff tests (project rule).
