## ADDED Requirements

### Requirement: The editor owns selection and node placement
Which node is selected MUST be editor state. Where a node sits on the graph canvas MUST be editor state. Neither MUST be a value the graph evaluates, orders, or reports as a change.

Node placement MUST be persisted through the node's annotations, so that reopening a project restores the canvas the author left. Selection MUST NOT be persisted; a reopened project starts with nothing selected.

Selecting a node, or moving one on the canvas, MUST NOT cause anything projected from that node to be respawned, rewritten or re-evaluated.

#### Scenario: Selecting a node changes nothing else
- **WHEN** the user selects a node
- **THEN** no node is reported as changed
- **AND** the projected world is untouched

#### Scenario: Canvas placement survives a reload
- **WHEN** the user moves nodes on the canvas, saves, and reopens the project
- **THEN** the nodes are where the user left them

#### Scenario: Selection does not survive a reload
- **WHEN** a project is saved with a node selected and reopened
- **THEN** nothing is selected

### Requirement: An editing control converts its value to the field's type
The editor MUST convert whatever an editing control produced into a value of the edited field's declared type before the edit reaches the graph, because the control is the only thing that knows what it produced.

A numeric edit that falls outside the range of the field's type MUST be clamped to that range rather than discarded, so that the control does not appear to have ignored the input.

#### Scenario: An out-of-range number is clamped
- **WHEN** a numeric control produces a value beyond the range of the field's integer type
- **THEN** the field takes the nearest representable value
- **AND** the control shows that value rather than reverting

#### Scenario: A control's value reaches the field as the field's type
- **WHEN** a control edits a field
- **THEN** the graph receives a value already of that field's declared type

## MODIFIED Requirements

### Requirement: The editor reads the graph without a parallel model
The editor MUST populate its display from the graph itself, using the reflected type information of each node kind. It MUST NOT maintain a second description of nodes, sockets, edges or field kinds alongside the graph.

State that exists only to display the graph — the selection, canvas placement, pan and zoom — is the editor's own and is not a parallel model. The distinction is that no such state describes what a node *is*: removing all of it MUST leave the graph fully described.

Which editing control a field gets MUST be decided from that field's reflected type. A field whose type has no control MUST be shown read-only rather than omitted or misrepresented.

#### Scenario: A new node kind needs no editor change
- **WHEN** a node kind is added whose inlet field types already have controls
- **THEN** it appears in the palette, inspector and canvas with no editor-side description written for it

#### Scenario: A field with no control is shown read-only
- **WHEN** a node has an inlet whose type has no editing control
- **THEN** the inspector shows that field
- **AND** the field is not editable

#### Scenario: Editor state describes no node
- **WHEN** every piece of editor-owned display state is discarded
- **THEN** the graph still describes every node's kind, inlets and connections
