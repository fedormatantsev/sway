## MODIFIED Requirements

### Requirement: Occurrences are one crate with one plugin
Everything occurrences need — the handle, the arena that holds the batches, the way a payload type is made known, and the emptying that happens before every tick — MUST live in a crate of its own that the graph engine does not depend on. Adding that crate's single plugin MUST be all a host does for the arena to exist and be emptied before every tick: a host MUST NOT have to register a system, a set, or a resource on the crate's behalf, and MUST NOT have to order the emptying against the tick itself.

A node domain that publishes or reads occurrences MUST depend on that crate for the mechanism. The occurrence crate MUST NOT depend on any node domain.

A domain that converts into the generic Trigger vocabulary MAY also depend on the base-nodes crate that owns that payload. Peer domains that are not that generic layer MUST NOT depend on each other in order to exchange occurrences.

#### Scenario: One plugin is the whole mechanism
- **WHEN** a host adds the occurrence plugin and nothing else from that crate
- **THEN** the arena exists and is emptied before each tick

#### Scenario: The engine names no occurrence
- **WHEN** the graph engine's dependencies and public items are inspected
- **THEN** none of them names a handle, an occurrence, a payload type or the arena

#### Scenario: Two domains exchange occurrences without depending on each other
- **WHEN** one peer node domain publishes occurrences of a payload type and another peer domain reads them
- **THEN** both depend on the occurrence crate
- **AND** neither depends on the other

#### Scenario: A converter fires the generic trigger payload
- **WHEN** the MIDI domain publishes Trigger occurrences that a base node reads
- **THEN** both depend on the occurrence crate
- **AND** the MIDI domain depends on the base-nodes crate for the payload
- **AND** the base-nodes crate does not depend on the MIDI domain
