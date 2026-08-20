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

### Requirement: A document stores inlets only
A node entry MUST store the node's kind, its authored inlets, and its annotations. It MUST NOT store state or outlets.

Annotations MUST be stored keyed by name, each carrying a value of any type the project's type registry knows, recorded so that its type is recoverable on load without the document declaring what any key holds. The document MUST NOT interpret a key, MUST NOT give any key a field of its own, and MUST NOT reject an entry carrying a key it does not recognise. The document therefore names no surface's concerns: a node that is annotated by an editor and a node that is not are stored and loaded by the same rule.

An annotation whose type is not registered MUST be reported and skipped, and the node it belongs to MUST still load. Annotations MUST be written in a stable order, so that saving an unchanged document twice produces the same bytes.

Loading MUST restore inlets and annotations, and MUST NOT require state, outlets or annotations to be present. Any value that identifies a loaded asset MUST NOT be stored, because it is meaningful only within one session; a node that references an asset MUST store the path it loads from instead.

#### Scenario: Saving omits state and outlets
- **WHEN** a node with populated state and outlets is saved
- **THEN** the entry holds its inlets, kind and annotations only

#### Scenario: An unrecognised annotation key round-trips
- **WHEN** a node carrying an annotation under an unfamiliar key is saved and reloaded
- **THEN** the annotation is restored unchanged, as the same type it was written with

#### Scenario: An annotation of an unregistered type is reported and skipped
- **WHEN** a node entry carries an annotation whose type the registry does not know
- **THEN** that annotation is reported and dropped
- **AND** the node's kind, inlets and other annotations still load

#### Scenario: Saving twice produces the same bytes
- **WHEN** a document with several annotations on one node is saved twice without changing
- **THEN** the two files are byte-identical

#### Scenario: No annotation key is privileged
- **WHEN** two nodes are saved, one annotated and one not
- **THEN** neither entry has a field named for a particular annotation
- **AND** both load without a diagnostic

#### Scenario: A document without annotations loads
- **WHEN** a node entry carries no annotations
- **THEN** it loads with none, and nothing is reported

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

### Requirement: Format version 4
The supported project format version MUST be `4`. A document whose version is not `4` MUST be rejected as a whole parse error, not partially applied.

A document written for an earlier version MUST be refused by version, naming the version it declares and the version this build reads, rather than failing on whichever field happens to be missing.

#### Scenario: An earlier version is refused
- **WHEN** a file declares a version earlier than 4
- **THEN** parse fails with an unsupported-version error naming both versions
- **AND** nothing is loaded

#### Scenario: Version 4 loads
- **WHEN** a well-formed file declares `version: 4`
- **THEN** parse succeeds
