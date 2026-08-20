## Why

Today every camera in the world is pointed at the one viewport texture, whose size is whatever the window or the editor pane happens to be (`headless::retarget_cameras`). A show's framing is therefore incidental rather than authored: the same document renders at a different resolution and a different aspect ratio depending on how the window was dragged. And nothing can get pixels out of the process at all — the only way to see what a document renders is to look at the screen, which rules out archiving a run and rules out an agent verifying a visual change without a human in front of the display.

## What Changes

- A `Camera` node gains a **`resolution` inlet**. A camera renders into a target of exactly that size, independent of the window, the editor pane, and every other camera in the document. Several cameras may exist, each with its own resolution.
- A new **`Output` node** with a single `camera` inlet names what the window and the fullscreen HDMI path present. A document with no output node, or an output node with nothing wired, presents nothing. This replaces "whichever camera happens to render last wins".
- A new **`Capture` node** with `camera`, `path` and `recording` inlets writes the wired camera's target to image files at the camera's full resolution, on a **fixed 60 fps timeline** on the wall clock the show already runs on — not the graph's tick rate. The show itself now renders at a fixed 60 fps whatever the display refreshes at and whether or not anything is capturing, so each file ordinarily holds a distinct frame. Keeping up with the external clock outranks completing the capture: under load, slots are dropped and counted rather than the show being made to wait. `recording` is a plain bool inlet today (toggled in the inspector) so that it can be driven by an event later with no change to the node.
- The **editor viewport** keeps sizing the editor camera to the pane. Previewing a scene camera renders at pane size and uses that camera's `resolution` for **aspect ratio only**, letterboxed inside the pane — so the preview shows the authored framing without paying for the authored resolution.
- The show gets a **fixed 60 fps frame rate**, paced against real time in the host rather than inherited from whatever the display refreshes at. **BREAKING** on a high-refresh display, where the editor and the show currently run as fast as the panel allows.
- A new **`--no-vsync`** flag stops the host waiting for the display's refresh, so a display slower than 60 no longer bounds the show. Off by default; a surface that cannot honour it falls back and says so.
- A new **`--capture-window <path>`** host flag opens the project, renders until the scene has settled, writes one PNG of the whole composited window (viewport plus editor UI, exactly what is on screen) and exits. This is the agent-facing path: it needs no window interaction.
- **BREAKING**: `Camera` gains a serialized field, so documents written before this change load with a default resolution rather than round-tripping byte-identically.
- **BREAKING**: a document that has a camera but no `Output` node presents nothing where it previously filled the window.

## Capabilities

### New Capabilities
- `app`: the host's own behaviour — the show's fixed frame rate, which camera the window and the HDMI path present, and the one-shot whole-window capture invoked from the command line. `sway-app` owns the window, the surface and the compositor, and none of that is authored in the graph, so it does not belong in `runtime` or `editor`. First use of the `app` domain the project config anticipates.

### Modified Capabilities
- `nodes`: the `Camera` node kind gains a `resolution` inlet; two node kinds are added that consume a camera rather than place anything in the scene (`Output`, `Capture`), which requires saying how they relate to the closed scene-node set they are deliberately not part of.
- `runtime`: render-side behaviour gains per-camera render targets sized by the authored resolution, and the rule that captured frames are written at the render rate off a tick-rate flag.
- `editor`: the viewport's camera selection and how a scene camera's authored resolution is honoured (aspect only) when previewing it in a pane of a different size.

## Impact

- `sway-runtime`: `nodes/scene.rs` (the `Camera` inlets), `nodes/protocol.rs` (a camera-target protocol marker and trait, following the existing material/mesh/sequence pattern), `headless.rs` (the single `VIEWPORT_HANDLE` becomes one `ManualTextureView` per camera), `project/scene.rs` (projecting cameras with their own targets), and two new node modules for `Output` and `Capture`.
- `sway-gpu`: a readback path — copying a render target to a mappable buffer and encoding a PNG — plus whole-surface readback for the window capture.
- `sway-editor-viewport`: `camera.rs` — `ViewportCamera` becomes a selection among the document's camera nodes rather than a two-state toggle, and the pane preview applies the aspect-only letterbox.
- `sway-app`: `main.rs` (the `--capture-window` and `--no-vsync` flags), `shell.rs` and `presenter.rs` (the 60 fps pace, presenting the output node's camera, the one-shot capture-then-exit path).
- `sway-gpu`: `surface.rs` — `PresentMode::Fifo` is hardcoded today and becomes a choice made from `SurfaceCapabilities::present_modes`.
- `sway-document`: the camera's new field is serialized; documents written before this change load with the default resolution.
- Testing: node inlets, target sizing, letterbox arithmetic and frame numbering are all testable without a device. Whether the written PNG matches what was on screen is not pixel-diff tested (project rule); the capture path is verified by writing to a temp directory and checking that files appear, are non-empty, and carry the camera's authored dimensions.
