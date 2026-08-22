## Why

Every value a wire carries today is a *level*: the tick copies a field, and a node reads whatever stands there this tick. Anything that *happens* — a note on, a beat boundary, a one-shot retrigger — has no way through the graph. `Envelope` already documents the cost: its gate is a boolean inlet sampled once per tick because "routing individual MIDI events onto graph inlets is `sway-events` territory, still out of scope". The graph redesign was done ahead of M9 specifically to leave room for events; this change takes that room.

The old `sway-events` sketch in `docs/architecture.md` §3 does not survive the redesign — it is built on `TriggerOut<P>` components, `Relationship` wires and per-wire `TriggerIn<W>` buffers, none of which exist in a model where a node is one reflected value and an edge is data. What the current model already has is exactly what an event wire needs: a field type, exact-type connect legality, `Option`/`Vec` wrappers for optional and variadic inlets, and a topological order that puts every producer before its consumers. `sway-events` keeps its name and its job; only its contents change.

## What Changes

- Add a new crate **`sway-events`** holding an **occurrence arena** — a `World` resource that holds this tick's batches of occurrences — and `EventHandle<P>`, a small payload-typed value naming one batch.
- The arena has two operations: **publish** a whole batch and get a handle back, and **read** a handle to get that batch. Batches are refcounted, so a read hands back an owned share rather than a borrow of the arena — nothing is locked, nothing is borrowed across a call, and there is no `unsafe` anywhere in the crate.
- A **producer** publishes during its own `evaluate`: it hands its occurrences to the arena and writes the returned handle to its own outlet. It keeps **no state** — no buffer, no handle, nothing between ticks. With nothing to publish it writes the **empty handle**.
- A **consumer** reads the handle standing on its inlet and gets that batch back. There is no write path from a handle, so the read and write sides are separated by construction rather than by discipline.
- The handle travels the wire, so fan-out costs nothing: every consumer of an outlet holds the same handle and reads the same batch, and connect legality, optional inlets and variadic merge all come from the existing rule — `EventHandle<P>` → `EventHandle<P>` direct, `Option<…>` optional, `Vec<…>` a variadic merge.
- The arena is **emptied completely before every tick** by the new crate's one plugin, scheduled before `GraphTickSet`. A handle from an earlier tick is stale and reads as no occurrences — never as another producer's batch — so publishing afresh each tick is the producer's job and nothing accumulates.
- **`sway-graph` is not touched.** The handle is an ordinary reflected field value and the arena is an ordinary world resource, so the engine needs no knowledge of either; `sway-events` depends on it only for `GraphTickSet`.
- `sway-app` adds the plugin beside `GraphPlugin`. Fixtures and behaviour tests live in `sway-events`, over a real `Graph` driven by `sway-graph`'s `test-support` harness.
- Add the first real producer: a **`MidiNotes`** node in `sway-midi` that publishes the tick's note-on and note-off messages as one batch — channel, note, velocity, on/off and the sub-tick offset the MIDI drain already records — and leaves the empty handle on a silent tick. It selects nothing: every note message of the tick is published, and choosing among them belongs to the converter nodes that come later.

## Capabilities

### New Capabilities

- `events`: occurrences carried through the graph's ordinary wires — what a handle is, how a producer publishes a batch and a consumer reads one, that fan-out shares a batch rather than copying it, that the arena is emptied before every tick and a stale handle reads empty, what publishing means for change tracking, and that all of it is one crate with one plugin the engine does not depend on.

### Modified Capabilities

- `midi`: adds note events as a MIDI domain node — what `MidiNotes` publishes each tick, that a zero-velocity note-on is a note-off, that nothing is selected or held between ticks, and that the note payload stays the MIDI domain's own vocabulary.
- `document`: a handle-typed inlet is session state; saving names no batch and loading restores the empty handle, the same rule that already applies to a loaded asset's identity. No document code changes — the guarantee is pinned by tests in `sway-events`.

## Impact

- **`sway-events` (new crate)**: the arena and its handle, payload registration, the clear system and its set, and `EventsPlugin`. Depends on `bevy_app`, `bevy_ecs`, `bevy_reflect`, `serde`, and `sway-graph` for `GraphTickSet` alone — it never looks at a node, an edge, or the graph. `serde` is a dependency `sway-graph`'s manifest deliberately does not carry, which is one more reason none of this belongs there.
- **`sway-graph`**: unchanged. Verified by a task, not by intention.
- **`sway-app`**: one line — `sway_events::EventsPlugin` beside `sway_graph::GraphPlugin`.
- **`sway-document`**: unchanged, and no format change. The round-trip guarantee is exercised from `sway-events` with `sway-document` as a dev-dependency.
- **`sway-editor`**: untouched. Sockets and connect legality come from reflection, so a trigger socket appears on its own; a handle field falls through to the existing read-only inspector control.
- **`sway-midi`**: gains a `sway-events` dependency, the `NoteEvent` payload, the `MidiNotes` node, and their registration in `MidiPlugin`. The MIDI drain already fills `TickMidi` with each message and its offset before `GraphTickSet`, so the node reads what is already there during its own evaluation — no new system and no new ordering constraint.
- Other node domains: unchanged. A domain that wants triggers depends on `sway-events`, registers its own payload type, and declares handle fields.
- **Change tracking**: a node that publishes writes a new handle every tick, so it and everything its handle reaches are reported changed on every tick they carry occurrences. Silent ticks (the empty handle) report nothing. This is a real consequence of one-tick handles and is specified rather than worked around.
- Out of scope, and deliberately: **nothing in the app reads note occurrences yet.** The converter nodes that turn notes into a vocabulary other domains understand (`OnNotePressed` and its kin, firing the generic events that will live in `sway-base-nodes`), an event-driven `Envelope` gate, event-driven scheduling (evaluating only the nodes a batch reaches), a payload-carrying inspector view, and cross-tick queues are all follow-up work. This change lands the mechanism, the first producer, and the tests for both; the read side stays exercised by `sway-events`' own fixtures until those converter nodes land.
