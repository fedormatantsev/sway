# document Specification

## Purpose

Defines how a project document names value wires and which components it stores: full reflected type paths as wire keys, inlets only in the component map, version compatibility.

## Requirements

### Requirement: A document is nodes and edges keyed by stable ids
A document MUST hold a collection of nodes, each keyed by an id that is stable across saves, and a list of edges that reference nodes by those ids.

An id MUST be assigned once when a node is created and MUST NOT change when other nodes are added or removed. Deleting one node MUST NOT change any other node's id.

Ids MUST be unique within a document. A document naming the same id twice MUST be rejected as a whole parse error.

#### Scenario: Deleting a node leaves other ids untouched
- **WHEN** a node is deleted from a document and the document is saved
- **THEN** every remaining node keeps the id it had
- **AND** every edge still names the same ids it named before

#### Scenario: Duplicate ids are refused
- **WHEN** a document names the same node id twice
- **THEN** parse fails
- **AND** nothing is loaded

### Requirement: An edge names two ids, two field paths, and an ordering key
Each edge entry MUST name the source node id and a path within its outlets, the destination node id and a path within its inlets, and an ordering key.

An edge MUST NOT be keyed by a type name. Paths MUST be field paths within the node kinds they address.

#### Scenario: An edge round-trips
- **WHEN** a document declares an edge from one node's outlet path to another node's inlet path with an ordering key
- **THEN** loading and saving reproduces the same source, destination, paths and key

#### Scenario: Connection order is preserved
- **WHEN** several edges land on one variadic inlet with distinct ordering keys
- **THEN** reloading the document presents them in the same order

### Requirement: Format version 3
The supported project format version MUST be `3`. A document whose version is not `3` MUST be rejected as a whole parse error, not partially applied.

#### Scenario: An earlier version is refused
- **WHEN** a file declares a version earlier than 3
- **THEN** parse fails with an unsupported-version error
- **AND** nothing is loaded

#### Scenario: Version 3 loads
- **WHEN** a well-formed file declares `version: 3`
- **THEN** parse succeeds

### Requirement: A document stores inlets only
A node entry MUST store the node's kind, its authored inlets, and its editor position. It MUST NOT store state or outlets.

Loading MUST restore inlets and MUST NOT require state or outlets to be present. Any value that identifies a loaded asset MUST NOT be stored, because it is meaningful only within one session; a node that references an asset MUST store the path it loads from instead.

#### Scenario: Saving omits state and outlets
- **WHEN** a node with populated state and outlets is saved
- **THEN** the entry holds its inlets, kind and position only

#### Scenario: A node referencing an asset stores a path
- **WHEN** a node that has loaded an asset is saved and reloaded
- **THEN** the entry stores the path it loads from
- **AND** the node loads that asset again on reload

### Requirement: Unresolved ids, kinds and paths are reported and skipped
Loading MUST report and skip a node whose kind is not known, and an edge naming an id that does not resolve or a path that does not exist on the node it addresses. A single bad entry MUST NOT prevent the rest of the document from loading.

Loading MUST NOT partially apply a node: a node whose inlets cannot be read MUST be reported and skipped as a whole.

#### Scenario: An unknown node kind is skipped
- **WHEN** a document names a node kind that is not known
- **THEN** a diagnostic naming that id and kind is reported
- **AND** every other node loads

#### Scenario: An edge naming a missing node is skipped
- **WHEN** an edge names a source id that no node entry declares
- **THEN** a diagnostic naming that edge is reported
- **AND** the graph loads without it

#### Scenario: An edge naming a missing path is skipped
- **WHEN** an edge names a field path that does not exist on the node it addresses
- **THEN** a diagnostic naming that edge is reported
- **AND** the graph loads without it
