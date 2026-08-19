## ADDED Requirements

### Requirement: A graph is nodes and edges
A graph MUST consist of nodes and edges and nothing else. A node MUST have an identity that is stable for as long as that node exists, and that identity MUST NOT be reused for a different node.

An edge MUST connect one node's outlet to one node's inlet. Edges MUST NOT be nodes, and a connection MUST NOT require a node of its own.

Adding a kind of connection MUST NOT require declaring a new type. Two nodes are connectable if and only if the types at the two named fields satisfy the legality rule.

#### Scenario: A connection is data
- **WHEN** a new pair of node kinds is connected for the first time
- **THEN** no new connection type had to be declared for it to be legal

#### Scenario: A stale identity does not resolve
- **WHEN** a node is deleted and another node is created
- **THEN** the deleted node's identity does not resolve to the new node

### Requirement: A node is inlets, state, and outlets
A node MUST be a single reflected value with exactly three parts: **inlets** (values it consumes), **state** (memory that persists between evaluations), and **outlets** (values other nodes may consume). Any part MAY be empty, and an empty part MUST be addressable in the same way as a populated one, so that nothing has to special-case its absence.

Inlets MUST be authorable and MUST be serialized. State and outlets MUST NOT be serialized.

#### Scenario: A node with no state is shaped like one that has state
- **WHEN** a node kind has no state
- **THEN** it is addressed, serialized and evaluated by the same rules as a node kind that has state

#### Scenario: State does not survive a save
- **WHEN** a node with populated state and outlets is saved and reloaded
- **THEN** its inlets are restored
- **AND** its state and outlets are at their defaults until it is next evaluated

### Requirement: Node evaluation reads inlets and writes state and outlets
Evaluating a node MUST give it its inlets as they stand this tick, its state to read and write in place, its outlets to write in place, and read-only access to external state outside the graph.

Evaluation MUST NOT reach the graph itself. A node MUST NOT be able to read another node's inlets, state or outlets, or observe the edges connected to it.

A node whose output depends only on external state MUST be an ordinary node that reads that state during evaluation. No separate mechanism may exist for such nodes.

A write MUST NOT mark a value changed when it equals what was already there.

#### Scenario: An external time source is an ordinary node
- **WHEN** a node's output depends only on a clock outside the graph
- **THEN** it is evaluated like every other node
- **AND** it reads that clock during its own evaluation

#### Scenario: A node cannot reach the graph
- **WHEN** a node is evaluated
- **THEN** the graph is not reachable from the external state it is given

#### Scenario: Equal writes do not dirty
- **WHEN** evaluation writes an outlet a value equal to its current one
- **THEN** that outlet is not marked changed
- **AND** nothing downstream of it is recomputed

### Requirement: An edge addresses fields by path
An edge MUST name a source node and a path within its outlets, and a destination node and a path within its inlets. That path MUST name a **declared field of the part** — a field of the inlets or outlets struct — not a nested field inside a compound value.

A compound inlet (for example a whole transform) MUST be connected as a whole: its type is the legality of the edge. To drive one component of a compound, that component MUST be a declared inlet of its own, or a separate node MUST construct the compound from its parts.

An edge MUST be legal if and only if the type at the source path is accepted by the type at the destination path. Legality MUST be decided when the connection is made, not when it is evaluated.

Evaluating an edge MUST read only the named source field and write only the named destination field.

#### Scenario: A compound inlet is wired as a whole
- **WHEN** an inlet's type is a compound value
- **THEN** an edge into that inlet names the inlet itself
- **AND** connecting a component type of that compound to the inlet is refused

#### Scenario: Driving a component is a declared inlet
- **WHEN** a scene node needs its translation driven by a `Vec3`
- **THEN** translation is a declared inlet of that node
- **AND** the edge names `translation`, not a nested path through a transform

#### Scenario: An illegal connection is refused when made
- **WHEN** a connection is attempted between two paths whose types are not compatible
- **THEN** the connection is not made
- **AND** no evaluation is required to discover this

### Requirement: Inlets may be optional or variadic
An inlet MUST be able to declare that it is optional, meaning it has no value when nothing is connected, and the node decides what that means. An unconnected optional inlet MUST NOT be given a substitute value.

An inlet MUST be able to declare that it accepts many connections. Every edge MUST carry an ordering key, and the values arriving at such an inlet MUST be presented in ascending order of that key, with node identity breaking ties so that the order is deterministic.

The ordering key MUST be a sort key rather than a position, so that keys may be sparse and reordering a connection requires changing only that connection's key.

An inlet that accepts one connection MUST reject a second; connecting again MUST replace the existing connection.

#### Scenario: An unconnected optional inlet is absent, not defaulted
- **WHEN** an optional inlet has no connection
- **THEN** the node observes it as absent
- **AND** no substitute value is supplied on its behalf

#### Scenario: Many connections arrive in key order
- **WHEN** three edges land on one variadic inlet with ordering keys 30, 10 and 20
- **THEN** the node observes the values in the order of the keys 10, 20, 30

#### Scenario: Reordering changes one connection
- **WHEN** one edge on a variadic inlet is reordered
- **THEN** only that edge's ordering key changes
- **AND** the other edges' keys are untouched

#### Scenario: A single-connection inlet is replaced, not doubled
- **WHEN** an inlet that accepts one connection already has one and another is connected
- **THEN** the inlet has exactly one connection, the new one

### Requirement: An edge may carry no value
An edge MUST be able to declare a connection that carries no value, existing only to establish that two nodes are related and to constrain their evaluation order. Evaluating such an edge MUST write nothing.

A node MUST NOT be able to observe such a connection during evaluation. Only a consumer outside the graph may read which nodes a valueless connection relates.

#### Scenario: A valueless connection writes nothing
- **WHEN** a valueless edge is evaluated
- **THEN** no field of the destination node is written

#### Scenario: A valueless connection still orders
- **WHEN** a valueless edge connects node A to node B
- **THEN** A is evaluated before B

### Requirement: The graph rejects connections that would break its invariants
Making a connection MUST refuse a connection from a node to itself. Deleting a node MUST delete every edge that names it, in either direction.

A graph MUST NOT be left holding an edge that names a node which does not exist.

#### Scenario: A self connection is refused
- **WHEN** a connection is attempted from a node's outlet to its own inlet
- **THEN** no edge is created

#### Scenario: Deleting a node deletes its edges
- **WHEN** a node with inbound and outbound edges is deleted
- **THEN** none of those edges remain

### Requirement: Changes are tracked per node
A graph MUST record which nodes changed since consumers outside the graph last read it, at the granularity of a single node. A consumer MUST be able to act on only the nodes that changed.

A node MUST be recorded as changed when a command edits it, when an edge writes one of its inlets, or when evaluation writes its state or outlets — and MUST NOT be recorded as changed when a write left every value equal to what was there.

#### Scenario: An unrelated node is not reported as changed
- **WHEN** one node's inlet is edited
- **THEN** only that node is reported as changed

#### Scenario: An equal write reports nothing
- **WHEN** a tick writes only values equal to those already present
- **THEN** no node is reported as changed

## MODIFIED Requirements

### Requirement: Evaluation order
Rebuild MUST order **nodes** topologically. Per node, every inbound edge MUST be evaluated, then that node MUST be evaluated. A cycle MUST NOT stop the tick; cycle members MUST be appended after the acyclic part and read the previous tick's values.

The unit of ordering MUST be the node, because evaluation reads every inlet and writes every outlet — so every outlet of a node genuinely depends on every one of its inlets, and no finer vertex would report a cycle the node does not actually have.

#### Scenario: Two-hop chain resolves in one tick
- **WHEN** A connects into B and B connects into C with no cycle
- **THEN** one tick writes A's value through to C

#### Scenario: Cycle is reported and still ticks
- **WHEN** two nodes connect into each other
- **THEN** diagnostics include both nodes
- **AND** the tick still evaluates those nodes

#### Scenario: Order is deterministic
- **WHEN** a graph is rebuilt twice without changing
- **THEN** the two orders are identical

## REMOVED Requirements

### Requirement: Wire is a relationship on the consumer
**Reason**: Connections are no longer components on entities. An edge is data in the graph naming two nodes and two field paths, so single-source, fan-out and rewire are properties of the edge set rather than of a relationship type. See `A graph is nodes and edges` and `The graph rejects connections that would break its invariants`.

**Migration**: Every relationship-based wire type is replaced by an edge. A connection previously expressed by inserting a wire component of type `T` on the consumer becomes an edge naming the source node's outlet path and the destination node's inlet path.

### Requirement: Reflection is the wire catalog
**Reason**: There are no wire types to catalog. Connection legality is decided by comparing the reflected types at the two named field paths, so there is nothing to register and nothing to enumerate.

**Migration**: Remove wire-type registration. Legality checks read the reflected type information of the node kinds involved.

### Requirement: Default wire evaluation is a reflected field copy
**Reason**: Superseded by `An edge addresses fields by path`, which copies the named outlet field into the named inlet field and removes the special case of the source being tuple field `0`.

**Migration**: A wire that copied source tuple field `0` into a named target field becomes an edge whose source path names that outlet field explicitly.

### Requirement: Behaviour is inlets, state, and outlets
**Reason**: Superseded by `A node is inlets, state, and outlets` and `Node evaluation reads inlets and writes state and outlets`. A node is now one reflected value holding all three parts rather than separate components resolved by type, evaluation may read external state outside the graph, and there is no separate registration distinguishing a behaviour from a wire.

**Migration**: Each behaviour becomes a node kind whose three parts are its inlets, state and outlets. Work that was excluded from the graph because it needed to read external state may now be a node, since evaluation is given read-only access to that state.

### Requirement: Authoring watches include behaviours
**Reason**: There are no components to watch. The graph is mutated only by commands, so the point at which the topology changes is known exactly and needs no observation.

**Migration**: Remove the authoring-gated watches and the show/authoring distinction they carried. A command that changes topology marks the order for rebuild directly.
