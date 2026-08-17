# nodes Specification

## Purpose

Defines the built-in authorable components a document may name and a palette may offer: what each contributes to the scene, what its inlets accept, and how mesh nodes and material wires divide responsibility for a mesh's material.

## Requirements

### Requirement: A plane mesh is authorable with independent subdivision counts
A `PlaneMesh` node MUST produce a tessellated quad mesh. It MUST expose a size and two subdivision counts named `horizontal` and `vertical`, settable independently. The quad MUST face a single fixed axis so that its authored horizontal and vertical extents correspond to the horizontal and vertical axes of any texture mapped onto it.

Subdivision counts MUST be authorable because the vertex density governs the fidelity of any displacement applied by a material (see the `runtime` capability), and that density is a property of the geometry rather than of the material.

#### Scenario: Subdivision counts produce a tessellated grid
- **WHEN** a `PlaneMesh` is authored with `horizontal: 3` and `vertical: 1`
- **THEN** the produced mesh has interior vertices along both axes
- **AND** the vertex count along the horizontal axis is greater than along the vertical axis

#### Scenario: Zero subdivisions is a flat quad
- **WHEN** a `PlaneMesh` is authored with `horizontal: 0` and `vertical: 0`
- **THEN** the produced mesh is a single quad of four vertices

#### Scenario: Subdivision counts are independent
- **WHEN** only the `vertical` count of an existing `PlaneMesh` is changed
- **THEN** the produced mesh's vertex density changes along the vertical axis only

### Requirement: A mesh node carries no material until one is wired
A mesh node MUST NOT supply a material component of its own. A mesh entity with no material wire MUST NOT render.

This is required because a material component is typed per material kind: a mesh node that supplied one kind unconditionally would, once a wire delivered a second kind, carry two material components and be drawn once per kind.

#### Scenario: A newly created mesh node has no material
- **WHEN** a mesh node is created with no material wired
- **THEN** the entity carries no material component
- **AND** nothing is drawn for that entity

#### Scenario: A mesh never carries two material kinds at once
- **WHEN** a mesh entity is connected to a material producer of one kind
- **AND** is then connected to a material producer of a different kind
- **THEN** the entity carries exactly one material component
- **AND** the mesh is drawn once

### Requirement: A material wire supplies the material component it writes into
Connecting a material wire MUST make the consumer carry the material component that wire targets, whether or not the consumer already carried one. Disconnecting the wire MUST remove that component.

#### Scenario: Connecting supplies the target component
- **WHEN** a material wire is connected from a material producer to a mesh entity that carries no material component
- **THEN** the entity carries the material component that wire targets
- **AND** the producer's material reaches it

#### Scenario: Disconnecting removes the target component
- **WHEN** a connected material wire is removed from a mesh entity
- **THEN** the entity no longer carries the material component that wire targeted
- **AND** nothing is drawn for that entity

#### Scenario: A hand-authored document connects the same way
- **WHEN** a document names a mesh node and a material wire on it, and names no material component
- **THEN** loading the document produces an entity that renders with the wired material
