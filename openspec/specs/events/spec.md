# events Specification

## Purpose

Carries things that *happen* — a note on, a beat boundary, a one-shot retrigger — through the graph's ordinary wires, as occurrences held in a per-tick arena and addressed by handles that live exactly one tick.

## Requirements

### Requirement: An occurrence handle is a value a wire may carry
A node kind MUST be able to declare an inlet or an outlet whose value is an **occurrence handle**: a small value naming a batch of occurrences of one payload type held outside the graph, rather than a level that stands until it is overwritten.

A handle MUST carry a payload type, and two handles MUST be connectable if and only if their payload types are the same. Adding a payload type MUST NOT require any change to the graph engine.

A handle MUST be an ordinary field value as far as the graph engine is concerned: copied along an edge by the same rule as any other value, with no step, type or legality case of its own. The engine MUST NOT name handles, occurrences, payload types, or the arena that holds them.

An inlet declared as an optional handle MUST behave as an optional inlet of any other type, and an inlet declared as a many-connection handle MUST accept several handles and present them in the graph's ordering-key order — so merging several trigger sources is the ordinary variadic rule, not a mechanism of its own.

#### Scenario: A trigger connection is an ordinary edge
- **WHEN** an outlet handle is connected to an inlet handle of the same payload type
- **THEN** the connection is made like any other edge
- **AND** no new kind of connection, node or type was required for it

#### Scenario: Payload types must match
- **WHEN** a connection is attempted between two handles whose payload types differ
- **THEN** the connection is refused when it is made
- **AND** no evaluation is required to discover this

#### Scenario: Several trigger sources merge on one inlet
- **WHEN** three outlet handles are connected to one many-connection inlet with ordering keys 30, 10 and 20
- **THEN** the node observes those handles in the order of the keys 10, 20, 30

#### Scenario: An unconnected optional handle inlet is absent
- **WHEN** an inlet declared as an optional handle has no connection
- **THEN** the node observes it as absent, exactly as for any other optional inlet

### Requirement: A producer publishes occurrences and holds no state
A node that has occurrences to publish MUST, during its own evaluation, hand the whole batch to the arena, receive a handle naming that batch, and write that handle to its own outlet. A producer MUST NOT keep the occurrences, the batch, or the handle in its state between evaluations: everything it published is reachable from the handle standing on its outlet, and only until the end of the tick.

A handle MUST become a value on a wire only by a node publishing it during its own evaluation.

A producer with nothing to publish MUST write the **empty handle** — a handle that names no batch, reads as no occurrences, and is never stale. Publishing an empty batch MUST yield the empty handle rather than a handle naming an empty batch, so that a producer which publishes unconditionally cannot report a change on a tick where nothing happened.

Because a handle is only valid for the tick it was published in, a producer MUST publish afresh on every tick it has occurrences for. A producer that stops publishing MUST leave nothing observable behind.

#### Scenario: A producer publishes and its outlet names the batch
- **WHEN** a node publishes two occurrences during its evaluation
- **THEN** its outlet holds a handle
- **AND** reading that handle yields those two occurrences in the order they were published

#### Scenario: Nothing to publish is the empty handle
- **WHEN** a node has no occurrences on a tick
- **THEN** its outlet holds the empty handle
- **AND** every consumer of that outlet reads no occurrences

#### Scenario: Publishing an empty batch is the empty handle
- **WHEN** a node publishes a batch containing no occurrences
- **THEN** it receives the empty handle
- **AND** no batch was recorded for it

#### Scenario: A producer keeps nothing between ticks
- **WHEN** a producing node's state is inspected after it has published
- **THEN** it holds neither the occurrences nor the handle

#### Scenario: A producer that stops publishing leaves nothing behind
- **WHEN** a node publishes occurrences on one tick and has none on the next
- **THEN** its consumers read those occurrences on the first tick
- **AND** read none on the second

### Requirement: A consumer reads by handle and cannot publish into what it reads
A node that holds a handle on an inlet MUST be able to read the occurrences it names during its own evaluation, and MUST NOT be able to add to them, remove from them, or alter them. Writing into a batch MUST be reachable only from publishing a new one.

Reading MUST NOT consume: a batch reads the same however many times, and by however many consumers, it is read.

A node that forwards or merges occurrences MUST publish a batch of its own and put that handle on its own outlet.

#### Scenario: Reading does not consume
- **WHEN** a consumer reads the same handle twice in one evaluation
- **THEN** both reads yield the same occurrences

#### Scenario: A consumer cannot write what it received
- **WHEN** a node holds a handle on an inlet
- **THEN** nothing it can do adds an occurrence to that batch

#### Scenario: Forwarding publishes a new batch
- **WHEN** a node reads occurrences on an inlet and passes them on
- **THEN** it publishes a batch of its own
- **AND** the handle on its outlet is not the handle it received

### Requirement: Occurrences fan out without being copied
Every consumer of one outlet handle MUST read the same batch. A batch MUST NOT be duplicated per connection, and what one consumer does with it MUST NOT change what another consumer reads.

#### Scenario: Two consumers read the same batch
- **WHEN** an outlet handle naming two occurrences is connected to two different nodes
- **THEN** both nodes read both occurrences during that tick

#### Scenario: One consumer does not affect the other
- **WHEN** one of two consumers reads the batch
- **THEN** the other still reads every occurrence

### Requirement: Occurrences reach consumers in the tick they were published
An occurrence published by a node MUST be readable by every node downstream of it in the same tick, because the evaluation order already places a producer before its consumers. A chain of trigger connections MUST carry occurrences end to end within one tick.

Occurrences MUST NOT be delayed to a later tick, and a consumer MUST NOT read occurrences its producer has not yet published this tick. A trigger connection that is part of a cycle therefore carries nothing: a cycle member holds the handle its partner published on the previous tick, and that handle is stale.

#### Scenario: A two-hop trigger chain resolves in one tick
- **WHEN** node A publishes a batch, A's outlet is connected to B, and B forwards it to C
- **THEN** C reads the occurrences in the same tick

#### Scenario: A trigger connection in a cycle carries nothing
- **WHEN** two nodes are connected into each other by trigger connections
- **THEN** the tick still evaluates both
- **AND** neither reads the other's occurrences from the previous tick

### Requirement: The arena is emptied before every tick, and a stale handle reads empty
Every batch MUST be discarded before a tick evaluates any node, so a node reads only what was published since that tick began. No handle may yield occurrences in a later tick than the one they were published in, and nothing may accumulate across ticks. What a node chooses to remember of what it read is its own state, like any other value it keeps between evaluations.

A handle published in an earlier tick MUST read as no occurrences. It MUST NOT read as occurrences published this tick by another producer, and MUST NOT fail the evaluation that reads it. A handle MUST therefore carry enough to tell the tick it belongs to from any other.

The empty handle MUST read as no occurrences on every tick, and MUST never become stale.

#### Scenario: Nothing survives to the next tick
- **WHEN** a batch is published on one tick and the next tick begins
- **THEN** the arena holds no batch from the previous tick

#### Scenario: A stale handle reads as no occurrences
- **WHEN** a node holds a handle published on an earlier tick
- **THEN** reading it yields no occurrences
- **AND** it does not yield another producer's occurrences
- **AND** the evaluation succeeds

#### Scenario: Publishing on every tick does not grow the arena
- **WHEN** a node publishes a batch of the same size on many consecutive ticks
- **THEN** the arena holds only the current tick's batches

### Requirement: Publishing a batch is a change to the producer's outlet
A handle names one tick's batch, so a node that publishes writes a different outlet value than it did last tick, and MUST be reported as changed like any other outlet write. Every node the handle then reaches MUST be reported as changed for the same reason.

The empty handle replacing the empty handle MUST be the one case that reports nothing: it equals what already stands in the field, so the node MUST NOT be reported as changed and neither may any consumer of that outlet. This MUST hold whether the producer wrote the empty handle itself or published a batch that turned out to be empty. A tick on which no node publishes any occurrence MUST report no node changed on account of occurrences.

#### Scenario: A silent producer reports no change
- **WHEN** a node writes the empty handle on a tick, having written it on the previous tick
- **THEN** it is not reported as changed
- **AND** neither is any consumer of that outlet

#### Scenario: An unconditional producer with nothing to say reports no change
- **WHEN** a node publishes an empty batch on every tick
- **THEN** no tick reports it as changed
- **AND** no tick reports any consumer of that outlet as changed

#### Scenario: A publishing producer reports a change
- **WHEN** a node publishes a batch
- **THEN** it is reported as changed
- **AND** so is each node its handle reaches this tick

### Requirement: A handle is session state, not authored data
A handle MUST NOT be authorable: no editing gesture may set one, and a node kind that declares a handle inlet MUST NOT require anything to be authored for it to evaluate.

A handle inlet that nothing is connected to MUST read as no occurrences for as long as it stays unconnected, rather than as a failed or absent evaluation. A node kind that declares handles MUST evaluate whether or not the arena is present.

#### Scenario: An unconnected handle inlet is empty, not an error
- **WHEN** a node whose inlet is a handle is evaluated with nothing connected to that inlet
- **THEN** it reads no occurrences
- **AND** the evaluation succeeds

#### Scenario: A handle inlet needs no authoring
- **WHEN** a node kind declaring a handle inlet is created
- **THEN** it evaluates with no value having been authored for that inlet

#### Scenario: No arena is no occurrences
- **WHEN** a node that publishes or reads occurrences is evaluated with no arena present
- **THEN** its evaluation succeeds
- **AND** its handle outlets are empty

### Requirement: Occurrences are one crate with one plugin
Everything occurrences need — the handle, the arena that holds the batches, the way a payload type is made known, and the emptying that happens before every tick — MUST live in a crate of its own that the graph engine does not depend on. Adding that crate's single plugin MUST be all a host does for the arena to exist and be emptied before every tick: a host MUST NOT have to register a system, a set, or a resource on the crate's behalf, and MUST NOT have to order the emptying against the tick itself.

A node domain that publishes or reads occurrences MUST depend on that crate rather than on another node domain, and the crate MUST NOT depend on any node domain.

#### Scenario: One plugin is the whole mechanism
- **WHEN** a host adds the occurrence plugin and nothing else from that crate
- **THEN** the arena exists and is emptied before each tick

#### Scenario: The engine names no occurrence
- **WHEN** the graph engine's dependencies and public items are inspected
- **THEN** none of them names a handle, an occurrence, a payload type or the arena

#### Scenario: Two domains exchange occurrences without depending on each other
- **WHEN** one node domain publishes occurrences of a payload type and another reads them
- **THEN** both depend on the occurrence crate
- **AND** neither depends on the other
