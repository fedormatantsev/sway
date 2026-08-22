## ADDED Requirements

### Requirement: An OnMidiNote node converts notes into pressed and released triggers
An `OnMidiNote` node MUST expose a note-event handle inlet, one authored string inlet `note`, and two Trigger handle outlets named `pressed` and `released`. It MUST NOT expose a channel inlet: which MIDI channel is heard is the producing `MidiNotes` node's concern.

`note` MUST be a pitch name in scientific pitch notation: a letter A through G, an optional accidental `#` or `b`, and an integer octave which MAY be negative. The letter MUST be matched case-insensitively. Surrounding whitespace MUST be ignored. MIDI note 60 MUST be `C4`. `D#1` and `Eb1` MUST name the same MIDI note.

On every tick the node MUST read the note occurrences named by its inlet and publish:
- one Trigger on `pressed` for each matching note-on, in arrival order
- one Trigger on `released` for each matching note-off, in arrival order

A matching occurrence is one whose MIDI note number equals the number the `note` string names. Occurrences that do not match MUST be ignored. A tick with no matching note-on MUST leave the empty handle on `pressed`; a tick with no matching note-off MUST leave the empty handle on `released`.

A `note` string that does not parse, or that names a MIDI number outside 0 through 127, MUST match nothing: both outlets hold the empty handle, and evaluation MUST succeed.

The node MUST NOT keep notes, batches or handles between ticks. It MUST NOT name a generic Trigger consumer; converting is this node's whole job.

An unconnected notes inlet, a missing arena, or a missing batch MUST leave both outlets at the empty handle rather than fail evaluation.

#### Scenario: A matching note-on fires pressed
- **WHEN** an `OnMidiNote` is authored with note `C4`
- **AND** its inlet names a note-on for MIDI note 60
- **THEN** `pressed` names a batch of one Trigger
- **AND** `released` holds the empty handle

#### Scenario: A matching note-off fires released
- **WHEN** an `OnMidiNote` is authored with note `C4`
- **AND** its inlet names a note-off for MIDI note 60
- **THEN** `released` names a batch of one Trigger
- **AND** `pressed` holds the empty handle

#### Scenario: Unmatched notes are ignored
- **WHEN** an `OnMidiNote` is authored with note `C4`
- **AND** its inlet names a note-on for MIDI note 64
- **THEN** both outlets hold the empty handle

#### Scenario: A sharp name matches that pitch
- **WHEN** an `OnMidiNote` is authored with note `D#1`
- **AND** its inlet names a note-on whose MIDI number is the scientific-pitch value of D♯1
- **THEN** `pressed` names a batch of one Trigger

#### Scenario: Two matching note-ons in one tick both fire
- **WHEN** two matching note-ons arrive in one batch
- **THEN** `pressed` names a batch of two Triggers, in that arrival order

#### Scenario: An unparseable note name is silent
- **WHEN** an `OnMidiNote` is authored with note `not-a-note`
- **AND** its inlet names any note-on
- **THEN** both outlets hold the empty handle
- **AND** evaluation succeeds

#### Scenario: An unconnected inlet is silent
- **WHEN** an `OnMidiNote` is evaluated with nothing connected to its notes inlet
- **THEN** both outlets hold the empty handle
- **AND** evaluation succeeds

### Requirement: OnMidiNote lives in the MIDI domain
Adding the MIDI domain's plugin MUST register `OnMidiNote` so it is authorable from the palette, loadable from a document, and evaluated with every other node. A host MUST NOT have to register the kind or its parts on the domain's behalf.

The node MUST live in the MIDI domain because it is the converter that reads the MIDI note payload. No other domain may be required to name that payload in order to react to a note: they consume Trigger.

#### Scenario: The palette offers OnMidiNote once the MIDI domain is added
- **WHEN** a host adds the MIDI domain plugin and nothing else from that domain
- **THEN** `OnMidiNote` appears in the palette
- **AND** a document that names an `OnMidiNote` node loads and evaluates

#### Scenario: No other domain names a MIDI note
- **WHEN** the dependencies and public items of a domain crate other than the MIDI one are inspected
- **THEN** none of them names the MIDI note payload

## MODIFIED Requirements

### Requirement: A MIDI notes node publishes the tick's note events
A `MidiNotes` node MUST expose one authored inlet `channel` and MUST publish, on every tick, the note-on and note-off messages that arrived during that tick **on that channel**, as one batch of occurrences named by a handle on its outlet. A tick in which no matching note message arrived MUST leave the empty handle on that outlet.

`channel` MUST address MIDI channels in the protocol numbering 0 through 15. Values outside that range MUST be clamped to it. Fractional inlet values MUST be truncated toward zero before clamping.

Each occurrence MUST carry the channel in the protocol's own 0–15 numbering, the note number, the velocity, whether it is a note-on or a note-off, and its offset in seconds from the start of the tick. A note-on whose velocity is zero MUST be published as a note-off.

Occurrences MUST be published in the order the messages arrived.

The node MUST NOT select among pitches: every matching-channel note message of the tick MUST be published, for every note number. Choosing which pitch matters is another node's concern, and the occurrence carries what that choice needs.

The node MUST NOT keep notes, batches or handles between ticks. A note message MUST NOT be published on a later tick than the one it arrived in.

#### Scenario: A note on and a note off are published in arrival order
- **WHEN** a note-on and then a note-off arrive during one tick on the node's channel
- **THEN** the node's outlet names a batch of those two occurrences, in that order
- **AND** each carries its channel, note number, velocity and offset within the tick

#### Scenario: A silent tick publishes nothing
- **WHEN** no note message arrives during a tick on the node's channel
- **THEN** the node's outlet holds the empty handle
- **AND** anything connected to it reads no occurrences

#### Scenario: A zero-velocity note on is a note off
- **WHEN** a note-on with velocity zero arrives on the node's channel
- **THEN** it is published as a note-off

#### Scenario: Every channel is published
- **WHEN** note messages arrive on two different channels during one tick
- **AND** two `MidiNotes` nodes are authored, one for each channel
- **THEN** each node publishes only the occurrences of its own channel
- **AND** each occurrence carries the channel it arrived on

#### Scenario: Notes do not survive their tick
- **WHEN** notes arrive on one tick and none on the next
- **THEN** the second tick's handle yields no occurrences
- **AND** the first tick's notes are not published again

#### Scenario: Channel is clamped
- **WHEN** a `MidiNotes` node's channel inlet is 20
- **AND** a note-on arrives on channel 15
- **THEN** that note is published
