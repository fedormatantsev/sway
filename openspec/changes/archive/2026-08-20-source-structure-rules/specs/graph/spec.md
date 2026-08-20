## ADDED Requirements

### Requirement: A node carries opaque metadata
A node MUST carry a set of annotations alongside its three parts, keyed by name, each holding a value of any type the project's type registry knows. The graph MUST NOT interpret any key or value in that set, MUST NOT act on a change to it, and MUST NOT require any particular key to be present.

Writing an annotation MUST NOT mark the node changed, because an annotation is not a node value and nothing downstream of the node depends on it.

Annotations MUST survive a save and a reload, so that a surface which stores presentation state there does not lose it.

#### Scenario: The graph does not interpret an annotation
- **WHEN** an annotation is written on a node under a key the graph has never seen
- **THEN** it is stored and readable
- **AND** evaluation, ordering and connection legality are unaffected

#### Scenario: An annotation keeps its type
- **WHEN** an annotation is written as a value of some registered type and read back
- **THEN** it is readable as that same type, without the reader parsing it out of another representation

#### Scenario: An annotation is not a node change
- **WHEN** a node's annotation is written
- **THEN** that node is not reported as changed

#### Scenario: Annotations round-trip
- **WHEN** a node with annotations is saved and reloaded
- **THEN** its annotations are restored

### Requirement: A field edit carries a reflected value
Writing one of a node's inlet fields MUST carry the new value reflectively. The graph MUST NOT enumerate the concrete types such a write may carry, so that a node kind declaring a field of a type no other node kind uses is editable without changing the graph.

A write whose value does not fit the type at the named path MUST be refused, and MUST leave the field as it was. A write whose value equals the field's current value MUST report that nothing changed and MUST NOT mark the node changed.

Converting whatever an editing control produced into a value of the field's type is the authoring surface's responsibility, not the graph's.

#### Scenario: A write reports which of the three things happened
- **WHEN** a field write is applied
- **THEN** the caller is told whether the value was written, was already equal, or was refused

#### Scenario: An edit of an unfamiliar type is applied
- **WHEN** a node declares an inlet whose type no other node kind uses, and an edit names that path
- **THEN** the value is written
- **AND** the graph required no knowledge of that type

#### Scenario: A mismatched edit is refused
- **WHEN** an edit carries a value whose type does not fit the field at the named path
- **THEN** the field keeps its previous value
- **AND** the caller is told the edit did not apply

#### Scenario: An equal edit reports no change
- **WHEN** an edit writes a value equal to the field's current value
- **THEN** the node is not reported as changed

## MODIFIED Requirements

### Requirement: A graph is nodes and edges
A graph MUST consist of nodes and edges and nothing else. A node MUST have an identity that is stable for as long as that node exists, and that identity MUST NOT be reused for a different node.

An edge MUST connect one node's outlet to one node's inlet. Edges MUST NOT be nodes, and a connection MUST NOT require a node of its own.

Adding a kind of connection MUST NOT require declaring a new type. Two nodes are connectable if and only if the types at the two named fields satisfy the legality rule.

A graph MUST NOT hold the state of any surface that displays it — no selection, no viewport, no canvas placement. Such state belongs to the surface that owns it; the graph offers only per-node annotations it does not interpret.

#### Scenario: A connection is data
- **WHEN** a new pair of node kinds is connected for the first time
- **THEN** no new connection type had to be declared for it to be legal

#### Scenario: A stale identity does not resolve
- **WHEN** a node is deleted and another node is created
- **THEN** the deleted node's identity does not resolve to the new node

#### Scenario: The graph holds no display state
- **WHEN** the graph is inspected with no editor present
- **THEN** it reports nodes, edges, per-node annotations and evaluation order
- **AND** it reports nothing about what is selected or where anything is drawn
