## Purpose

Defines how value wires and in-tick behaviours are catalogued, ordered, and run: relationships on consumers, reflection as the type catalog, field-copy along wires, and nodes as optional inlets, state, and outlets.

## ADDED Requirements

### Requirement: Wire is a relationship on the consumer
A value wire MUST be a one-source relationship component on the consumer that names the producer. Fan-out MUST be the corresponding relationship-target collection on the producer. The engine MUST NOT introduce edge entities.

#### Scenario: Single source per inlet type
- **WHEN** a consumer already has a wire of type T pointing at producer A
- **AND** a wire of the same type T is inserted pointing at producer B
- **THEN** the consumer's T names B and A is no longer that inlet's source

#### Scenario: Fan-out
- **WHEN** two consumers each carry the same wire type naming one producer
- **THEN** both connections remain and each consumer receives that producer's source value independently

### Requirement: Reflection is the wire catalog
A relationship type MUST be treated as a value wire if and only if it is registered for reflection as a wire.

#### Scenario: Unregistered relationship is not a value wire
- **WHEN** an entity carries a relationship type that is not registered as a reflected wire
- **THEN** the tick MUST NOT copy a value along that relationship
- **AND** the document MUST NOT emit or apply it as a value wire

### Requirement: Default wire evaluation is a reflected field copy
Unless a wire type defines its own evaluation, a tick MUST copy the producer's outlet (source component) tuple field `0` into the named field of the consumer's inlet (target component). Evaluation MUST read only that outlet and write only that inlet. The relationship component MUST be read only for routing (the producer entity) and MUST NOT be mutated.

#### Scenario: Source tuple field reaches the named target field
- **WHEN** a registered value wire connects a producer whose source component tuple field `0` is `0.5` to a consumer that has the target component
- **THEN** after the tick the named target field is `0.5`

#### Scenario: Equal value does not dirty the target
- **WHEN** a field-copy wire would write a value equal to the target field
- **THEN** the target component MUST NOT be marked changed

#### Scenario: Missing source or target is a no-op
- **WHEN** the producer lacks the source component or the consumer lacks the target component
- **THEN** evaluation MUST NOT panic
- **AND** the other entity's components MUST be left unchanged
- **AND** rebuild MUST record the miss in graph diagnostics

### Requirement: Behaviour is inlets, state, and outlets
A node MAY have any combination of three optional parts: **inlets** (authored and/or driven by wires), **state** (internal memory), and **outlets** (values other wires may read). A behaviour MUST be registered for reflection as a behaviour, not as a wire. When inbound wires target this entity's inlets, those wires MUST be evaluated before the behaviour. Ordinary systems MUST still handle work that does not need that placement.

A behaviour MUST read inlets, read and write state in place, and write outlets in place. It MUST NOT read previous outlet values. It MUST NOT read or write any other world state. Inputs are the current inlets, mutable state if the node has a state part, a mutable outlet slot if the node has outlets, and this tick's context. The tick MUST insert a default state or outlet component when that part exists but the component is missing, then pass it in. A write MUST NOT mark a component changed when the value equals what was there.

#### Scenario: Behaviour sees this tick's inlets
- **WHEN** an entity has inlets and a value wire into those inlets
- **THEN** that tick MUST write the inlet before the behaviour publishes its outlets

#### Scenario: Outlets are write-only
- **WHEN** a behaviour runs
- **THEN** evaluation must not depend on the previous outlet value
- **AND** it writes outlets (and state, if any) in place, not inlets

#### Scenario: Missing state on first run
- **WHEN** a behaviour has a state part and that component is absent
- **THEN** the tick inserts a default state
- **AND** evaluation receives that state as a mutable slot

#### Scenario: Changed-only work stays a system
- **WHEN** work depends only on a changed component and not on a same-tick wired inlet
- **THEN** that work MUST NOT run as a graph behaviour

#### Scenario: Unregistered component is not a behaviour
- **WHEN** an entity carries a component that is not registered as a reflected behaviour
- **THEN** that component MUST NOT run as a graph behaviour this tick

### Requirement: Evaluation order
Rebuild MUST order entities topologically. Per entity, every inbound value wire MUST be evaluated, then that entity's behaviours MUST run. Each wire and behaviour in that order MUST be identified by its reflected type (type id and type path). A cycle MUST NOT stop the tick; cycle members MUST be appended after the acyclic part and read the previous tick's values.

#### Scenario: Two-hop chain resolves in one tick
- **WHEN** A wires into B and B wires into C with no cycle
- **THEN** one tick writes A's value through to C

#### Scenario: Cycle is reported and still ticks
- **WHEN** two entities wire each other
- **THEN** diagnostics include both entities
- **AND** the tick still evaluates those entities

### Requirement: Authoring watches include behaviours
While authoring is enabled, inserting or removing a reflected wire **or** a reflected behaviour carrier MUST mark the topology dirty so the next rebuild sees the entity. While authoring is absent (show), those mutations MUST NOT trigger a rescan; the existing order MUST be used.

#### Scenario: Adding a behaviour without a new wire still rebuilds
- **WHEN** authoring is enabled
- **AND** a behaviour carrier is added to an entity that had no wires change
- **THEN** subsequent ticks run that behaviour

#### Scenario: Show build ignores later wiring
- **WHEN** authoring is absent
- **AND** a wire is inserted after the initial rebuild
- **THEN** later ticks MUST NOT evaluate that wire
