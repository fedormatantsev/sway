# editor Specification

## Purpose

Defines how the graph editor identifies inlets and edges, and which node parts are inspectable: sockets keyed by wire type path; inspector shows inlets only.

## Requirements

### Requirement: Inlet identity is a field path
Each inlet socket on a node MUST be identified by a field path within that node kind's inlets. Sockets MUST be discovered from the node kind's own declared inlets, so that an unconnected inlet has a socket and a node's sockets do not depend on what happens to be connected.

Outlet sockets MUST be identified the same way, by a field path within that node kind's outlets.

Connect legality MUST be decided by comparing the types at the two paths.

#### Scenario: Two inlets stay distinct
- **WHEN** a node kind declares two inlets
- **THEN** the editor lists two sockets whose keys are those field paths
- **AND** an edge for one of them names that path

#### Scenario: Unwired inlet still has a socket
- **WHEN** a node has an inlet with nothing connected to it
- **THEN** the editor still lists that inlet
- **AND** the inlet is not connected

#### Scenario: Layout order is not identity
- **WHEN** a node's visual sockets are re-sorted
- **THEN** an existing edge still attaches to the socket whose key matches its path

### Requirement: Connect and disconnect name a field path
An editor connect or disconnect command MUST name the source node and outlet path and the destination node and inlet path. A connect MUST be refused when the two types are not compatible, when the two nodes are the same node, and when the destination inlet already has a connection and does not accept several.

Connecting to an inlet that already has a connection and does not accept several MUST replace that connection rather than fail.

#### Scenario: Legal drop creates the edge
- **WHEN** the user completes a drag from a legal outlet onto an inlet
- **THEN** an edge is created naming both nodes and both paths

#### Scenario: Illegal drop is a no-op
- **WHEN** a connect command names two paths whose types are not compatible
- **THEN** no edge is created

#### Scenario: A self connection is refused
- **WHEN** the user drags from a node's outlet onto its own inlet
- **THEN** no edge is created

#### Scenario: Reconnecting a single-connection inlet replaces
- **WHEN** the user drops a second connection onto an inlet that accepts one
- **THEN** the inlet holds the new connection only

### Requirement: Edges carry two field paths and an ordering key
Each painted edge MUST carry the source node and outlet path, the destination node and inlet path, and its ordering key. The edge MUST attach to the sockets whose keys are those paths.

Edges landing on an inlet that accepts several connections MUST be presented in ordering-key order, and the editor MUST allow that order to be changed.

#### Scenario: Edge lands on its own socket
- **WHEN** a node has multiple inlets and a connection into one of them
- **THEN** the edge is drawn to the socket whose key is that inlet's path

#### Scenario: Reordering a variadic inlet is possible
- **WHEN** several edges land on one variadic inlet
- **THEN** the editor presents them in ordering-key order
- **AND** the user can change that order

### Requirement: The editor reads the graph without a parallel model
The editor MUST populate its display from the graph itself, using the reflected type information of each node kind. It MUST NOT maintain a second description of nodes, sockets, edges or field kinds alongside the graph.

Which editing control a field gets MUST be decided from that field's reflected type. A field whose type has no control MUST be shown read-only rather than omitted or misrepresented.

#### Scenario: A new node kind needs no editor change
- **WHEN** a node kind is added whose inlet field types already have controls
- **THEN** it appears in the palette, inspector and canvas with no editor-side description written for it

#### Scenario: A field with no control is shown read-only
- **WHEN** a node has an inlet whose type has no editing control
- **THEN** the inspector shows that field
- **AND** the field is not editable

### Requirement: Inspector shows inlets only
The inspector MUST show a node's authored inlet fields. It MUST NOT show state. Outlets MUST appear as outlet sockets for wiring, not as authored inspector fields.

A field with a connection MUST still be editable. An edit to a connected field holds until the next tick overwrites it.

#### Scenario: Outlet is a socket, not a field
- **WHEN** a node has inlets and outlets
- **THEN** the inspector lists the inlet fields
- **AND** the canvas offers an outlet socket
- **AND** the inspector does not list the outlet as an editable field

#### Scenario: State is hidden
- **WHEN** a node kind has state
- **THEN** the inspector does not list it

#### Scenario: A connected field is still editable
- **WHEN** an inlet has a connection into it
- **THEN** the inspector still accepts an edit to that field
