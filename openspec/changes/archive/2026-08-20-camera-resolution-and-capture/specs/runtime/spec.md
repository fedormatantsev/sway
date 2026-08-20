## ADDED Requirements

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
