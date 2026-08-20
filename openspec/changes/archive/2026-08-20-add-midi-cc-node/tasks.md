## 1. CC snapshot resource (`sway-midi`)

- [x] 1.1 Add a `MidiControls` resource: 16×128 last raw CC values, default 0, with a lookup that truncates and clamps `channel` / `cc` the same way `MidiCc` will
- [x] 1.2 `init_resource::<MidiControls>()` from `MidiPlugin`
- [x] 1.3 In `drain_and_clock`, after filling `TickMidi`, write every `MidiMessage::Control` in that tick into `MidiControls` (last write wins)
- [x] 1.4 Plugin test: push a Control into `MidiInbox`, `app.update()`, assert the snapshot cell is the raw value; a second Control on the same cell overwrites it; a Control on a different cell is ignored by the first

## 2. `MidiCc` node (`sway-midi`)

- [x] 2.1 Add `nodes/midi_cc.rs`: `inlets: MidiCcIn { channel: f32, cc: f32 }`, `state: ()`, `outlets: MidiCcOut { out: f32 }`; `evaluate` reads `MidiControls` and writes `raw as f32 / 127.0` (missing resource → 0). `MidiCcIn` needs a hand-written `impl Default` (channel `0.0`, cc `1.0`) — a derived one would give cc `0`. Mark it `#[reflect(Default, Debug, PartialEq)]` as `CameraIn` / `OutputIn` do; `MidiTime` predates that convention and is no longer the whole template
- [x] 2.2 Export from `nodes/mod.rs` and `lib.rs`; `MidiPlugin` registers `MidiCc`, `MidiCcIn`, `MidiCcOut`
- [x] 2.3 Node tests (seeded `MidiControls`, no inbox): matching 127 → 1; 64 → 64/127; missing resource → 0; channel 20 / cc 200 indexes 15 / 127; two nodes with the same inlets publish the same outlet
- [x] 2.4 Plugin test: Control through the inbox, then tick a freshly inserted `MidiCc` — first evaluation already holds the value (`midi`: a new node sees the session's last matching value)

## 3. Docs and verify

- [x] 3.1 Update `docs/architecture.md` so the MIDI domain lists `MidiCc` beside `MidiTime`: the ownership-table row (`| Beat / transport snapshot + MidiTime |`) and the **Supporting crates** paragraph (`sway-midi` (Bevy MIDI plugin, transport snapshot, and `MidiTime` as an ordinary node)`)
- [x] 3.2 While in that paragraph: it still calls the base-node crate `sway-nodes`, renamed to `sway-base-nodes` by `source-structure-rules`. Fix that one mention. Leave §11's "which is why `sway-nodes` became `sway-base-nodes`" alone — that one is history and already reads correctly
- [x] 3.3 `cargo test -p sway-midi`
