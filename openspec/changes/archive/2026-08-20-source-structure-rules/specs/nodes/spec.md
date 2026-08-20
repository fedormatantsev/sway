## ADDED Requirements

### Requirement: A node kind is named for what it does, not for what it produces
A node kind's name MUST NOT be the name of a type it constructs or consumes. A name that collides with a type already in scope forces every use of that type to be aliased, and makes a document entry ambiguous about whether it names a node kind or a value type.

A node kind whose purpose is to assemble a value MUST be named for the assembling, so the node and the type it produces can be discussed and imported together.

#### Scenario: A constructing node does not take its output type's name
- **WHEN** a node kind's outlet is a value of some named type
- **THEN** the node kind has a different name from that type

#### Scenario: A palette entry is unambiguous
- **WHEN** the palette lists a node kind that assembles a value
- **THEN** its name says what it makes rather than naming the made type

### Requirement: A vector inlet may be driven whole or per component
A node kind that consumes a vector value MUST declare it as one vector inlet. That inlet MUST be connectable as a whole, and MUST also accept a connection naming a single component, so that one component can be driven while the others keep their authored values.

Both routes MUST be available: assembling a vector once and fanning it out to several consumers, and reaching into one consumer's vector inlet directly.

#### Scenario: One component is driven and the rest are kept
- **WHEN** a connection names the second component of a vector inlet
- **THEN** that component takes the connected value each tick
- **AND** the inlet's other components keep their authored values

#### Scenario: Whole and partial connections coexist
- **WHEN** one node's vector inlet is connected whole and another's is connected per component
- **THEN** both are legal
- **AND** neither route required a change to the consuming node kind

### Requirement: A base node is a pure function of its inlets and state
A node kind in the base set MUST derive its outlets from its own inlets and state alone. It MUST NOT read anything outside the graph during evaluation.

A base node whose behaviour advances over time MUST take that time as an inlet, so that the same inlets and state always produce the same outlets and the source of time is a connection the author can see and change.

#### Scenario: The same inputs give the same output
- **WHEN** a base node is evaluated twice with identical inlets and state
- **THEN** it produces identical outlets both times

#### Scenario: Time arrives on a connection
- **WHEN** a time-driven base node is evaluated
- **THEN** its notion of time came from an inlet
- **AND** nothing outside the graph was read

#### Scenario: Retiming is authored, not built in
- **WHEN** a time-driven base node's time inlet is connected to a different time source
- **THEN** the node follows that source with no change to the node kind
