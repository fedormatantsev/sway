## ADDED Requirements

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

## MODIFIED Requirements

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

## REMOVED Requirements

### Requirement: A material wire supplies the material component it writes into
**Reason**: A connection no longer inserts anything. The material node applies its own material to the scene nodes connected to it, which is what keeps the material kind known only to the material node. Superseded by `A material node attaches itself to what it is connected to`.

**Migration**: Remove the per-wire hooks that supplied and withdrew a typed material component on connect and disconnect. Each material node kind takes responsibility for applying and removing its own material.
