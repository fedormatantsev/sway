## Purpose

Defines render-side behaviour authored from the graph: how a sprite material binds
a colour and a depth frame sequence, selects an animation frame, displaces the
geometry it is applied to, and takes part in depth testing against meshes and other
sprite layers.

## ADDED Requirements

### Requirement: A sprite material is a node wired to a mesh

A `SpriteMaterial` node SHALL be authorable on its own entity and SHALL reach a
mesh through a material wire, in the same manner as any other material node. It
SHALL NOT carry geometry of its own.

One sprite material SHALL be connectable to more than one mesh, and every connected
mesh SHALL show the same frame, tint and opacity.

#### Scenario: A sprite material renders on a wired mesh

- **WHEN** a sprite material node is connected to a mesh entity
- **THEN** that mesh renders with the material's colour run

#### Scenario: A sprite material applies to any mesh

- **WHEN** a sprite material is connected to a mesh that is not a flat quad
- **THEN** that mesh renders with the material's colour run mapped through
  the mesh's own texture coordinates

#### Scenario: Sharing is visible in the graph

- **WHEN** one sprite material node is connected to two mesh entities
- **THEN** both meshes render the same frame of the sequence

### Requirement: A frame sequence node loads an ordered run of images as one texture

A `FrameSequence` node SHALL load every image in an authored folder and publish them
as a single layered texture, one layer per image, preserving order. It SHALL expose
the resulting layer count.

Order SHALL be by filename, ascending, and SHALL NOT depend on the order the
filesystem or the asset system reports its contents.

The node SHALL expose a colour space, so that the same node type can carry a colour
run — interpreted with a display transfer curve — and a depth run, interpreted as
data with no transfer curve applied.

Frames SHALL be addressed by an integer layer index. Sampling one layer SHALL NOT
read texels of any other layer, at any filtering setting or magnification.

#### Scenario: Frames load in filename order

- **WHEN** a folder holds frames named `000.png` through `029.png`
- **THEN** the published texture has 30 layers
- **AND** layer 3 holds the contents of `003.png`

#### Scenario: Order does not depend on enumeration order

- **WHEN** the asset system reports a folder's contents in an arbitrary order
- **THEN** the layer order is still ascending by filename

#### Scenario: A depth run is read without a colour transfer curve

- **WHEN** a frame sequence is authored as a data run and a frame holds a mid-range
  value
- **THEN** the value read from that layer is proportional to the value stored

#### Scenario: One sequence serves many consumers

- **WHEN** one frame sequence node is connected to two sprite materials
- **THEN** both sample the same texture and no second copy is loaded

#### Scenario: Frames of differing dimensions are rejected

- **WHEN** a folder holds images that are not all the same dimensions
- **THEN** a diagnostic naming the folder is reported
- **AND** no texture is published

#### Scenario: An oversized sequence is reported

- **WHEN** a folder holds more frames than the device's maximum texture array layers
- **THEN** a diagnostic naming the folder and the limit is reported
- **AND** no texture is published, rather than a silently truncated one

#### Scenario: A sequence is unavailable until every frame has loaded

- **WHEN** some frames of a sequence have loaded and others have not
- **THEN** no texture is published
- **AND** the sequence publishes one once the remaining frames arrive

### Requirement: A sprite material takes its colour and depth runs from wires

A `SpriteMaterial` SHALL receive a colour run and a depth run as separate inlets,
each fed by a frame sequence. It SHALL NOT name image paths of its own.

The number of frames available SHALL be the layer count of the wired sequences
rather than an authored number. Where the two disagree in length, the system SHALL
report a diagnostic and use the shorter.

The two runs SHALL NOT be required to share a resolution: both are addressed by
normalized texture coordinates, and the depth run's useful resolution is bounded by
the tessellation of the mesh it displaces rather than by the colour run.

A material whose runs are not both available SHALL render nothing rather than render
incorrectly.

#### Scenario: Colour and depth arrive over separate wires

- **WHEN** two frame sequences are connected to a sprite material's colour and depth
  inlets
- **THEN** the material samples colour from the first and displacement from the
  second

#### Scenario: The same sequence may serve either inlet

- **WHEN** a frame sequence is connected to a colour inlet on one material and a
  depth inlet on another
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
- **THEN** nothing is drawn for the meshes it is wired to

### Requirement: The frame number selects a layer and is clamped into range

A `SpriteMaterial` SHALL expose a `frame` number as a scalar inlet.

The layer shown SHALL be the frame number truncated toward negative infinity and
clamped into the range `[0, layers)`, where `layers` is the layer count of the wired
frame sequences. Clamping SHALL be applied where the frame number is read, so that
an authored frame number and a frame number arriving over a wire select the same
layer for the same value.

Clamping is a safeguard against sampling outside the sequence and nothing more. The
read side SHALL NOT loop, reverse, or otherwise reinterpret an out-of-range frame
number: looping, ping-pong and hold-at-end are animation policy and SHALL be
expressed in the graph, where they are visible and interchangeable.

No blending between adjacent layers SHALL occur; a fractional frame number SHALL
select exactly one layer.

#### Scenario: Fractional frame numbers select one layer

- **WHEN** the frame number is `3.7` on a sequence of 30 frames
- **THEN** layer 3 is shown, unblended with layer 4

#### Scenario: Frame numbers past the end clamp to the last layer

- **WHEN** the frame number is `37.5` on a sequence of 30 frames
- **THEN** layer 29 is shown

#### Scenario: Negative frame numbers clamp to the first layer

- **WHEN** the frame number is `-1.0` on a sequence of 30 frames
- **THEN** layer 0 is shown

#### Scenario: A wired frame number is clamped identically

- **WHEN** a wire delivers the frame number `37.5` to a sprite material on a
  sequence of 30 frames
- **THEN** the same layer is shown as when `37.5` is authored directly

#### Scenario: Looping is the graph's decision, not the material's

- **WHEN** a graph drives the frame number with a periodic ramp over `[0, layers)`
- **THEN** the material cycles through every layer and returns to the first
- **AND** replacing that ramp with a different periodic shape changes the playback
  order without any change to the material

### Requirement: The depth run displaces the geometry it is applied to

The depth run's selected layer SHALL displace the mesh's vertices along the mesh's own normals,
scaled by an authorable depth range, and offset so that a chosen pivot value in the
run leaves a vertex undisplaced.

Because the geometry is moved rather than the depth value alone, the relief SHALL
exhibit parallax: rotating the mesh relative to the camera SHALL shift near parts of
the relief further across the image than far parts.

The mesh's visible bounds SHALL account for the displacement, so that a mesh whose
displaced geometry is on screen is not culled.

#### Scenario: Rotating the mesh reveals parallax

- **WHEN** a mesh carrying a sprite material with a non-flat depth run is rotated
  relative to the camera
- **THEN** parts of the relief nearer the camera shift further across the image than
  parts further from it

#### Scenario: The pivot value leaves geometry undisplaced

- **WHEN** a region of the depth run's current layer holds exactly the pivot value
- **THEN** the vertices in that region sit on the mesh's undisplaced surface

#### Scenario: Displaced geometry is not culled

- **WHEN** a mesh's undisplaced bounds are off screen but its displaced geometry is
  on screen
- **THEN** the mesh is drawn

#### Scenario: Denser geometry resolves the relief more finely

- **WHEN** the same sprite material is applied to two quads of different subdivision
  counts
- **THEN** the quad with more subdivisions reproduces the depth run's relief more
  closely

### Requirement: Sprite layers occlude and interpenetrate by depth

A sprite material SHALL be alpha-blended and SHALL write depth. A sprite layer
SHALL therefore interpenetrate opaque meshes and other sprite layers, being visible
where its geometry is nearer the camera and hidden where it is further, rather than
sitting wholly in front of or behind them.

Fragments below an alpha threshold SHALL be discarded so that fully transparent
regions of a run neither shade nor occlude.

#### Scenario: A sprite interpenetrates an opaque mesh

- **WHEN** a sprite layer's displaced geometry passes through an opaque mesh
- **THEN** the part of the sprite nearer the camera than the mesh is visible
- **AND** the part further away is hidden by the mesh

#### Scenario: Two sprite layers interpenetrate each other

- **WHEN** two sprite layers' displaced geometry overlap on screen and interleave in
  depth
- **THEN** each layer is visible where its geometry is nearer the camera

#### Scenario: Transparent regions do not occlude

- **WHEN** a region of a sprite's colour run is fully transparent
- **THEN** whatever lies behind that region is drawn unobstructed

### Requirement: Tint and opacity are inlets

A `SpriteMaterial` SHALL expose a tint as a three-component colour inlet and an
opacity as a scalar inlet, both drivable by wires and both editable directly.

#### Scenario: Tint multiplies the colour run

- **WHEN** a tint is applied to a sprite material
- **THEN** the rendered colour is the colour run's value scaled by that tint

#### Scenario: Opacity scales the run's alpha

- **WHEN** opacity is reduced on a sprite material
- **THEN** the layer blends more weakly with what is behind it
