## Purpose

Defines how a project document names value wires and which components it stores: full reflected type paths as wire keys, inlets only in the component map, version compatibility.

## ADDED Requirements

### Requirement: Wire keys are full type paths
Each entry in an entity's `wires` map MUST use the wire type's full reflected type path as the key and the producer entity's document id as the value. Component map keys MUST remain short registered names.

#### Scenario: Emit writes type paths
- **WHEN** a world has a `TranslationFrom` from `vec3A` to `cubeA` and a `ChildOf` from `cubeA` to `group`
- **THEN** the emitted entity `cubeA` has wire keys `sway_nodes::spatial::TranslationFrom` and `bevy_ecs::hierarchy::ChildOf`

#### Scenario: Apply resolves a type path
- **WHEN** a version-2 document names `"sway_nodes::osc::TimeFrom": "midiTime"` on an oscillator entity
- **AND** that type is registered as a reflected wire
- **THEN** apply inserts that relationship from the oscillator to the entity whose document id is `midiTime`

### Requirement: Format version 2
The supported project format version MUST be `2`. A document whose version is not `2` MUST be rejected as a whole parse error, not partially applied.

#### Scenario: Version 1 is refused
- **WHEN** a file declares `version: 1`
- **THEN** parse fails with an unsupported-version error
- **AND** the world is not mutated by apply

#### Scenario: Version 2 loads
- **WHEN** a well-formed file declares `version: 2` with type-path wire keys
- **THEN** parse succeeds

### Requirement: Only inlets are document components
The entity `components` map MUST contain authored inlet (and other authorable) components only. State and outlet components MUST NOT appear in a document. Apply MUST restore inlets; it MUST NOT require state or outlets to be present in the file. Runtime may seed outlet components so wires have a source; the first tick fills them.

#### Scenario: Emit omits outlets
- **WHEN** an entity has authorable inlets and a runtime outlet component
- **THEN** the emitted `components` map includes the inlets and does not include the outlet

#### Scenario: Load without outlets still ticks
- **WHEN** a version-2 document names only inlets on an entity that has a behaviour with outlets
- **THEN** apply succeeds
- **AND** a later tick publishes those outlets

### Requirement: Unknown or unresolved wires are skipped
Apply MUST look up each wire key in the reflection catalog of wire types. An unknown type path or a producer id that does not resolve MUST be reported and MUST NOT remove an already-applied wire of that key. Wire types in the catalog that the entity does not name MUST be removed from that entity (disconnect).

#### Scenario: Unknown path does not rip out an existing wire
- **WHEN** the world already has a valid `TimeFrom` on an entity
- **AND** a reload names a typo type path for that entity instead
- **THEN** diagnostics include an unknown-wire item
- **AND** the existing `TimeFrom` remains

#### Scenario: Omitted catalog wire is disconnected
- **WHEN** the world has a `TimeFrom` on an entity
- **AND** a reload of that entity omits that type path from `wires`
- **THEN** the `TimeFrom` is removed
