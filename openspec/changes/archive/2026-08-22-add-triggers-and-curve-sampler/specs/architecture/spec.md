## MODIFIED Requirements

### Requirement: Dependencies point from host to domain to engine
Dependencies MUST run one way: the engine depends on no crate of this project; a domain crate depends on the engine; the host depends on domain crates.

The base-nodes crate is the generic signal layer. Other node domains MAY depend on it for shared vocabulary — in particular the Trigger occurrence payload. Peer node domains MUST NOT depend on each other.

Where two peer crates need to share a vocabulary and neither may depend on the other, that vocabulary MUST live in a crate of its own rather than being parked in the engine. Generic signal vocabulary MUST live in the base-nodes crate.

A declared dependency that the crate does not use MUST be removed.

#### Scenario: A converter depends on the generic layer, not the other way around
- **WHEN** the MIDI domain publishes Trigger occurrences that a base node reads
- **THEN** Trigger lives in the base-nodes crate
- **AND** the MIDI crate depends on the base-nodes crate
- **AND** the base-nodes crate does not depend on the MIDI crate

#### Scenario: Peer domains still do not depend on each other
- **WHEN** the MIDI domain and the runtime domain are inspected
- **THEN** neither crate depends on the other

#### Scenario: Two domains share a vocabulary without depending on each other
- **WHEN** a producing crate and a consuming crate need the same event or value vocabulary and neither is the base-nodes crate
- **THEN** that vocabulary lives in a crate both depend on
- **AND** neither the engine nor either domain crate is that crate's owner by default

#### Scenario: Declared dependencies are real
- **WHEN** a crate's manifest is compared against its source
- **THEN** every declared dependency is referenced by that source
