# nodes Specification

## Purpose

Defines the built-in authorable components a document may name and a palette may offer: what each contributes to the scene, what its inlets accept, and how mesh nodes and material wires divide responsibility for a mesh's material.

## Requirements

### Requirement: A plane mesh is authorable with independent subdivision counts
A `PlaneMesh` node MUST produce a tessellated quad mesh. It MUST expose a size and two subdivision counts named `horizontal` and `vertical`, settable independently. The quad MUST face a single fixed axis so that its authored horizontal and vertical extents correspond to the horizontal and vertical axes of any texture mapped onto it.

Subdivision counts MUST be authorable because the vertex density governs the fidelity of any displacement applied by a material (see the `runtime` capability), and that density is a property of the geometry rather than of the material.

#### Scenario: Subdivision counts produce a tessellated grid
- **WHEN** a `PlaneMesh` is authored with `horizontal: 3` and `vertical: 1`
- **THEN** the produced mesh has interior vertices along both axes
- **AND** the vertex count along the horizontal axis is greater than along the vertical axis

#### Scenario: Zero subdivisions is a flat quad
- **WHEN** a `PlaneMesh` is authored with `horizontal: 0` and `vertical: 0`
- **THEN** the produced mesh is a single quad of four vertices

#### Scenario: Subdivision counts are independent
- **WHEN** only the `vertical` count of an existing `PlaneMesh` is changed
- **THEN** the produced mesh's vertex density changes along the vertical axis only

### Requirement: Geometry, material and placement are separate nodes
A mesh, a material and a placement in the scene MUST each be a node of its own. A node that produces geometry MUST NOT carry a placement, and a node that places something in the scene MUST NOT carry geometry.

One geometry node MUST be connectable to several placements, and every connected placement MUST use the same geometry without loading or building it more than once. Sharing MUST therefore be visible as connections in the graph rather than implied by two nodes naming the same path.

#### Scenario: One mesh serves several placements
- **WHEN** one mesh node is connected to three scene nodes
- **THEN** all three render that mesh
- **AND** the mesh is loaded or built once

#### Scenario: A geometry node has no placement
- **WHEN** a mesh node is created and connected to nothing
- **THEN** it has no transform
- **AND** nothing is drawn for it

### Requirement: A node that owns an asset does not pass it along a connection
A node that loads or builds an asset MUST keep that asset to itself. A connection to such a node MUST carry no value; the consumer MUST reach the asset through the connection's existence rather than by receiving it.

Consequently no connection in the graph carries an asset, and nothing that identifies a loaded asset is ever authored, serialized, or observed during evaluation.

#### Scenario: An asset connection carries nothing
- **WHEN** a node that owns an asset is connected to a consumer
- **THEN** evaluating that connection writes no value into the consumer

#### Scenario: The consumer still reaches the asset
- **WHEN** a node that owns an asset is connected to a consumer
- **THEN** the consumer renders with that asset

#### Scenario: Disconnecting releases the asset
- **WHEN** the node that owns an asset is deleted
- **THEN** the asset is released
- **AND** the consumers that were connected to it no longer render with it

### Requirement: A material node attaches itself to what it is connected to
A material node MUST be responsible for applying its own material to every scene node connected to it. Nothing outside the material node may need to know which kind of material it is.

Connecting a material node MUST make the connected scene node render with that material. Disconnecting it MUST stop that.

#### Scenario: Connecting applies the material
- **WHEN** a material node is connected to a scene node that has no material
- **THEN** that scene node renders with the material

#### Scenario: Disconnecting removes the material
- **WHEN** a connected material node is disconnected from a scene node
- **THEN** that scene node no longer renders with the material
- **AND** nothing is drawn for it

#### Scenario: Adding a material kind needs no change elsewhere
- **WHEN** a new kind of material node is added
- **THEN** it applies to scene nodes with no change to the scene node kind

#### Scenario: A hand-authored document connects the same way
- **WHEN** a document names a scene node and a connection from a material node to it
- **THEN** loading the document produces a scene node that renders with that material

### Requirement: The scene node set is fixed
The nodes that place things in the scene MUST be a fixed set: a mesh placement, a group, a camera, a directional light and a point light. Scene nodes MUST NOT be assembled from an open set of parts.

A group MUST carry translation, rotation, scale and children and nothing else. It MUST NOT accept geometry or a material.

Every scene node MUST accept children. A child connection MUST make the child's placement relative to its parent's, and a scene node with no child connection MUST NOT be given a parent.

Scene placement MUST be three declared inlets — translation (`Vec3`), rotation (`Quat`) and scale (`Vec3`) — not one compound transform inlet. A `Vec3` outlet MUST connect to `translation` or `scale`; it MUST NOT connect through a nested path on a transform.

#### Scenario: A group places its children without drawing
- **WHEN** three mesh placements are connected as children of a group and the group is moved
- **THEN** all three move with it
- **AND** nothing is drawn for the group itself

#### Scenario: A group refuses geometry
- **WHEN** a mesh node is connected to a group
- **THEN** the connection is refused

#### Scenario: Translation is a declared inlet
- **WHEN** a `Vec3` node is connected to a mesh placement's translation
- **THEN** the edge names `translation`
- **AND** the canvas draws that edge to the translation socket

#### Scenario: An unparented node has no parent
- **WHEN** a scene node has no child connection into any other scene node
- **THEN** its placement is not relative to any other node

### Requirement: A mesh node carries no material until one is wired
A scene node MUST NOT supply a material of its own. A scene node with no material connected MUST NOT render.

This is required because a material is typed per material kind: a scene node that supplied one kind unconditionally would, once a connection delivered a second kind, be drawn once per kind.

#### Scenario: A newly created mesh node has no material
- **WHEN** a scene node is created with no material connected
- **THEN** it carries no material
- **AND** nothing is drawn for it

#### Scenario: A mesh never carries two material kinds at once
- **WHEN** a scene node is connected to a material node of one kind
- **AND** is then connected to a material node of a different kind
- **THEN** it renders with exactly one material
- **AND** it is drawn once

### Requirement: A node kind is named for what it does, not for what it produces
A node kind's name MUST NOT be the name of a type it constructs or consumes. A name that collides with a type already in scope forces every use of that type to be aliased, and makes a document entry ambiguous about whether it names a node kind or a value type.

A node kind whose purpose is to assemble a value MUST be named for the assembling, so the node and the type it produces can be discussed and imported together.

#### Scenario: A constructing node does not take its output type's name
- **WHEN** a node kind's outlet is a value of some named type
- **THEN** the node kind has a different name from that type

#### Scenario: A palette entry is unambiguous
- **WHEN** the palette lists a node kind that assembles a value
- **THEN** its name says what it makes rather than naming the made type

### Requirement: A vector inlet may be driven whole or per component
A node kind that consumes a vector value MUST declare it as one vector inlet. That inlet MUST be connectable as a whole, and MUST also accept a connection naming a single component, so that one component can be driven while the others keep their authored values.

Both routes MUST be available: assembling a vector once and fanning it out to several consumers, and reaching into one consumer's vector inlet directly.

#### Scenario: One component is driven and the rest are kept
- **WHEN** a connection names the second component of a vector inlet
- **THEN** that component takes the connected value each tick
- **AND** the inlet's other components keep their authored values

#### Scenario: Whole and partial connections coexist
- **WHEN** one node's vector inlet is connected whole and another's is connected per component
- **THEN** both are legal
- **AND** neither route required a change to the consuming node kind

### Requirement: A base node is a pure function of its inlets and state
A node kind in the base set MUST derive its outlets from its own inlets and state alone. It MUST NOT read anything outside the graph during evaluation.

A base node whose behaviour advances over time MUST take that time as an inlet, so that the same inlets and state always produce the same outlets and the source of time is a connection the author can see and change.

#### Scenario: The same inputs give the same output
- **WHEN** a base node is evaluated twice with identical inlets and state
- **THEN** it produces identical outlets both times

#### Scenario: Time arrives on a connection
- **WHEN** a time-driven base node is evaluated
- **THEN** its notion of time came from an inlet
- **AND** nothing outside the graph was read

#### Scenario: Retiming is authored, not built in
- **WHEN** a time-driven base node's time inlet is connected to a different time source
- **THEN** the node follows that source with no change to the node kind

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
