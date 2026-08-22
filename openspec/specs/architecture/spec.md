# architecture Specification

## Purpose

Defines how Sway's layers own the scene: the graph as the single authored model, the Bevy world as an artifact derived from it, and the project as a directory that bounds one editing session.

## Requirements

### Requirement: The graph is the authored model and the world is derived
The graph MUST be the only authored model of a scene. Every entity, asset and component that a scene needs MUST be produced from the graph by projection, and MUST NOT be authored directly in the world.

Projection MUST be one-directional. No value that projection writes into the world may flow back into the graph. A projected artifact MUST be recreated from the graph after any change to the graph, and MUST be destroyed when the graph node that produced it is removed.

Graph shape and world shape MUST NOT be assumed to match. A node MAY produce an entity, an asset, a component on another node's entity, or nothing at all.

#### Scenario: A removed node takes its projection with it
- **WHEN** a node that produced a scene entity is deleted
- **THEN** that entity is despawned
- **AND** no orphaned entity, asset or component remains

#### Scenario: The world is not an authoring surface
- **WHEN** a component on a projected entity is changed directly in the world
- **THEN** the next projection restores it from the graph
- **AND** the graph is unchanged

#### Scenario: A node may produce no entity
- **WHEN** a node produces only an asset
- **THEN** no entity exists for that node
- **AND** the asset is still reachable by the nodes connected to it

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

### Requirement: A project is a directory and one project is open per session
A project MUST be a directory containing the graph document and every asset it references. Every path a graph names MUST resolve relative to that directory, so that moving or copying the directory preserves the project.

Exactly one project MUST be open at a time. Opening a different project MUST discard the current session's derived state entirely, rather than merging or reusing it.

Saving MUST write the graph back to the file it was opened from. Saving a project to a different directory is NOT supported.

#### Scenario: A project directory is portable
- **WHEN** a project directory is moved or copied to another location and opened
- **THEN** every asset the graph names still resolves
- **AND** the scene is identical

#### Scenario: Opening another project discards the previous one
- **WHEN** a project is open and another project is opened
- **THEN** no node, entity or asset from the previous project remains

#### Scenario: A saved project reopens unchanged
- **WHEN** a project is saved and then reopened
- **THEN** the graph is identical to the one that was saved

### Requirement: Evaluation waits for assets; input capture does not
Graph evaluation and projection MUST NOT run until every asset the project references has finished loading. Frames MAY be skipped while loading is outstanding.

Input capture MUST NOT be gated on asset loading. In particular, capture of the external time source MUST run every frame from startup, so that its notion of elapsed time is continuous across the loading period.

#### Scenario: The scene does not appear half-loaded
- **WHEN** some of a project's assets have loaded and others have not
- **THEN** no projection has run
- **AND** nothing of the scene is drawn

#### Scenario: Time is continuous across loading
- **WHEN** a project takes several seconds to load
- **THEN** the external time source is captured throughout
- **AND** the first evaluated tick reflects the time actually elapsed

### Requirement: Reloading a project is an explicit action
A change to the graph document on disk MUST NOT reload the project. Reloading MUST happen only when the user asks for it.

A change to a referenced content asset on disk MAY be picked up while the project is open.

#### Scenario: Saving does not trigger a reload
- **WHEN** the user saves the project
- **THEN** the graph in memory is not replaced by the file that was just written

#### Scenario: An edited graph file is not picked up
- **WHEN** the graph document is modified by another program while the project is open
- **THEN** the open project is unchanged

#### Scenario: An edited image is picked up
- **WHEN** an image the graph references is modified on disk while the project is open
- **THEN** the scene reflects the new image without reopening the project

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
Dependencies MUST run one way: the engine depends on no crate of this project; a domain crate depends on the engine; the host depends on domain crates.

The base-nodes crate is the generic signal layer. Other node domains MAY depend on it for shared vocabulary — in particular the Trigger occurrence payload. Peer node domains MUST NOT depend on each other.

Where two peer crates need to share a vocabulary and neither may depend on the other, that vocabulary MUST live in a crate of its own rather than being parked in the engine. Generic signal vocabulary MUST live in the base-nodes crate.

A declared dependency that the crate does not use MUST be removed.

#### Scenario: A converter depends on the generic layer, not the other way around
- **WHEN** the MIDI domain publishes Trigger occurrences that a base node reads
- **THEN** Trigger lives in the base-nodes crate
- **AND** the MIDI crate depends on the base-nodes crate
- **AND** the base-nodes crate does not depend on the MIDI crate

#### Scenario: Peer domains still do not depend on each other
- **WHEN** the MIDI domain and the runtime domain are inspected
- **THEN** neither crate depends on the other

#### Scenario: Two domains share a vocabulary without depending on each other
- **WHEN** a producing crate and a consuming crate need the same event or value vocabulary and neither is the base-nodes crate
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
