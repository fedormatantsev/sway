## Purpose

Lets the graph read live MIDI Control Change as held 0–1 parameters that other nodes can consume.

## ADDED Requirements

### Requirement: A MIDI CC node publishes a held parameter
A `MidiCc` node MUST expose two authored inlets, `channel` and `cc`, and one outlet that is the last Control Change matching those inlets, held until a later matching message replaces it.

`channel` MUST address MIDI channels in the protocol numbering 0 through 15. `cc` MUST address controller numbers 0 through 127. Values outside those ranges MUST be clamped to them. Fractional inlet values MUST be truncated toward zero before clamping.

The outlet MUST be a scalar in 0 through 1, mapping MIDI value 0 to 0 and MIDI value 127 to 1. Until a matching Control Change has been received in the current session, the outlet MUST be 0.

A matching message is a Control Change whose channel and controller number equal the node's inlets after truncation and clamping. When several matching messages arrive before the node next evaluates, the last of them MUST be the held value.

#### Scenario: A matching CC becomes the outlet
- **WHEN** a `MidiCc` node is authored with channel 0 and cc 1
- **AND** a Control Change on channel 0, controller 1, value 127 is received
- **THEN** the node's outlet is 1

#### Scenario: The value is held between messages
- **WHEN** a matching Control Change of value 64 has been received
- **AND** no later matching Control Change has been received
- **THEN** subsequent evaluations keep publishing 64 / 127
- **AND** unmatched Control Changes do not change the outlet

#### Scenario: The last matching message in a burst wins
- **WHEN** two matching Control Changes arrive before the next evaluation, values 10 then 20
- **THEN** the outlet is 20 / 127

#### Scenario: Nothing received yet is zero
- **WHEN** a `MidiCc` node is evaluated and no matching Control Change has been received in the session
- **THEN** its outlet is 0

#### Scenario: Channel and controller number are clamped
- **WHEN** a `MidiCc` node's channel inlet is 20 and its cc inlet is 200
- **THEN** it matches Control Changes on channel 15, controller 127

#### Scenario: A new node sees the session's last matching value
- **WHEN** a Control Change on channel 0, controller 1, value 127 has already been received
- **AND** a new `MidiCc` node is then authored with channel 0 and cc 1
- **THEN** its first evaluation publishes 1

### Requirement: MIDI CC nodes live in the MIDI domain
Adding the MIDI domain's plugin MUST register `MidiCc` so it is authorable from the palette, loadable from a document, and evaluated with every other node. A host MUST NOT have to register the kind, its parts, or any CC snapshot on the domain's behalf.

`MidiCc` MUST read the live MIDI snapshot during its own evaluation. It MUST NOT require a pre-tick injection into the graph, and the graph engine MUST NOT name a MIDI type to support it.

Absence of MIDI input MUST leave the outlet at 0 rather than fail evaluation.

#### Scenario: The palette offers MidiCc once the MIDI domain is added
- **WHEN** a host adds the MIDI domain plugin and nothing else from that domain
- **THEN** `MidiCc` appears in the palette
- **AND** a document that names a `MidiCc` node loads and evaluates

#### Scenario: No MIDI input is a zero outlet
- **WHEN** a `MidiCc` node is evaluated with no MIDI input present
- **THEN** its outlet is 0
- **AND** evaluation completes

#### Scenario: Two nodes on the same controller agree
- **WHEN** two `MidiCc` nodes are authored with the same channel and cc
- **AND** a matching Control Change is received
- **THEN** both outlets equal the same 0–1 value
