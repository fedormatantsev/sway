## Context

See `proposal.md` — Why. Constraints that shape the approach:

- `MidiTime` is the pattern: an ordinary node in `sway-midi` that reads a plugin-owned snapshot (`Transport`) from `&World` during `evaluate`. The graph engine names no MIDI type.
- `MidiPlugin` already drains the inbox into `TickMidi` before `GraphTickSet`. `PulseClock` consumes clock/transport messages and ignores `MidiMessage::Control`, which `sway-midi-core` already parses.
- Driveable numeric inlets elsewhere are `f32`, so they wire to `Math` / `Oscillator` / `Remap` without a new field type. Non-`f32` numeric inlets do exist (`Camera.resolution` is a `UVec2`), but they are authored-only: connect legality is exact-type, so nothing in the graph can drive them.
- Architecture: one plugin is the whole MIDI domain; `sway-midi` must not depend on `sway-base-nodes` or `sway-runtime`.

## Goals / Non-Goals

**Goals:**

- A `MidiCc` kind that is a close sibling of `MidiTime`: same crate, same plugin, same evaluate-reads-World shape.
- A CC snapshot filled on the existing drain path so last-write-wins is decided before any node evaluates.

**Non-Goals:**

- No change to pulse-clock math, `Transport`, or `MidiTime`.
- No new inlet types, editor controls, or document version.
- No per-node "learn" gesture and no 14-bit / NRPN packing.

## Decisions

### D1: Session-wide CC snapshot, not per-node event scanning

`drain_and_clock` writes matching `MidiMessage::Control` into a `MidiControls` resource (16×128 last raw values, default 0). `MidiCc::evaluate` indexes that table by its inlets and writes `raw as f32 / 127.0`.

This is how two nodes on the same controller stay in lockstep, and how a node created after a fader has already moved still publishes that position (`specs/midi`: a new node sees the session's last matching value). Scanning `TickMidi` from each node would miss history and could disagree if evaluation order ever interleaved with a later drain.

`TickMidi` stays the per-tick event list for the clock. CC does not need to remain in it after the snapshot is updated.

Alternative rejected: store last value in the node's `state`. Cheaper to write, but a freshly created node would stay at 0 until the next CC, which contradicts the spec.

### D2: Outlet is 0–1, not 0–127

`value / 127` so a fader wires straight into `Remap`, `Oscillator.amplitude`, or a scene scale component. Raw MIDI units would force a remap on every use. Authors who want 0–127 can multiply.

### D3: `channel` and `cc` are `f32` inlets

Same type as every other *driveable* numeric inlet, so they are inspector-editable **and** connectable. Evaluation truncates toward zero then clamps to 0–15 / 0–127. Defaults: channel `0`, cc `1` (mod wheel).

Alternative rejected: `u8` inlets. Not for want of a control — `reflect_ui` already offers a saturating text control for every integer width, and `Camera.resolution` is an authored `UVec2` inlet. The reason is connect legality: `sway-graph`'s rule is exact type match (plus `Option<S>` / `Vec<S>` wrappers), so no existing `f32` outlet — `Math`, `Remap`, `MidiTime` — could drive a `u8` inlet. A `channel` nothing can reach is a knob the graph cannot automate.

### D4: Channel numbers are protocol 0–15

`MidiMessage::Control` already uses the status nibble 0–15. The node matches that, not display 1–16. A 1–16 offset would silently miss every message from the parser we already have.

### D5: Snapshot lifetime is the MIDI plugin, not the project

Opening another project discards graph-derived world state. Controller position is live MIDI, not a project artifact: the snapshot is not cleared on project open. Reloading a project must not zero a fader that has not moved.

### D6: Tests live next to `MidiTime`

Unit tests on the node (world with a seeded snapshot) plus a plugin test that a Control message through the inbox updates the snapshot before the tick. No new golden trace file; existing MIDI traces stay transport-only.

## Risks / Trade-offs

- **[Risk] Authors expect channel 1–16.** → Mitigation: documented in the spec; inspector shows the raw inlet. A later display offset is a separate change.
- **[Risk] 7-bit only feels coarse on a high-res encoder.** → Mitigation: out of scope; 14-bit is a new node or an inlet, not a silent packing of this one.
- **[Trade-off] Session snapshot means project A can leak a CC value into project B.** → Accepted: that is the hardware's position, not a document field.

## Migration Plan

Additive. Existing documents keep working; `MidiCc` is opt-in from the palette. Rollback is reverting the `sway-midi` changes. Update the ownership row in `docs/architecture.md` so MIDI lists `MidiCc` beside `MidiTime`.

## Open Questions

None.
