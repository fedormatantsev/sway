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
