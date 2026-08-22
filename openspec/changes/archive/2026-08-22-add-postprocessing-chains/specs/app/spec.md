## MODIFIED Requirements

### Requirement: The window presents the camera the output node names
The window and the fullscreen display MUST present the camera target connected to the document's output node. When no output node exists, or none has a camera target connected, the window MUST present nothing rather than choosing a producer.

The presented target's resolution and the window's size are independent and MUST NOT be reconciled by changing either one. The image MUST be fitted into the window: scaled to the largest size of the source camera's aspect ratio that fits, centred, with the remainder of the window left as letterboxing. Resizing the window MUST rescale the presented image and MUST NOT change the camera's resolution or what it frames.

A camera node and a post-process node MUST both be presentable this way. Presenting a post-process node MUST show that node's frames, letterboxed using the source camera's aspect ratio.

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

#### Scenario: Presenting a post-process node shows that node's frames
- **WHEN** the output node is connected to a `FilmGrain` node whose source camera is authored at 1920×1080, in a window 1000 by 1000 pixels
- **THEN** the image occupies a centred 1000 by 563 region
- **AND** the image shown is the grain node's frames
