# runtime Specification

## Purpose

Defines render-side behaviour authored from the graph: how a sprite material binds a colour and a depth frame sequence, selects an animation frame, displaces the geometry it is applied to, and takes part in depth testing against meshes and other sprite layers.

## Requirements

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

### Requirement: A frame sequence node loads an ordered run of images as one texture
A `FrameSequence` node MUST load every image in an authored folder and publish them as a single layered texture, one layer per image, preserving order. It MUST expose the resulting layer count.

Order MUST be by filename, ascending, and MUST NOT depend on the order the filesystem or the asset system reports its contents.

The node MUST expose a colour space, so that the same node type can carry a colour run — interpreted with a display transfer curve — and a depth run, interpreted as data with no transfer curve applied.

Frames MUST be addressed by an integer layer index. Sampling one layer MUST NOT read texels of any other layer, at any filtering setting or magnification.

#### Scenario: Frames load in filename order
- **WHEN** a folder holds frames named `000.png` through `029.png`
- **THEN** the published texture has 30 layers
- **AND** layer 3 holds the contents of `003.png`

#### Scenario: Order does not depend on enumeration order
- **WHEN** the asset system reports a folder's contents in an arbitrary order
- **THEN** the layer order is still ascending by filename

#### Scenario: A depth run is read without a colour transfer curve
- **WHEN** a frame sequence is authored as a data run and a frame holds a mid-range value
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


### Requirement: The frame number selects a layer and is clamped into range
A `SpriteMaterial` MUST expose a `frame` number as a scalar inlet.

The layer shown MUST be the frame number truncated toward negative infinity and clamped into the range `[0, layers)`, where `layers` is the layer count of the wired frame sequences. Clamping MUST be applied where the frame number is read, so that an authored frame number and a frame number arriving over a wire select the same layer for the same value.

Clamping is a safeguard against sampling outside the sequence and nothing more. The read side MUST NOT loop, reverse, or otherwise reinterpret an out-of-range frame number: looping, ping-pong and hold-at-end are animation policy and MUST be expressed in the graph, where they are visible and interchangeable.

No blending between adjacent layers MUST occur; a fractional frame number MUST select exactly one layer.

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
- **WHEN** a wire delivers the frame number `37.5` to a sprite material on a sequence of 30 frames
- **THEN** the same layer is shown as when `37.5` is authored directly

#### Scenario: Looping is the graph's decision, not the material's
- **WHEN** a graph drives the frame number with a periodic ramp over `[0, layers)`
- **THEN** the material cycles through every layer and returns to the first
- **AND** replacing that ramp with a different periodic shape changes the playback order without any change to the material

### Requirement: The depth run displaces the geometry it is applied to
The depth run's selected layer MUST displace the mesh's vertices along the mesh's own normals, scaled by an authorable depth range, and offset so that a chosen pivot value in the run leaves a vertex undisplaced.

Because the geometry is moved rather than the depth value alone, the relief MUST exhibit parallax: rotating the mesh relative to the camera MUST shift near parts of the relief further across the image than far parts.

The mesh's visible bounds MUST account for the displacement, so that a mesh whose displaced geometry is on screen is not culled.

#### Scenario: Rotating the mesh reveals parallax
- **WHEN** a mesh carrying a sprite material with a non-flat depth run is rotated relative to the camera
- **THEN** parts of the relief nearer the camera shift further across the image than parts further from it

#### Scenario: The pivot value leaves geometry undisplaced
- **WHEN** a region of the depth run's current layer holds exactly the pivot value
- **THEN** the vertices in that region sit on the mesh's undisplaced surface

#### Scenario: Displaced geometry is not culled
- **WHEN** a mesh's undisplaced bounds are off screen but its displaced geometry is on screen
- **THEN** the mesh is drawn

#### Scenario: Denser geometry resolves the relief more finely
- **WHEN** the same sprite material is applied to two quads of different subdivision counts
- **THEN** the quad with more subdivisions reproduces the depth run's relief more closely

### Requirement: Sprite layers occlude and interpenetrate by depth
A sprite material MUST be alpha-blended and MUST write depth. A sprite layer MUST therefore interpenetrate opaque meshes and other sprite layers, being visible where its geometry is nearer the camera and hidden where it is further, rather than sitting wholly in front of or behind them.

Fragments below an alpha threshold MUST be discarded so that fully transparent regions of a run neither shade nor occlude.

#### Scenario: A sprite interpenetrates an opaque mesh
- **WHEN** a sprite layer's displaced geometry passes through an opaque mesh
- **THEN** the part of the sprite nearer the camera than the mesh is visible
- **AND** the part further away is hidden by the mesh

#### Scenario: Two sprite layers interpenetrate each other
- **WHEN** two sprite layers' displaced geometry overlap on screen and interleave in depth
- **THEN** each layer is visible where its geometry is nearer the camera

#### Scenario: Transparent regions do not occlude
- **WHEN** a region of a sprite's colour run is fully transparent
- **THEN** whatever lies behind that region is drawn unobstructed

### Requirement: Tint and opacity are inlets
A `SpriteMaterial` MUST expose a tint as a three-component colour inlet and an opacity as a scalar inlet, both drivable by wires and both editable directly.

#### Scenario: Tint multiplies the colour run
- **WHEN** a tint is applied to a sprite material
- **THEN** the rendered colour is the colour run's value scaled by that tint

#### Scenario: Opacity scales the run's alpha
- **WHEN** opacity is reduced on a sprite material
- **THEN** the layer blends more weakly with what is behind it

### Requirement: Every camera renders into a target of its own
Each camera in the world MUST render into a render target sized by that camera's authored resolution. Two cameras MUST NOT share one target, and no camera's target may be resized by the window, by an editor pane, or by another camera being added or removed.

A camera whose target cannot be produced — because its resolution has a zero component, or because the device refuses a target that large — MUST render nothing and MUST be reported once, rather than falling back to a target of some other size.

Changing a camera's authored resolution MUST replace that camera's target with one of the new size, and everything reading that camera — what is presented, what is previewed, what is captured — MUST see the new size from then on without the project being reopened.

The editor's own camera has no authored resolution and is excluded from this requirement: it takes its size from the pane it is drawn into.

#### Scenario: Adding a camera does not disturb an existing one
- **WHEN** a second camera is added to a document while the first is being presented
- **THEN** the first camera's target keeps its size and contents
- **AND** the second renders into a target of its own authored size

#### Scenario: A resolution edit resizes only that camera's target
- **WHEN** one camera's resolution is edited from 1920×1080 to 1280×720
- **THEN** that camera renders at 1280×720 from the next frame
- **AND** no other camera's target changes

#### Scenario: An impossible target renders nothing
- **WHEN** a camera's authored resolution exceeds what the device can allocate
- **THEN** that camera renders nothing
- **AND** a diagnostic naming the camera and the limit is reported once
- **AND** every other camera still renders

### Requirement: A capture writes on a fixed cadence
A capture MUST write files at a fixed rate — a whole number of frames per second of the show's own time, currently 60. That rate MUST NOT be the graph's tick rate, and MUST NOT be the rate at which frames happen to be rendered.

Show time is wall time: the show follows an external clock running in real time, and a capture MUST follow that same clock. A capture slot occurs once every fixed interval of it, so one second of wall time holds the capture rate's worth of slots whatever the render loop is doing.

The show renders at a fixed rate of its own, whether or not anything is capturing (see the `app` capability), and the capture rate is currently that same 60 — so each slot ordinarily holds a distinct, newly rendered frame. The two rates MUST remain separately stated: they are separate concerns, and changing one MUST NOT silently change the other.

The recording flag is a graph value and therefore changes at the tick rate. Whether it changed zero, one or several times between two capture slots MUST NOT change how many files those slots produce.

#### Scenario: The tick rate does not set the file rate
- **WHEN** the graph ticks at 120 Hz with recording true for one second of show time
- **THEN** about 60 files are written, not about 120

#### Scenario: Each slot holds its own frame
- **WHEN** a capture records for one second while the show renders at its fixed rate
- **THEN** about 60 files are written
- **AND** each holds a distinct frame rather than a repeat of the one before

#### Scenario: Starting a run does not change the frame rate
- **WHEN** a capture node's recording flag becomes true
- **THEN** the show goes on rendering at the same fixed rate it rendered at before

#### Scenario: A loop that cannot reach the rate still writes the rate
- **WHEN** frames can only be rendered at 45 Hz with recording true for one second
- **THEN** about 60 files are written, not about 45

### Requirement: Capture never delays the show
Recording MUST NOT slow the frame loop, delay a graph tick, or make the system fall behind the external clock. Keeping up with that clock takes priority over completing a capture.

Where frames cannot be read back, encoded or written at the capture rate, capture slots MUST be dropped and the drop reported. Neither the show's frame rate nor its tick may fall in order to preserve a slot.

A run MUST report how many slots it dropped when it ends, so that a recording known to be incomplete is not mistaken for a complete one.

#### Scenario: A slow disk does not slow the show
- **WHEN** files cannot be written as fast as the capture rate produces them
- **THEN** the frame loop keeps pace with the external clock
- **AND** slots are dropped rather than the show waiting for them

#### Scenario: A finished run reports what it lost
- **WHEN** a run ends after dropping slots
- **THEN** a diagnostic naming the capture node and the number of slots dropped is reported

### Requirement: A capture's numbering is a timeline
Each file's number MUST be its capture slot's index within the run, counted from zero at the run's start. The sequence played back at the capture rate MUST therefore match the show's own timing.

A slot for which no new frame was rendered MUST repeat the most recently rendered frame, so that a render rate below the capture rate costs duplicate images rather than distorted timing. This is the exception rather than the ordinary case: it arises only when the scene cannot be rendered at the show's fixed rate at all.

A dropped slot MUST leave its number unused rather than shifting the frames after it, because renumbering would move every later frame earlier in time.

#### Scenario: A slow render rate repeats rather than reslots
- **WHEN** frames are rendered at 30 Hz while capturing at 60
- **THEN** each rendered frame appears in about two consecutively numbered files
- **AND** one second of show time still spans about 60 numbers

#### Scenario: A dropped slot leaves a hole
- **WHEN** the slot numbered 40 is dropped
- **THEN** no file numbered 40 exists
- **AND** the next file written is numbered 41

#### Scenario: Playback matches the show
- **WHEN** a run recorded over ten seconds of show time is played back at the capture rate
- **THEN** it lasts about ten seconds
- **AND** what happened at a given moment of the show appears at that moment of playback

### Requirement: Capturing does not change what is rendered or presented
Reading a camera's target back in order to write it MUST NOT change the image that camera renders, and MUST NOT change what is presented to the window.

A camera that is captured and presented at the same time MUST show the same image in both places. Whether a camera is being captured MUST NOT be visible in the presented image.

#### Scenario: The presented image is unaffected by capture
- **WHEN** the camera wired to the output node is also wired to a recording capture node
- **THEN** the presented image is the same as it is when the capture node is not recording

#### Scenario: A captured camera is not tinted, flipped or rescaled by being captured
- **WHEN** a frame is written for a camera
- **THEN** the written image is that camera's rendered frame at its authored resolution, with the same orientation and colours it is presented with
