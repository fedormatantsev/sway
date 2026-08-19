## Purpose

Defines how Sway's layers own the scene: the graph as the single authored model, the Bevy world as an artifact derived from it, and the project as a directory that bounds one editing session.

## ADDED Requirements

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
Every authoring gesture — creating, deleting, editing a field, connecting, disconnecting, and manipulating a selection in the viewport — MUST be expressed as a command against the graph. No authoring surface may write the world directly.

Selection MAY be resolved from a projected entity back to the node that produced it. That resolution MUST carry identity only, and MUST NOT carry a value.

#### Scenario: A viewport manipulation writes the graph
- **WHEN** the user manipulates a projected entity's transform in the viewport
- **THEN** the change is applied to the node that produced that entity
- **AND** it survives the next projection

#### Scenario: Picking resolves identity only
- **WHEN** the user picks a projected entity
- **THEN** the node that produced it becomes the selection
- **AND** no field value is read out of the world into the graph

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
