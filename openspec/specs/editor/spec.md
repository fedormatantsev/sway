# editor Specification

## Purpose

Defines how the graph editor identifies inlets and edges, and which node parts are inspectable: sockets keyed by wire type path; inspector shows inlets only.

## Requirements

### Requirement: Inlet identity is a field path
Each inlet socket on a node MUST be identified by a field path within that node kind's inlets. Sockets MUST be discovered from the node kind's own declared inlets, so that an unconnected inlet has a socket and a node's sockets do not depend on what happens to be connected.

Outlet sockets MUST be identified the same way, by a field path within that node kind's outlets.

Connect legality MUST be decided by comparing the types at the two paths.

#### Scenario: Two inlets stay distinct
- **WHEN** a node kind declares two inlets
- **THEN** the editor lists two sockets whose keys are those field paths
- **AND** an edge for one of them names that path

#### Scenario: Unwired inlet still has a socket
- **WHEN** a node has an inlet with nothing connected to it
- **THEN** the editor still lists that inlet
- **AND** the inlet is not connected

#### Scenario: Layout order is not identity
- **WHEN** a node's visual sockets are re-sorted
- **THEN** an existing edge still attaches to the socket whose key matches its path

### Requirement: Connect and disconnect name a field path
An editor connect or disconnect command MUST name the source node and outlet path and the destination node and inlet path. A connect MUST be refused when the two types are not compatible, when the two nodes are the same node, and when the destination inlet already has a connection and does not accept several.

Connecting to an inlet that already has a connection and does not accept several MUST replace that connection rather than fail.

#### Scenario: Legal drop creates the edge
- **WHEN** the user completes a drag from a legal outlet onto an inlet
- **THEN** an edge is created naming both nodes and both paths

#### Scenario: Illegal drop is a no-op
- **WHEN** a connect command names two paths whose types are not compatible
- **THEN** no edge is created

#### Scenario: A self connection is refused
- **WHEN** the user drags from a node's outlet onto its own inlet
- **THEN** no edge is created

#### Scenario: Reconnecting a single-connection inlet replaces
- **WHEN** the user drops a second connection onto an inlet that accepts one
- **THEN** the inlet holds the new connection only

### Requirement: Edges carry two field paths and an ordering key
Each painted edge MUST carry the source node and outlet path, the destination node and inlet path, and its ordering key. The edge MUST attach to the sockets whose keys are those paths.

Edges landing on an inlet that accepts several connections MUST be presented in ordering-key order, and the editor MUST allow that order to be changed.

#### Scenario: Edge lands on its own socket
- **WHEN** a node has multiple inlets and a connection into one of them
- **THEN** the edge is drawn to the socket whose key is that inlet's path

#### Scenario: Reordering a variadic inlet is possible
- **WHEN** several edges land on one variadic inlet
- **THEN** the editor presents them in ordering-key order
- **AND** the user can change that order

### Requirement: The editor reads the graph without a parallel model
The editor MUST populate its display from the graph itself, using the reflected type information of each node kind. It MUST NOT maintain a second description of nodes, sockets, edges or field kinds alongside the graph.

State that exists only to display the graph — the selection, canvas placement, pan and zoom — is the editor's own and is not a parallel model. The distinction is that no such state describes what a node *is*: removing all of it MUST leave the graph fully described.

Which editing control a field gets MUST be decided from that field's reflected type. A field whose type has no control MUST be shown read-only rather than omitted or misrepresented.

#### Scenario: A new node kind needs no editor change
- **WHEN** a node kind is added whose inlet field types already have controls
- **THEN** it appears in the palette, inspector and canvas with no editor-side description written for it

#### Scenario: A field with no control is shown read-only
- **WHEN** a node has an inlet whose type has no editing control
- **THEN** the inspector shows that field
- **AND** the field is not editable

#### Scenario: Editor state describes no node
- **WHEN** every piece of editor-owned display state is discarded
- **THEN** the graph still describes every node's kind, inlets and connections

### Requirement: Inspector shows inlets only
The inspector MUST show a node's authored inlet fields. It MUST NOT show state. Outlets MUST appear as outlet sockets for wiring, not as authored inspector fields.

A field with a connection MUST still be editable. An edit to a connected field holds until the next tick overwrites it.

#### Scenario: Outlet is a socket, not a field
- **WHEN** a node has inlets and outlets
- **THEN** the inspector lists the inlet fields
- **AND** the canvas offers an outlet socket
- **AND** the inspector does not list the outlet as an editable field

#### Scenario: State is hidden
- **WHEN** a node kind has state
- **THEN** the inspector does not list it

#### Scenario: A connected field is still editable
- **WHEN** an inlet has a connection into it
- **THEN** the inspector still accepts an edit to that field

### Requirement: The editor owns selection and node placement
Which node is selected MUST be editor state. Where a node sits on the graph canvas MUST be editor state. Neither MUST be a value the graph evaluates, orders, or reports as a change.

Node placement MUST be persisted through the node's annotations, so that reopening a project restores the canvas the author left. Selection MUST NOT be persisted; a reopened project starts with nothing selected.

Selecting a node, or moving one on the canvas, MUST NOT cause anything projected from that node to be respawned, rewritten or re-evaluated.

#### Scenario: Selecting a node changes nothing else
- **WHEN** the user selects a node
- **THEN** no node is reported as changed
- **AND** the projected world is untouched

#### Scenario: Canvas placement survives a reload
- **WHEN** the user moves nodes on the canvas, saves, and reopens the project
- **THEN** the nodes are where the user left them

#### Scenario: Selection does not survive a reload
- **WHEN** a project is saved with a node selected and reopened
- **THEN** nothing is selected

### Requirement: An editing control converts its value to the field's type
The editor MUST convert whatever an editing control produced into a value of the edited field's declared type before the edit reaches the graph, because the control is the only thing that knows what it produced.

A numeric edit that falls outside the range of the field's type MUST be clamped to that range rather than discarded, so that the control does not appear to have ignored the input.

#### Scenario: An out-of-range number is clamped
- **WHEN** a numeric control produces a value beyond the range of the field's integer type
- **THEN** the field takes the nearest representable value
- **AND** the control shows that value rather than reverting

#### Scenario: A control's value reaches the field as the field's type
- **WHEN** a control edits a field
- **THEN** the graph receives a value already of that field's declared type

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
