## 1. CC snapshot resource (`sway-midi`)

- [ ] 1.1 Add a `MidiControls` resource: 16×128 last raw CC values, default 0, with a lookup that truncates and clamps `channel` / `cc` the same way `MidiCc` will
- [ ] 1.2 `init_resource::<MidiControls>()` from `MidiPlugin`
- [ ] 1.3 In `drain_and_clock`, after filling `TickMidi`, write every `MidiMessage::Control` in that tick into `MidiControls` (last write wins)
- [ ] 1.4 Plugin test: push a Control into `MidiInbox`, `app.update()`, assert the snapshot cell is the raw value; a second Control on the same cell overwrites it; a Control on a different cell is ignored by the first

## 2. `MidiCc` node (`sway-midi`)

- [ ] 2.1 Add `nodes/midi_cc.rs` mirroring `MidiTime`: `inlets: { channel, cc }` (defaults 0 and 1), `state: ()`, `outlets: { out: f32 }`, `evaluate` reads `MidiControls` and writes `raw as f32 / 127.0` (missing resource → 0)
- [ ] 2.2 Export from `nodes/mod.rs` and `lib.rs`; `MidiPlugin` registers `MidiCc`, `MidiCcIn`, `MidiCcOut`
- [ ] 2.3 Node tests (seeded `MidiControls`, no inbox): matching 127 → 1; 64 → 64/127; missing resource → 0; channel 20 / cc 200 indexes 15 / 127; two nodes with the same inlets publish the same outlet
- [ ] 2.4 Plugin test: Control through the inbox, then tick a freshly inserted `MidiCc` — first evaluation already holds the value (`midi`: a new node sees the session's last matching value)

## 3. Docs and verify

- [ ] 3.1 Update `docs/architecture.md` ownership table (and the MIDI plugin paragraph) so the MIDI domain lists `MidiCc` beside `MidiTime`
- [ ] 3.2 `cargo test -p sway-midi`
