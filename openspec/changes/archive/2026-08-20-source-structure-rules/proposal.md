## Why

The workspace has ten crates and no written rule for what belongs in which one, so
the boundaries have drifted. `sway-graph` — the crate that is supposed to be a
generic graph engine — carries a masonry pointer/key vocabulary, an editor
selection field, a canvas position on every node, and a closed enum of concrete
value types (`Vec2`/`Vec3`/`Quat`) that must be edited whenever a node kind wants
a new inlet type. Meanwhile `sway-nodes` is a grab-bag named after a language
construct rather than a domain, three crates declare dependencies they never use,
and three render pipelines are exported but reachable from nothing.

Writing the rule down and then making the tree obey it is cheaper now, at ten
crates, than after the next domain lands.

## What Changes

**The rules, written down** (`docs/architecture.md` gains a "Source structure"
section, and the `architecture` spec gains the requirements behind it):

- `sway-graph` is the generic engine. It knows nodes, edges, paths, order and the
  tick, and it names no concrete node kind, no UI toolkit, no MIDI type, no
  render type. Its public surface is minimised deliberately; anything expressible
  with a Bevy built-in uses the built-in.
- A node-domain crate is self-contained: its node kinds, their ECS projections,
  and exactly one top-level plugin that registers every type, system and resource
  the domain needs. `sway-midi` is the shape to copy.
- Dependency direction is one way: engine ← domain crates ← host. No domain crate
  depends on another domain crate.

**`sway-graph` surface reduction:**

- **BREAKING** `Node::pos` / `set_pos` are replaced by a generic per-node
  `metadata` map (`HashMap<String, Box<dyn PartialReflect>>`). The editor writes
  `"pos"` into it as a `Vec2`; the graph never interprets a key. The document
  serializes the map with untyped reflection in place of the dedicated `pos`
  field, so the document names no editor concern either. **The format breaks**:
  `FORMAT_VERSION` goes to 4 and version-3 files are refused by version. The one
  file affected — `demo.sway.ron` — is rewritten as part of this change.
- **BREAKING** `GraphCommand`, `CommandOutcome`, `apply_graph_command` and
  `apply_graph_commands` are deleted. The graph is mutated through its own
  methods; `create` and `set_field` are added to cover the two commands that did
  work no method did, and `FieldWrite { Written, Unchanged, Rejected }` replaces
  `CommandOutcome`. Deferring an edit past a widget's event dispatch is an editor
  problem, so the editor gets its own `EditorEdit` payload for its channel.
- **BREAKING** `Graph::selection` / `set_selection` are removed. Selection is
  editor state and moves to a `sway-editor`-owned resource.
- **BREAKING** A field write carries `&dyn PartialReflect` instead of a
  `FieldValue`. The `FieldValue` enum and the ~90 lines of float/int narrowing it
  needed leave the engine for the inspector, which is the only thing that knows
  what a widget produced. `sway-graph` stops naming `Vec2`, `Vec3` and `Quat`.
- **BREAKING** `viewport_input` (the masonry→Bevy pointer/key/scroll vocabulary)
  leaves `sway-graph` for a new `sway-viewport-input` crate, shared by the editor
  that produces the events and the viewport that consumes them.
- `GraphRx` and the channel drain leave the engine with it: the boundary that
  owns a channel owns the plumbing. The gizmo and picker, which already hold
  `ResMut<Graph>`, stop round-tripping through a payload entirely.
- `Compat` and `Target` — two enums for the same three-way direct/optional/
  variadic distinction — collapse into one. `NodeParts`/`PartType` type data is
  dropped in favour of reading the part's type off `TypeInfo` at the call site.
  Items with no caller outside the crate (`EvalOrder`, `GraphStep`,
  `PropagateStep`, `Link`, `Sorted`, `topological_order`, `absolute_path`,
  `compatibility_of_values`) become crate-private.
- `sway-graph` drops its `crossbeam-channel`, `bevy_transform` and `bevy_time`
  dependencies (the last two are already unused outside tests).
- The tick test harness — currently copy-pasted into `sway-nodes`, `sway-editor`
  and `sway-runtime` because `graph::testing` is `#[cfg(test)]` — becomes a
  `test-support` feature of `sway-graph`.

**Crate restructuring:**

- **BREAKING** `sway-nodes` becomes `sway-base-nodes`: one domain crate, one
  top-level `BaseNodesPlugin`, holding `Oscillator`, `Envelope`, `Math`,
  `Remap` and `MakeVec3`. The pure-helper modules fold into the node modules
  they serve.
- **BREAKING** The `Vec3` node kind is renamed `MakeVec3` (`Vec3In`/`Vec3Out` →
  `MakeVec3In`/`MakeVec3Out`). Its behaviour is unchanged; the old name collided
  with `bevy::math::Vec3`, which the node's own outlet is made of, forcing an
  alias at every use. Since a document keys node kinds by short name, this
  changes `type: "Vec3"` on disk.
- **BREAKING** `Envelope` takes `time` as an inlet, matching `Oscillator`, instead
  of reading `Time<Fixed>` from the world. `sway-base-nodes` then has no
  `bevy_time` dependency and every base node is a pure function of inlets and
  state.
- `sway-runtime/src/viewport/` (camera, gizmo, picking — editor-only interaction,
  ~2200 lines) becomes `sway-editor-viewport`. `sway-runtime` is left as the
  runtime domain: headless app, render-coupled node kinds, projectors.
- `sway-runtime`'s split modules are folded: `sprite_material.rs` +
  `nodes/sprite_material.rs` into one, `frame_sequence.rs` +
  `nodes/frame_sequence.rs` into one.
- Dead code removed: `PointCloudPlugin`, `ScatterPlugin`, `SpriteLayerPlugin`
  (exported, added by no app), `beat.rs`, `NoteField`, and `tests/traces.rs`
  (golden traces of pure functions from the deleted wire model).
- Unused dependency edges removed: `sway-midi → sway-nodes`,
  `sway-geo → sway-graph`, `sway-runtime → sway-nodes`,
  `sway-editor → sway-nodes` (dev).

## Capabilities

### New Capabilities

None. The rules land as requirements on the existing `architecture` domain.

### Modified Capabilities

- `architecture`: adds the source-structure requirements — what the engine crate
  may know, what a node-domain crate owns, the single-plugin rule, and the
  permitted dependency direction. Authoring reaches the graph through one
  mutation surface — the graph's own operations — rather than a second
  vocabulary restating them as data.
- `graph`: a node carries typed, uninterpreted annotations rather than a canvas
  position; the graph holds no selection; a field write carries a reflected value
  rather than a fixed set of value types.
- `editor`: the editor owns selection and node canvas placement, reading and
  writing them as its own state rather than as graph fields, and converts a
  control's value to the edited field's type before the edit reaches the graph.
- `document`: a node entry stores typed, uninterpreted annotations in place of a
  dedicated editor-position field, giving no key a field of its own, reporting
  and skipping one whose type is unregistered, and writing them in a stable
  order; the format version goes to 4 and version-3 files are refused by version.
- `nodes`: a node kind is named for what it does rather than for the type it
  produces; a vector inlet is connectable whole or per component; and every base
  node is a pure function that takes its time as an inlet.

## Impact

**Crates added:** `sway-viewport-input`, `sway-editor-viewport`.
**Crates renamed:** `sway-nodes` → `sway-base-nodes`.
**Crates deleted:** none.

**Code:** `sway-graph` (every module), `sway-editor` (selection, inspector
coercion, canvas metadata, viewport input import), `sway-document` (node metadata
in place of `pos`, format version 4, `v3` module renamed to `v4`),
`sway-runtime` (viewport extraction, module folding, dead pipeline removal),
`sway-midi` / `sway-geo` (manifest only), `sway-app` (plugin wiring, new crate
imports).

**Assets:** `crates/sway-app/assets/demo.sway.ron` is rewritten — `version: 4`,
annotations in place of `pos` on all 25 nodes, and `type: "Vec3"` →
`type: "MakeVec3"` on its two vector nodes.

**Docs:** `docs/architecture.md` gains §11 "Source structure"; §8's crate layout
and §5's ownership table are corrected. §10's `Vec3`-node decision stands and is
only renamed.

**Risk:** this is a wide mechanical refactor with no behavioural target of its
own. The graph golden traces, the document round-trip tests and the projector
tests are the safety net; the demo project (`demo.sway.ron`) must still load,
tick and render unchanged.
