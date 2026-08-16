# editor Specification

## Purpose

Defines how the graph editor identifies inlets and edges, and which node parts are inspectable: sockets keyed by wire type path; inspector shows inlets only.

## Requirements

### Requirement: Inlet identity is the wire type path
Each inlet socket on a node MUST be identified by the full reflected type path of a wire type. The snapshot MUST discover sockets by scanning that entity's existing components (its inlet part and any relationship components registered as wires). A type is a wire if and only if that type is in the reflection catalog. The snapshot MUST NOT invent sockets by iterating the catalog, and MUST NOT construct a placeholder entity to query wire metadata. Connect legality MUST scan the producer and consumer entities' components.

#### Scenario: Two inlets stay distinct
- **WHEN** an entity carries components for two different registered wire types (inlet fields and/or those relationships)
- **THEN** the snapshot lists two inlets whose keys are those types' type paths
- **AND** an edge for one of them names that type path

#### Scenario: Unwired inlet still has a socket
- **WHEN** an entity has an inlet or target component and does not carry that field's wire relationship
- **THEN** the snapshot still lists that inlet
- **AND** the inlet is not connected

#### Scenario: Layout order is not identity
- **WHEN** a new legal wire type appears for an entity and visual sockets are re-sorted
- **THEN** an existing edge still attaches to the socket whose key matches its type path

### Requirement: Connect and disconnect name a type path
An editor connect or disconnect command MUST name the wire by full type path. The world MUST refuse a connect when the producer lacks that wire's source component or the consumer lacks its target component.

#### Scenario: Legal drop inserts the relationship
- **WHEN** the user completes a drag from a legal producer onto an inlet
- **THEN** the consumer gains the wire type named by that inlet's type path, naming the producer

#### Scenario: Illegal drop is a no-op
- **WHEN** a connect command names a type path whose source component the producer does not have
- **THEN** no relationship of that type is inserted

### Requirement: Edges carry the wire type path
Each painted edge MUST carry the wire type path, the producer node, and the consumer node. The edge MUST attach to the inlet socket whose key is that path.

#### Scenario: Edge lands on its own inlet
- **WHEN** a node has multiple inlets and a connection for one wire type
- **THEN** the edge is drawn to the socket whose key is that wire's type path

### Requirement: Inspector shows inlets only
The inspector MUST show authored inlet fields. It MUST NOT show state. Outlets MUST appear as outlet sockets for wiring, not as authored inspector fields.

#### Scenario: Outlet is a socket, not a field
- **WHEN** an entity has inlets and outlets
- **THEN** the inspector lists the inlet fields
- **AND** the canvas offers an outlet socket
- **AND** the inspector does not list the outlet as an editable component

#### Scenario: State is hidden
- **WHEN** an entity has a state component
- **THEN** the inspector does not list that component
