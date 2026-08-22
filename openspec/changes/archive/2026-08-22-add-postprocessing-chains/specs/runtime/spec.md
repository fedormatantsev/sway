## ADDED Requirements

### Requirement: A post-process node renders into a target of its own
Each post-process node that has a connected source MUST render into a render target sized by that source's resolution. Two post-process nodes MUST NOT share one target, and a post-process node's target MUST NOT be resized by the window, by an editor pane, or by another node being added or removed.

The pass MUST read the source node's target and write this node's target. The source's own target MUST be left unchanged, so a second consumer of the source still sees the source's frames.

A post-process node whose source cannot produce a target — unwired, diagnosed, or itself producing nothing — MUST produce no target.

Changing the source camera's authored resolution MUST replace every downstream post-process target with one of the new size, and everything reading those nodes MUST see the new size from then on without the project being reopened.

The 3D camera MUST still render once. Each effect in a chain is a further pass, not a further 3D render.

#### Scenario: An effect has its own target
- **WHEN** a camera at 1920×1080 is connected to a `ColorGrade` node that is connected to the output
- **THEN** the color-grade node writes a target 1920 pixels wide and 1080 pixels tall
- **AND** the camera's own target is still the ungraded 3D image

#### Scenario: Branching does not overwrite the source
- **WHEN** a camera is connected to both the output and a `FilmGrain` node, and a capture is connected to the grain node
- **THEN** the presented image is the camera's unprocessed frames
- **AND** the captured files hold the grained frames

#### Scenario: A chain shares one 3D render
- **WHEN** a camera is connected to `DepthOfField`, that node to `ColorGrade`, and that node to the output
- **THEN** the camera renders the scene once
- **AND** each effect writes its own target from the previous node's target

#### Scenario: A resolution edit resizes the chain
- **WHEN** a camera's resolution is edited from 1920×1080 to 1280×720 while a `FilmGrain` node is connected to it
- **THEN** the grain node's target is 1280 by 720 from the next frame
- **AND** the camera's target is 1280 by 720 from the next frame

### Requirement: Depth of field reads the camera's depth
A camera that is the source of a `DepthOfField` node MUST keep a depth buffer that the effect can sample. A camera that is not the source of any `DepthOfField` node is not required to keep one.

The depth buffer MUST match the camera's colour target in size. Replacing the colour target MUST replace the depth buffer with it.

#### Scenario: A camera feeding depth of field has depth
- **WHEN** a camera is connected to a `DepthOfField` node
- **THEN** that effect can distinguish near geometry from far geometry
- **AND** blur amount follows distance from the focal plane

#### Scenario: A camera without depth of field needs no extra depth
- **WHEN** a camera is connected only to the output, with no `DepthOfField` node in any chain from it
- **THEN** that camera still renders its colour target as it did before this change

### Requirement: Film grain is stable for a repeated frame
When a capture slot repeats the most recently rendered frame because the render rate fell below the capture rate, the repeated file MUST hold the same grain as the frame it repeats. Grain MUST NOT be re-rolled for a slot that did not render a new frame.

#### Scenario: A repeated capture slot repeats the grain
- **WHEN** frames are rendered at 30 Hz while capturing a `FilmGrain` node at 60
- **THEN** each rendered frame appears in about two consecutively numbered files
- **AND** those two files hold the same grain

## MODIFIED Requirements

### Requirement: Capturing does not change what is rendered or presented
Reading a camera target back in order to write it MUST NOT change the image that producer renders, and MUST NOT change what is presented to the window.

A producer that is captured and presented at the same time MUST show the same image in both places. Whether a producer is being captured MUST NOT be visible in the presented image.

#### Scenario: The presented image is unaffected by capture
- **WHEN** the camera wired to the output node is also wired to a recording capture node
- **THEN** the presented image is the same as it is when the capture node is not recording

#### Scenario: A captured camera is not tinted, flipped or rescaled by being captured
- **WHEN** a frame is written for a camera
- **THEN** the written image is that camera's rendered frame at its authored resolution, with the same orientation and colours it is presented with

#### Scenario: Capturing an effect does not change the presented chain
- **WHEN** a `FilmGrain` node is wired to the output and to a recording capture node
- **THEN** the presented image is the same as it is when the capture node is not recording
