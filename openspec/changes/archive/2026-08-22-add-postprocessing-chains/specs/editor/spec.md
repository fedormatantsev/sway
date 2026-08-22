## MODIFIED Requirements

### Requirement: The viewport previews one camera at a time, chosen by the editor
The viewport MUST show exactly one camera target at a time: the editor's own camera, one of the document's camera nodes, or one of the document's post-process nodes. Which one is showing MUST be editor state — it MUST NOT be a graph value, MUST NOT be reported as a node change, and MUST NOT be persisted with the document.

Every camera node and every post-process node in the document MUST be offerable as a preview, so that a document with several cameras and a chain of effects can be inspected at each producer without rewiring the graph. Which producer the output node names MUST NOT constrain which producer may be previewed.

A previewed producer that leaves the document — deleted, or gone after a reload — MUST fall back to the editor's own camera rather than leaving the viewport blank or showing a stale image.

Exactly one producer's image MUST reach the pane at any moment: switching the preview MUST stop the previous image reaching it rather than layering one over the other.

This constrains what the pane shows, and nothing else. A producer the graph consumes — one an output or a capture node is connected to — MUST go on rendering into its own target whether or not it is the one being previewed, because its consumers are entitled to the same frames either way. Previewing is a further consumer, not a switch that turns the others off.

#### Scenario: Previewing one camera does not stop another being captured
- **WHEN** a capture node records a camera while the viewport previews a different one
- **THEN** the recorded files hold that camera's rendered frames
- **AND** they are not blank

#### Scenario: Previewing a camera does not edit the graph
- **WHEN** the viewport is switched from the editor camera to a scene camera
- **THEN** no node is reported as changed
- **AND** nothing projected from the graph is respawned or rewritten

#### Scenario: Every camera can be previewed
- **WHEN** a document holds two cameras and only one is connected to the output node
- **THEN** both are offered as previews
- **AND** previewing the unwired one shows its view

#### Scenario: A deleted camera falls back
- **WHEN** the camera currently being previewed is deleted
- **THEN** the viewport shows the editor's own camera

#### Scenario: The preview choice is not saved
- **WHEN** a project is saved while a scene camera is previewed, and then reopened
- **THEN** the viewport shows the editor's own camera

#### Scenario: A post-process node can be previewed
- **WHEN** a document holds a camera connected to a `FilmGrain` node, and the output is connected to the grain node
- **THEN** both the camera and the grain node are offered as previews
- **AND** previewing the camera shows the ungrained view
- **AND** previewing the grain node shows the grained view

#### Scenario: Previewing an effect does not stop it being presented
- **WHEN** the viewport previews a camera while the output is connected to a `ColorGrade` node fed by that camera
- **THEN** the presented image is still the graded frames

### Requirement: A previewed camera keeps its aspect, not its resolution
Previewing a camera node MUST render at the pane's own pixel size, not at the camera's authored resolution. The authored resolution MUST contribute its aspect ratio only.

Previewing a post-process node MUST follow the same rule, taking aspect ratio from the source camera that begins that node's chain. The preview MUST show that node's frames, not the source camera's unprocessed frames.

The preview MUST occupy the largest rectangle of the camera's aspect ratio that fits inside the pane, centred, with the remainder of the pane left as letterboxing. What is framed inside that rectangle MUST match what the camera renders at its authored resolution, so that the preview shows the authored framing.

Resizing the pane MUST change how many pixels the preview is drawn with, and MUST NOT change what is framed. Editing a camera's resolution without changing its aspect ratio MUST NOT change the preview at all.

The editor's own camera has no authored resolution: it MUST fill the pane, taking its aspect ratio from the pane itself. The editor's own camera MUST NOT run a document post-process chain.

#### Scenario: The preview is letterboxed to the camera's aspect
- **WHEN** a camera authored at 1920×1080 is previewed in a pane 640 by 480 pixels
- **THEN** the preview occupies a centred 640 by 360 region of the pane
- **AND** the rest of the pane is letterboxing

#### Scenario: The preview costs the pane's pixels, not the camera's
- **WHEN** a camera authored at 3840×2160 is previewed in a pane 640 by 360 pixels
- **THEN** the preview is rendered at 640 by 360

#### Scenario: Resizing the pane reframes nothing
- **WHEN** the pane holding a previewed camera is made larger
- **THEN** the preview is drawn with more pixels
- **AND** the same part of the scene is framed

#### Scenario: A resolution change at the same aspect changes nothing
- **WHEN** a previewed camera's resolution is edited from 1920×1080 to 1280×720
- **THEN** the preview is unchanged

#### Scenario: The editor camera fills the pane
- **WHEN** the editor's own camera is shown in a pane of any size
- **THEN** it fills the whole pane with no letterboxing

#### Scenario: A post-process preview keeps the source camera's aspect
- **WHEN** a `ColorGrade` node whose source camera is authored at 1920×1080 is previewed in a pane 640 by 480 pixels
- **THEN** the preview occupies a centred 640 by 360 region of the pane
- **AND** the image shown is the graded frames
