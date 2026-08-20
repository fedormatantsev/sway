## Why

On stage, MIDI is the only live input. The graph already exposes transport position through `MidiTime`, but a Control Change from a fader or encoder has nowhere to land: CC messages are parsed and then ignored. A held CC parameter is the missing counterpart to `MidiTime` — the authorable knob that drives a scene from a hardware controller.

## What Changes

- Add a `MidiCc` node kind in `sway-midi`: authored `channel` and `cc` inlets, one `f32` outlet that holds the last matching Control Change as a 0–1 parameter.
- Keep a session-wide CC snapshot in the MIDI plugin (same role `Transport` plays for time). `MidiCc` reads that snapshot during evaluation; it does not scan the tick's event list itself.
- Register the new kind and its part types from `MidiPlugin`, so a host that already adds that plugin gets the node in the palette, the document, and evaluation with no extra wiring.
- No editor, graph-engine, or document-format change: `f32` inlets and outlets already have inspector controls, connect legality, and serialization.

## Capabilities

### New Capabilities

- `midi`: MIDI as an authorable graph domain — nodes that read the live MIDI snapshot and publish values other nodes may consume. This change introduces the domain with `MidiCc`; `MidiTime` already exists in code and is not restated here.

### Modified Capabilities

- (none)

## Impact

- `sway-midi`: new node module, a CC snapshot resource filled during the existing drain, registration in `MidiPlugin`.
- `sway-midi-core`: unchanged. `MidiMessage::Control` is already parsed.
- Palette, inspector, canvas, and document pick the kind up through the type registry; `sway-editor` and `sway-document` stay untouched.
- Out of scope: 14-bit CC / NRPN, MIDI learn, note/velocity/aftertouch/pitch-bend nodes, CC as a trigger/event rather than a held value.
