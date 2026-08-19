## MODIFIED Requirements

### Requirement: A sprite material is a node wired to a mesh
A `SpriteMaterial` node MUST be a node of its own and MUST reach a scene node through a material connection, in the same manner as any other material node. It MUST NOT carry geometry or a placement of its own.

One sprite material MUST be connectable to more than one scene node, and every connected scene node MUST show the same frame, tint and opacity.

#### Scenario: A sprite material renders on a wired mesh
- **WHEN** a sprite material node is connected to a scene node
- **THEN** that scene node renders with the material's colour run

#### Scenario: A sprite material applies to any mesh
- **WHEN** a sprite material is connected to a scene node whose mesh is not a flat quad
- **THEN** that mesh renders with the material's colour run mapped through the mesh's own texture coordinates

#### Scenario: Sharing is visible in the graph
- **WHEN** one sprite material node is connected to two scene nodes
- **THEN** both render the same frame of the sequence

### Requirement: A sprite material takes its colour and depth runs from wires
A `SpriteMaterial` MUST receive a colour run and a depth run as two separate inlets, each connected to a frame sequence node. It MUST NOT name image paths of its own.

Neither connection carries the sequence itself: the frame sequence node owns its texture, and the material reaches it through the connection. Both inlets MUST accept a frame sequence node regardless of the colour space that sequence was authored with, so that one node kind serves either role.

The number of frames available MUST be the layer count of the connected sequences rather than an authored number. Where the two disagree in length, the system MUST report a diagnostic and use the shorter.

The two runs MUST NOT be required to share a resolution: both are addressed by normalized texture coordinates, and the depth run's useful resolution is bounded by the tessellation of the mesh it displaces rather than by the colour run.

A material whose runs are not both connected MUST render nothing rather than render incorrectly.

#### Scenario: Colour and depth arrive over separate wires
- **WHEN** two frame sequence nodes are connected to a sprite material's colour and depth inlets
- **THEN** the material samples colour from the first and displacement from the second

#### Scenario: The same sequence may serve either inlet
- **WHEN** a frame sequence node is connected to a colour inlet on one material and a depth inlet on another
- **THEN** both connections are legal and both materials render

#### Scenario: Disagreeing run lengths are reported
- **WHEN** a material's colour run has 30 layers and its depth run has 24
- **THEN** a diagnostic naming the material is reported
- **AND** the frame number is bounded by 24

#### Scenario: Runs of differing resolution are legal
- **WHEN** a material's colour run is 512×512 and its depth run is 64×64
- **THEN** both are sampled correctly and no diagnostic is reported

#### Scenario: An incomplete material renders nothing
- **WHEN** a sprite material has no depth run connected
- **THEN** nothing is drawn for the scene nodes it is connected to
