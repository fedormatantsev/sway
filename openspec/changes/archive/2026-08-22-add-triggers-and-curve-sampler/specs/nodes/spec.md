## ADDED Requirements

### Requirement: Trigger is the generic occurrence payload
The base node set MUST own a **Trigger** payload: a unit occurrence that means something happened, and carries nothing else.

A node kind in any domain MUST be able to publish and read Trigger occurrences through ordinary handle inlets and outlets. Adding Trigger MUST NOT require a change to the graph engine or to the occurrence crate's public surface.

#### Scenario: A trigger connection is payload-typed
- **WHEN** a Trigger outlet handle is connected to a Trigger inlet handle
- **THEN** the connection is made
- **AND** a connection to a handle of any other payload type is refused

#### Scenario: A silent tick carries no trigger
- **WHEN** a producer has no Trigger to publish on a tick
- **THEN** its outlet holds the empty handle
- **AND** every consumer of that outlet reads no occurrences

### Requirement: A Timer accumulates time and resets on trigger
A `Timer` node MUST expose two inlets — `time` as a scalar, and `trigger` as a Trigger handle — and one scalar outlet.

The outlet MUST be the elapsed time, in the `time` inlet's own units, since the most recent Trigger occurrence on `trigger`, or since the node began if it has never been triggered. A Trigger occurrence on a tick MUST set the outlet to zero on that same tick.

`time` MUST be an ordinary scalar inlet so a MIDI transport node can drive it. An unconnected `trigger` MUST never reset the timer. Several Trigger sources MUST be connectable to `trigger` as a many-connection inlet, and any occurrence on any of those handles MUST reset the timer.

Absence of the occurrence arena MUST leave the timer accumulating, never resetting, rather than fail evaluation.

#### Scenario: Time since start with no trigger
- **WHEN** a Timer's `time` inlet is driven from 0 to 4 with nothing connected to `trigger`
- **THEN** the outlet equals 4
- **AND** the node was not reset

#### Scenario: A trigger zeros elapsed time
- **WHEN** a Timer has accumulated 2 against its `time` inlet
- **AND** a Trigger occurrence arrives on `trigger`
- **THEN** that tick's outlet is 0
- **AND** further time advances from that tick's `time` value

#### Scenario: MidiTime can drive a Timer
- **WHEN** a Timer's `time` inlet is connected to a MIDI transport time outlet
- **THEN** the Timer's outlet advances in that transport's units
- **AND** a Trigger still resets it to zero

#### Scenario: Several trigger sources all reset
- **WHEN** two Trigger outlets are connected to one Timer's `trigger` inlet
- **AND** either source publishes an occurrence
- **THEN** the Timer resets

#### Scenario: No arena does not fail
- **WHEN** a Timer is evaluated with no occurrence arena present
- **THEN** evaluation succeeds
- **AND** the outlet keeps accumulating against `time`

### Requirement: A CurveSampler samples a curve at a time
A `CurveSampler` node MUST expose a scalar `time` inlet, an authored list of piecewise keys, and a scalar outlet that is those keys sampled at that time.

The node MUST NOT wrap time. It MUST clamp `time` to the keys' minimum and maximum `x` values, then linearly interpolate between adjacent keys sorted by `x`. An empty key list MUST yield outlet 0. A single key MUST yield that key's `y` for every time.

The node MUST be a pure function of its inlets: the same inlets MUST produce the same outlet, and it MUST NOT keep sample position in state. It MUST NOT expose `period`, `phase`, `amplitude`, or a named waveform shape.

#### Scenario: A piecewise envelope is sampled
- **WHEN** a CurveSampler is authored with keys from (0, 0) to (1, 1) and time 0.5
- **THEN** its outlet is 0.5

#### Scenario: Time is clamped, not wrapped
- **WHEN** a CurveSampler is authored with keys from (0, 0) to (1, 1) and time 2
- **THEN** its outlet is 1
- **AND** at time −1 the outlet is 0

#### Scenario: Driven time reaches the outlet in one tick
- **WHEN** an upstream scalar outlet is connected to a CurveSampler's `time` inlet
- **THEN** that tick's CurveSampler outlet reflects the upstream value sampled on the keys

#### Scenario: An envelope is a Timer into a CurveSampler
- **WHEN** a Timer that resets on Trigger drives a CurveSampler whose curve is piecewise
- **THEN** the CurveSampler walks that curve from the start on each Trigger
- **AND** time past the last key holds the last key's value
- **AND** no separate envelope node kind is required

## MODIFIED Requirements

### Requirement: A base node is a pure function of its inlets and state
A node kind in the base set MUST derive its outlets from its own inlets and state alone. It MUST NOT read a clock, a MIDI snapshot, or any other world resource during evaluation, other than the occurrence arena used to resolve a handle inlet.

Resolving a handle inlet through the occurrence arena is reading that inlet, not reaching outside the graph. A base node whose inlet is a handle MUST treat a missing arena as no occurrences, and MUST still evaluate.

A base node whose behaviour advances over time MUST take that time as an inlet, so that the same inlets and state always produce the same outlets and the source of time is a connection the author can see and change.

#### Scenario: The same inputs give the same output
- **WHEN** a base node is evaluated twice with identical inlets and state
- **THEN** it produces identical outlets both times

#### Scenario: Time arrives on a connection
- **WHEN** a time-driven base node is evaluated
- **THEN** its notion of time came from an inlet
- **AND** no clock or MIDI snapshot was read

#### Scenario: Retiming is authored, not built in
- **WHEN** a time-driven base node's time inlet is connected to a different time source
- **THEN** the node follows that source with no change to the node kind

#### Scenario: A handle inlet is resolved, not stored
- **WHEN** a base node has a Trigger handle on an inlet
- **THEN** it reads the occurrences that handle names from the arena during evaluation
- **AND** it keeps neither the occurrences nor the handle in its state
