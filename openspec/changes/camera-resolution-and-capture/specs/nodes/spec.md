## ADDED Requirements

### Requirement: A camera declares the resolution it renders at
A `Camera` node MUST expose a resolution as a two-component integer inlet. The camera MUST render into a target of exactly that size, independent of the size of any window, any editor pane, and any other camera in the document.

Several camera nodes MUST be able to coexist in one document, each rendering at its own authored resolution. A camera's resolution MUST NOT be changed by anything outside the graph: presenting it, previewing it, or capturing it MUST leave the authored value alone.

A resolution with a zero component MUST NOT produce a render target. A diagnostic naming that camera MUST be reported, and it MUST be reported once rather than on every frame.

#### Scenario: The authored resolution is the target size
- **WHEN** a camera is authored with a resolution of 1920×1080
- **THEN** it renders into a target 1920 pixels wide and 1080 pixels tall

#### Scenario: Resolution does not follow the window
- **WHEN** the window is resized while a camera with a fixed resolution is presented
- **THEN** that camera still renders at its authored resolution

#### Scenario: Two cameras render at their own sizes
- **WHEN** a document holds one camera at 1920×1080 and another at 512×512, each connected to a consumer
- **THEN** each renders into a target of its own size
- **AND** neither one's resolution affects the other

#### Scenario: A zero resolution is reported and renders nothing
- **WHEN** a camera is authored with a resolution whose height is zero
- **THEN** no target is produced for it
- **AND** a diagnostic naming that camera is reported once

### Requirement: A camera offers what it rendered through a connection
A `Camera` node MUST expose an outlet that offers what it renders, so that a consumer names a camera by connecting to it rather than by naming it in a field.

That connection MUST carry identity only. No value and no image may travel along it; the consumer MUST reach the rendered frames through the connection's existence, in the same manner as every other connection to a node that owns an asset.

A camera MUST accept more than one consumer at once, and every consumer MUST see the same frames at the same resolution. Connecting or disconnecting a consumer MUST NOT change what the camera renders, or the resolution it renders at.

#### Scenario: The connection carries no value
- **WHEN** a camera is connected to a consumer
- **THEN** evaluating that connection writes no value into the consumer

#### Scenario: One camera serves several consumers
- **WHEN** one camera is connected to an output node and to two capture nodes
- **THEN** all three receive the same frames at the camera's authored resolution
- **AND** the camera renders once, not once per consumer

#### Scenario: Consuming a camera does not change it
- **WHEN** a consumer is connected to a camera and then disconnected
- **THEN** the camera's authored resolution and what it renders are unchanged

### Requirement: What is presented is authored by an output node
An `Output` node MUST exist whose single inlet accepts a camera. What the running system presents — to the window and to the fullscreen display — MUST be the camera connected to that inlet, and nothing else.

A document with no output node, or whose output node has no camera connected, MUST present nothing rather than selecting a camera on the author's behalf. Which camera is presented MUST NOT depend on the order cameras were created, the order they project, or which one rendered last.

The output inlet MUST accept one camera. Connecting a second MUST replace the first rather than fail.

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

### Requirement: A capture node writes a camera's frames to image files
A `Capture` node MUST expose three inlets: a camera, an output path, and a boolean recording flag.

While recording is true, the system MUST write the connected camera's image at a fixed capture rate — currently 60 files per second of show time — at that camera's authored resolution, never at the size of a window or an editor pane. The rate is fixed rather than authored for now; making it an inlet later MUST NOT require any other part of the node to change.

The path MUST say explicitly where files are written, including how each frame's number appears in the filename. The node MUST NOT choose a directory, a filename or a numbering scheme of its own.

Recording MUST default to false, so that opening a project never writes a file. Each transition of recording from false to true MUST begin a run whose first file is numbered zero; a file already at a path a run writes to MUST be overwritten rather than skipped or renamed.

A capture node whose camera is not connected, or whose path is empty, MUST write nothing and MUST report a diagnostic naming that node. That diagnostic MUST be reported once rather than on every frame.

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
