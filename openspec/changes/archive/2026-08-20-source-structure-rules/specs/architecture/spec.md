## ADDED Requirements

### Requirement: The engine crate knows no concrete domain
Exactly one crate MUST own the generic graph mechanics — identity, nodes, edges, path resolution, connect legality, ordering and the tick. That crate MUST NOT name any concrete node kind, any UI toolkit type, any MIDI type, any render type, or any on-disk format.

Its public surface MUST stay as small as the mechanics require. Where a behaviour is already provided by the ECS framework the project builds on, the engine MUST use that provision rather than introducing its own type for it. An item with no consumer outside the engine MUST NOT be public.

The engine MUST NOT enumerate the concrete value types a node kind may declare. Anything the engine carries on behalf of a node's fields MUST be expressed reflectively, so that adding a node kind with a new field type requires no edit to the engine.

#### Scenario: A new node kind needs no engine change
- **WHEN** a node kind is added whose inlets use a field type no existing node kind uses
- **THEN** it registers, connects, evaluates and serializes with no change to the engine crate

#### Scenario: The engine names no domain type
- **WHEN** the engine crate's dependencies and public items are inspected
- **THEN** none of them names a UI toolkit, a MIDI type, a render type or a document format

#### Scenario: An unused public item is not public
- **WHEN** an engine item has no caller outside the engine crate
- **THEN** it is not part of the engine's public surface

### Requirement: A node domain is a self-contained crate with one plugin
Each domain of node kinds MUST live in its own crate holding both the node kinds and their projection onto the ECS world. A domain crate MUST expose exactly one top-level plugin, and adding that plugin MUST register every type, system and resource the domain needs to work.

A host MUST NOT have to add a second plugin, register a type, or insert a resource on a domain's behalf.

A crate MUST be named for the domain it covers, not for the language construct it contains.

#### Scenario: One plugin is the whole domain
- **WHEN** a host adds a domain crate's top-level plugin and nothing else from that crate
- **THEN** every node kind in that domain is in the palette, loads from a document, evaluates, and projects

#### Scenario: A domain does not leak registration
- **WHEN** a domain crate's plugin is inspected
- **THEN** it registers its own node kinds and part types
- **AND** it registers nothing belonging to another domain

### Requirement: Dependencies point from host to domain to engine
Dependencies MUST run one way: the engine depends on no crate of this project; a domain crate depends on the engine; the host depends on domain crates. A domain crate MUST NOT depend on another domain crate.

Where two crates need to share a vocabulary and neither may depend on the other, that vocabulary MUST live in a crate of its own rather than being parked in the engine.

A declared dependency that the crate does not use MUST be removed.

#### Scenario: Two domains share a vocabulary without depending on each other
- **WHEN** a producing crate and a consuming crate need the same event or value vocabulary
- **THEN** that vocabulary lives in a crate both depend on
- **AND** neither the engine nor either domain crate is that crate's owner by default

#### Scenario: Declared dependencies are real
- **WHEN** a crate's manifest is compared against its source
- **THEN** every declared dependency is referenced by that source

### Requirement: Code that nothing reaches is deleted
The workspace MUST NOT retain a public item, module, or plugin that no build path reaches. Work deliberately deferred past the current milestone MUST be recorded in the roadmap rather than kept as unreachable code.

#### Scenario: An unreachable plugin is removed
- **WHEN** a plugin is exported but added by no application or test
- **THEN** it is deleted rather than left exported

## MODIFIED Requirements

### Requirement: Authoring writes reach the world only through the graph
Every authoring gesture that changes the scene — creating, deleting, editing a field, connecting, disconnecting, and manipulating a projected entity in the viewport — MUST be applied to the graph. No authoring surface may write the world directly.

The graph MUST offer one mutation surface, and it MUST be the graph's own operations. There MUST NOT be a second vocabulary in the engine that only restates those operations as data. A surface that cannot reach the graph at the moment a gesture happens MAY record that gesture in a form of its own choosing and apply it later; that form belongs to the surface, not to the engine.

Presentation state that does not change the scene — which node is selected, where a node sits on the editor canvas — MUST NOT be a graph value and MUST NOT be applied as an authoring gesture. It is owned by the editor.

Selection MAY be resolved from a projected entity back to the node that produced it. That resolution MUST carry identity only, and MUST NOT carry a value.

#### Scenario: A viewport manipulation writes the graph
- **WHEN** the user manipulates a projected entity's transform in the viewport
- **THEN** the change is applied to the node that produced that entity
- **AND** it survives the next projection

#### Scenario: Picking resolves identity only
- **WHEN** the user picks a projected entity
- **THEN** the node that produced it becomes the selection
- **AND** no field value is read out of the world into the graph

#### Scenario: Selecting a node is not a scene edit
- **WHEN** the user selects a different node
- **THEN** no node is reported as changed
- **AND** nothing in the projected world is respawned or rewritten

#### Scenario: A surface that can reach the graph does not defer
- **WHEN** an authoring surface already holds the graph mutably at the moment of the gesture
- **THEN** it applies the gesture directly
- **AND** it does not construct an intermediate description of the gesture first
