# app Specification

## Purpose

Defines what the host puts on screen and how a run can be observed from outside it: which camera the window and the fullscreen display present, how that image is fitted to a window of a different size, and the one-shot whole-window capture that lets a run be inspected without a person at the display.

## Requirements

### Requirement: The show renders at a fixed frame rate
The host MUST render at a fixed rate — currently 60 frames per second — whatever the display it is attached to refreshes at, and whether or not anything is capturing. A display that refreshes faster MUST NOT make the show render faster, and starting or stopping a capture MUST NOT change the rate.

The rate MUST be paced against real time rather than counted in refreshes, because a display's refresh rate is neither guaranteed to be a multiple of the show's rate nor guaranteed to be exactly what it claims.

Where a frame cannot be produced in time the rate MAY fall, but the host MUST NOT render ahead to make up for it: a late frame is late, not two frames at once.

By default the host MUST wait for the display's refresh before presenting, so a display refreshing more slowly than the show's rate bounds it. The host MUST accept a request to stop waiting for the refresh; with it, the fixed rate is enforced by the host's own pacing alone and a slow display no longer bounds the show. A surface that cannot honour the request MUST fall back to waiting and MUST report that it did, rather than failing to start.

#### Scenario: A fast display does not speed the show up
- **WHEN** the show runs on a 144 Hz display
- **THEN** it renders about 60 frames per second

#### Scenario: The rate does not depend on capture
- **WHEN** a capture starts and later stops
- **THEN** the show renders at the same fixed rate throughout

#### Scenario: A late frame is not made up for
- **WHEN** one frame takes longer than the fixed interval to produce
- **THEN** the frame after it is not rendered early to compensate

#### Scenario: A slow display bounds the show by default
- **WHEN** the show runs on a 30 Hz display with no request to stop waiting for the refresh
- **THEN** it renders about 30 frames per second

#### Scenario: Waiting can be turned off
- **WHEN** the show is asked not to wait for the refresh on that same 30 Hz display
- **THEN** it renders about 60 frames per second

#### Scenario: An unsupported request falls back rather than failing
- **WHEN** the surface cannot present without waiting for the refresh
- **THEN** the show starts, waiting for the refresh as it does by default
- **AND** a diagnostic says the request could not be honoured

### Requirement: The window presents the camera the output node names
The window and the fullscreen display MUST present the camera connected to the document's output node. When no output node exists, or none has a camera connected, the window MUST present nothing rather than choosing a camera.

The presented camera's authored resolution and the window's size are independent and MUST NOT be reconciled by changing either one. The camera's image MUST be fitted into the window: scaled to the largest size of the camera's aspect ratio that fits, centred, with the remainder of the window left as letterboxing. Resizing the window MUST rescale the presented image and MUST NOT change the camera's resolution or what it frames.

#### Scenario: A window of a different aspect letterboxes
- **WHEN** a camera authored at 1920×1080 is presented in a window 1000 by 1000 pixels
- **THEN** the image occupies a centred 1000 by 563 region
- **AND** the rest of the window is letterboxing

#### Scenario: Resizing rescales rather than reframes
- **WHEN** the window presenting a camera is resized
- **THEN** the same part of the scene is framed
- **AND** the camera's authored resolution is unchanged

#### Scenario: Nothing wired to the output presents nothing
- **WHEN** a project whose output node has no camera connected is run
- **THEN** the window presents nothing
- **AND** the application does not exit or fail

### Requirement: The whole window can be captured from the command line
The host MUST accept a command-line request to capture the window to a named path. That request MUST open the project, render until the image has settled, write one image of the composited window, and exit — with no pointer or keyboard interaction at any point.

What is written MUST be the whole window exactly as displayed, at the window's own pixel size: on the editor path that includes the editor's interface composited over the viewport, and on the show path it is the presented camera alone. It MUST NOT be a camera's render target, and MUST NOT be at a camera's authored resolution.

The path MUST be given explicitly by the request. The host MUST NOT choose a directory, a filename, or a numbering scheme of its own, and MUST NOT write anywhere other than the path given.

The image is settled once every asset the project references has loaded, the graph has been evaluated and projected at least once, and further rendering no longer changes the image. The host MUST NOT write before then: a frame showing a partly loaded scene, or a placeholder produced before rendering is ready, MUST NOT be written.

If the image does not settle within a bounded time, or the file cannot be written, the host MUST report a diagnostic naming the path and MUST exit with a failure status, leaving no partial file behind. A capture that succeeds MUST exit with a success status, so that the exit status alone distinguishes the two.

#### Scenario: A capture request writes one file and exits
- **WHEN** the host is asked to capture the window to a path
- **THEN** exactly one image is written at that path
- **AND** the process exits with a success status

#### Scenario: The capture is the window, not the camera
- **WHEN** the editor is captured while presenting a camera authored at 1920×1080 in a 1280 by 800 window
- **THEN** the written image is 1280 by 800
- **AND** it shows the editor's interface as well as the viewport

#### Scenario: An unsettled scene is not written
- **WHEN** a capture is requested for a project whose assets are still loading
- **THEN** nothing is written until they have loaded and the scene has been drawn

#### Scenario: A failed capture is reported and leaves no file
- **WHEN** a capture is requested to a path that cannot be written
- **THEN** a diagnostic naming that path is reported
- **AND** the process exits with a failure status
- **AND** no partial file exists at that path

#### Scenario: Capturing needs no interaction
- **WHEN** a capture is requested with no one at the keyboard or pointer
- **THEN** the capture completes and the process exits on its own
