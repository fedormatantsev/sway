## ADDED Requirements

### Requirement: A post-process node consumes a camera target and produces one
A post-process node MUST expose a single `source` inlet that accepts a camera target, and an outlet that offers a camera target. The connection on either side MUST carry identity only: no value and no image may travel along it.

The node MUST accept one source. Connecting a second MUST replace the first rather than fail. A node whose source is not connected MUST produce no frames and MUST report a diagnostic naming that node, once rather than on every frame.

A post-process node MUST NOT place anything in the scene: it MUST NOT accept a mesh, a material, children, or a placement, and nothing MUST be drawn for the node itself. It is not a scene node.

Several consumers MUST be able to connect to one post-process node, and every consumer MUST see that node's frames at the same resolution. Connecting or disconnecting a consumer MUST NOT change what the node produces.

#### Scenario: The chain is a sequence of camera-target connections
- **WHEN** a camera is connected to a post-process node's source, and that node's outlet is connected to an output node
- **THEN** both connections are legal
- **AND** evaluating either connection writes no value

#### Scenario: An unwired source produces nothing
- **WHEN** a post-process node's source is not connected
- **THEN** that node produces no frames
- **AND** a diagnostic naming that node is reported once rather than every frame

#### Scenario: A second source replaces the first
- **WHEN** a second camera-target producer is connected to a post-process node's source
- **THEN** the node holds the new connection only

#### Scenario: A post-process node is not a placement
- **WHEN** a mesh node is connected to a post-process node
- **THEN** the connection is refused

#### Scenario: One effect serves several consumers
- **WHEN** one post-process node is connected to an output node and to a capture node
- **THEN** both receive that node's frames at the same resolution
- **AND** the effect runs once, not once per consumer

### Requirement: Depth of field is a post-process node wired to a camera
A `DepthOfField` node MUST be a post-process node. Its source MUST be a `Camera` node, because the effect reads that camera's depth as well as its colour.

A `DepthOfField` whose source is any other camera-target producer MUST produce no frames and MUST report a diagnostic naming that node, once rather than on every frame.

The node MUST expose inlets for focal distance and aperture, both drivable by wires and both editable directly. Default values MUST produce a visible focus effect at typical scene scales rather than leaving the whole image sharp.

#### Scenario: Depth of field follows a camera
- **WHEN** a camera is connected to a `DepthOfField` node's source, and that node is connected to the output
- **THEN** the presented image is that camera's view with out-of-focus regions blurred
- **AND** objects at the focal distance stay sharp

#### Scenario: Depth of field after another effect is reported
- **WHEN** a `ColorGrade` node is connected to a `DepthOfField` node's source
- **THEN** the `DepthOfField` node produces no frames
- **AND** a diagnostic naming that `DepthOfField` node is reported once

#### Scenario: Aperture is driveable
- **WHEN** a wire delivers a new aperture value to a `DepthOfField` node
- **THEN** the amount of blur on the next presented frame reflects that value

### Requirement: Color grading is a parametric post-process node
A `ColorGrade` node MUST be a post-process node. It MUST expose inlets for exposure, contrast, saturation, temperature, tint, and hue, all drivable by wires and all editable directly.

Default values MUST leave the source image unchanged: connecting a `ColorGrade` with every inlet at its default MUST present the same colours as connecting the source directly.

The node MUST accept any camera-target producer as its source, including a camera and any other post-process node.

#### Scenario: Defaults are a no-op
- **WHEN** a camera is connected to a `ColorGrade` node whose inlets all hold their defaults, and that node is connected to the output
- **THEN** the presented colours match the camera's ungraded image

#### Scenario: Exposure is driveable
- **WHEN** a wire raises a `ColorGrade` node's exposure
- **THEN** the presented image is brighter on the next frame

#### Scenario: Grading follows another effect
- **WHEN** a `DepthOfField` node is connected to a `ColorGrade` node's source, and that `ColorGrade` is connected to the output
- **THEN** the presented image is depth-of-field blurred and then graded

### Requirement: Film grain is a post-process node
A `FilmGrain` node MUST be a post-process node. It MUST expose an intensity inlet, drivable by wires and editable directly. The default intensity MUST be greater than zero, so that adding the node is visible without a further edit.

The grain MUST change from frame to frame of the show. It MUST NOT take a time inlet: the show's own frame clock is the source of that variation, the way a capture's file rate is the show's clock rather than a graph value.

An intensity of zero MUST present the source image with no grain.

The node MUST accept any camera-target producer as its source.

#### Scenario: Adding film grain is visible
- **WHEN** a camera is connected to a `FilmGrain` node at default intensity, and that node is connected to the output
- **THEN** the presented image shows grain over the camera's view

#### Scenario: Zero intensity is a no-op
- **WHEN** a `FilmGrain` node's intensity is set to zero
- **THEN** the presented image matches the source

#### Scenario: Grain varies across frames
- **WHEN** two consecutive show frames are presented through the same `FilmGrain` node
- **THEN** the grain pattern on the second frame is not the same as on the first

#### Scenario: Intensity is driveable
- **WHEN** a wire raises a `FilmGrain` node's intensity
- **THEN** the grain is stronger on the next presented frame

## MODIFIED Requirements

### Requirement: What is presented is authored by an output node
An `Output` node MUST exist whose single inlet accepts a camera target. What the running system presents — to the window and to the fullscreen display — MUST be the frames of the node connected to that inlet, and nothing else.

A document with no output node, or whose output node has no camera target connected, MUST present nothing rather than selecting a producer on the author's behalf. Which producer is presented MUST NOT depend on the order nodes were created, the order they project, or which one rendered last.

The output inlet MUST accept one camera-target producer. Connecting a second MUST replace the first rather than fail. A camera node and a post-process node MUST both be legal producers.

An output node MUST NOT place anything in the scene: it MUST NOT accept a mesh, a material, children, or a placement, and nothing MUST be drawn for the node itself.

#### Scenario: The wired camera is what is shown
- **WHEN** a document holds two cameras and one of them is connected to the output node
- **THEN** the presented image is that camera's

#### Scenario: Rewiring the output changes what is shown
- **WHEN** the output node's camera connection is moved to the other camera
- **THEN** the presented image becomes the second camera's
- **AND** the output node holds one connection

#### Scenario: Nothing wired presents nothing
- **WHEN** a document holds a camera but no output node
- **THEN** nothing is presented

#### Scenario: An output node is not a placement
- **WHEN** a mesh node is connected to an output node
- **THEN** the connection is refused

#### Scenario: The output may name a post-process node
- **WHEN** a camera is connected to a `FilmGrain` node and that node is connected to the output
- **THEN** the presented image is the grain node's frames
- **AND** not the camera's unprocessed frames

### Requirement: A capture node writes a camera's frames to image files
A `Capture` node MUST expose three inlets: a camera target, an output path, and a boolean recording flag.

While recording is true, the system MUST write the connected producer's image at a fixed capture rate — currently 60 files per second of show time — at that producer's resolution, never at the size of a window or an editor pane. For a camera that resolution is the camera's authored resolution; for a post-process node it is the resolution of its source, which is the source camera's authored resolution. The rate is fixed rather than authored for now; making it an inlet later MUST NOT require any other part of the node to change.

The path MUST say explicitly where files are written, including how each frame's number appears in the filename. The node MUST NOT choose a directory, a filename or a numbering scheme of its own.

Recording MUST default to false, so that opening a project never writes a file. Each transition of recording from false to true MUST begin a run whose first file is numbered zero; a file already at a path a run writes to MUST be overwritten rather than skipped or renamed.

A capture node whose camera target is not connected, or whose path is empty, MUST write nothing and MUST report a diagnostic naming that node. That diagnostic MUST be reported once rather than on every frame.

A camera node and a post-process node MUST both be legal producers for the camera-target inlet.

#### Scenario: Recording writes at the capture rate
- **WHEN** a capture node's recording flag is true for one second of show time
- **THEN** about 60 image files are written
- **AND** their numbers ascend from zero

#### Scenario: Files carry the camera's authored resolution
- **WHEN** a capture node is connected to a camera authored at 1920×1080 and the editor pane showing it is 640×360
- **THEN** each written file is 1920 pixels wide and 1080 pixels tall

#### Scenario: Recording defaults to off
- **WHEN** a project holding a capture node is opened and nothing sets its recording flag
- **THEN** no file is written

#### Scenario: Clearing the flag stops the run
- **WHEN** a capture node's recording flag becomes false
- **THEN** no further files are written
- **AND** the files already written are left as they are

#### Scenario: A second run restarts numbering
- **WHEN** a capture node records, stops, and records again to the same path
- **THEN** the second run's first frame is numbered zero
- **AND** it overwrites the file the first run wrote at that number

#### Scenario: An unwired capture is reported once
- **WHEN** a capture node's recording flag is true and no camera is connected to it
- **THEN** no file is written
- **AND** a diagnostic naming that node is reported once rather than every frame

#### Scenario: Capturing a post-process node writes that node's frames
- **WHEN** a capture node is connected to a `ColorGrade` node whose source camera is authored at 1920×1080
- **THEN** each written file is 1920 pixels wide and 1080 pixels tall
- **AND** the files hold the graded image, not the camera's ungraded image
