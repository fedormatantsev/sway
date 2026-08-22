## MODIFIED Requirements

### Requirement: A document stores inlets only
A node entry MUST store the node's kind, its authored inlets, and its annotations. It MUST NOT store state or outlets.

Annotations MUST be stored keyed by name, each carrying a value of any type the project's type registry knows, recorded so that its type is recoverable on load without the document declaring what any key holds. The document MUST NOT interpret a key, MUST NOT give any key a field of its own, and MUST NOT reject an entry carrying a key it does not recognise. The document therefore names no surface's concerns: a node that is annotated by an editor and a node that is not are stored and loaded by the same rule.

An annotation whose type is not registered MUST be reported and skipped, and the node it belongs to MUST still load. Annotations MUST be written in a stable order, so that saving an unchanged document twice produces the same bytes.

Loading MUST restore inlets and annotations, and MUST NOT require state, outlets or annotations to be present. Any value that identifies a loaded asset MUST NOT be stored, because it is meaningful only within one session; a node that references an asset MUST store the path it loads from instead.

An inlet that holds session state rather than an authored value MUST be stored as a placeholder that carries nothing of the session that wrote it, and MUST load as freshly initialised session state. An occurrence handle is such an inlet: what a document records of it MUST NOT name a batch of occurrences or the tick it belonged to, and MUST load as the empty handle. A node kind that declares a handle inlet MUST otherwise save and load by exactly the same rule as every other node, without the document naming that inlet or its payload type.

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

#### Scenario: An occurrence handle inlet round-trips as the empty handle
- **WHEN** a node whose inlets include a handle naming a batch of occurrences is saved and reloaded
- **THEN** the node loads with its other inlets restored
- **AND** its handle inlet is the empty handle

#### Scenario: A handle inlet does not stop a node from saving
- **WHEN** a document containing a node kind that declares a handle inlet is saved
- **THEN** that node is written like every other node
- **AND** no diagnostic is reported for it
