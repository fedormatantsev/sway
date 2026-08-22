## ADDED Requirements

### Requirement: A MIDI notes node publishes the tick's note events
A `MidiNotes` node MUST publish, on every tick, the note-on and note-off messages that arrived during that tick, as one batch of occurrences named by a handle on its outlet. A tick in which no note message arrived MUST leave the empty handle on that outlet.

Each occurrence MUST carry the channel in the protocol's own 0–15 numbering, the note number, the velocity, whether it is a note-on or a note-off, and its offset in seconds from the start of the tick. A note-on whose velocity is zero MUST be published as a note-off.

Occurrences MUST be published in the order the messages arrived.

The node MUST NOT select among the messages: every note message of the tick MUST be published, on every channel and for every note number. Choosing which of them matters is another node's concern, and the occurrence carries what that choice needs.

The node MUST NOT keep notes, batches or handles between ticks. A note message MUST NOT be published on a later tick than the one it arrived in.

#### Scenario: A note on and a note off are published in arrival order
- **WHEN** a note-on and then a note-off arrive during one tick
- **THEN** the node's outlet names a batch of those two occurrences, in that order
- **AND** each carries its channel, note number, velocity and offset within the tick

#### Scenario: A silent tick publishes nothing
- **WHEN** no note message arrives during a tick
- **THEN** the node's outlet holds the empty handle
- **AND** anything connected to it reads no occurrences

#### Scenario: A zero-velocity note on is a note off
- **WHEN** a note-on with velocity zero arrives
- **THEN** it is published as a note-off

#### Scenario: Every channel is published
- **WHEN** note messages arrive on two different channels during one tick
- **THEN** both are published
- **AND** each carries the channel it arrived on

#### Scenario: Notes do not survive their tick
- **WHEN** notes arrive on one tick and none on the next
- **THEN** the second tick's handle yields no occurrences
- **AND** the first tick's notes are not published again

### Requirement: MIDI note events live in the MIDI domain
Adding the MIDI domain's plugin MUST register the notes node, the occurrence payload and its handle, so the node is authorable from the palette, loadable from a document, and evaluated with every other node. A host MUST NOT have to register the payload or the handle on the domain's behalf.

The note payload MUST be the MIDI domain's own vocabulary. A node kind that turns note occurrences into a vocabulary another domain understands MUST itself live in the MIDI domain; no other domain may be required to name a MIDI note type in order to react to a note.

`MidiNotes` MUST read the live MIDI messages during its own evaluation, as the other MIDI nodes do. It MUST NOT require a pre-tick injection into the graph, and the graph engine MUST NOT name a MIDI type to support it.

Absence of MIDI input MUST leave the outlet at the empty handle rather than fail evaluation.

#### Scenario: One plugin is the whole domain
- **WHEN** a host adds the MIDI domain's plugin and nothing else
- **THEN** the notes node is in the palette, loads from a document, and publishes on every tick

#### Scenario: No other domain names a MIDI note
- **WHEN** the dependencies and public items of a domain crate other than the MIDI one are inspected
- **THEN** none of them names the MIDI note payload

#### Scenario: No MIDI input is the empty handle
- **WHEN** a notes node is evaluated with no MIDI input present at all
- **THEN** its evaluation succeeds
- **AND** its outlet holds the empty handle
