## 1. `Trigger` and crate deps (`sway-base-nodes`)

- [x] 1.1 Add `sway-events` to `sway-base-nodes` dependencies. Manifest comment: this crate owns the generic `Trigger` payload and may resolve handle inlets through the arena; it still reads no clock and no MIDI
- [x] 1.2 `nodes/trigger.rs`: unit `Trigger` (D1) — `Reflect`, `Default`, `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`. Export from `nodes/mod.rs` and `lib.rs`
- [x] 1.3 `BaseNodesPlugin` registers `Trigger` and `register_event_handle::<Trigger>()`. Comment: a host that adds this plugin registers neither on the domain's behalf (`nodes`: Trigger is the generic occurrence payload)

## 2. `CurveSampler` (`sway-base-nodes`)

- [x] 2.1 `nodes/curve_sampler.rs`: piecewise keys only. Public `curve_sampler_value(time, keys)`. No `CurveShape`, `period`, `phase`, or `amplitude`
- [x] 2.2 `CurveSamplerIn { time, keys: CurveKeys }` — `CurveKeys` is an opaque `Vec<Vec2>` so the engine does not truncate it as a variadic inlet. `state: ()`. `CurveSamplerOut { out: f32 }`
- [x] 2.3 `evaluate`: empty keys → 0; otherwise sort by `x`, clamp `time` to `[min_x, max_x]`, linear interpolate (`nodes`: A CurveSampler samples a curve at a time)
- [x] 2.4 Tests: (0,0)→(1,1) at 0.5 is 0.5; at 2.0 is 1; at −1 is 0; empty is 0; driven `time` reaches the outlet in one tick; equal write does not dirty
- [x] 2.5 Register `CurveSampler` and its parts (and `CurveKeys`) in `BaseNodesPlugin`; export from `nodes/mod.rs` and `lib.rs`

## 3. `Timer` (`sway-base-nodes`)

- [x] 3.1 `nodes/timer.rs`: `TimerIn { time: f32, trigger: Vec<EventHandle<Trigger>> }`, `TimerState { origin: f32, primed: bool }`, `TimerOut { out: f32 }` (D4)
- [x] 3.2 `evaluate`: no arena → accumulate only. First evaluate latches `origin = time`. Any occurrence on any trigger handle relatches `origin = time` this tick. Outlet `(time - origin).max(0.0)`. Keep neither occurrences nor handles in state (`nodes`: A handle inlet is resolved, not stored)
- [x] 3.3 Tests over `trace_world` + an inserted `EventArena`: time 0 then 4 with empty trigger → outlet 4; publish a Trigger, write the handle onto `trigger` (slot 0), same-tick outlet 0 and further advances from that time; two handles on the variadic inlet, either resets; no arena still evaluates and accumulates; two Timers with identical inlets and state produce identical outlets
- [x] 3.4 Register `Timer` and its parts in `BaseNodesPlugin`; export from `nodes/mod.rs` and `lib.rs`
- [x] 3.5 Unique-short-name test lists `CurveSampler` and `Timer` instead of `Oscillator` and `Envelope`

## 4. Remove `Oscillator` and `Envelope` (`sway-base-nodes`)

- [x] 4.1 Delete `nodes/osc.rs` and `nodes/envelope.rs`. Drop `oscillator_value`, `adsr_unscaled`, `EnvelopeParams`, `Waveform` from `lib.rs` exports. Retarget `math.rs`'s oscillator chain test at `CurveSampler`
- [x] 4.2 `cargo test -p sway-base-nodes`

## 5. `MidiNotes` channel filter (`sway-midi`)

- [x] 5.1 `MidiNotesIn { channel: f32 }` default 0. Truncate toward zero, clamp 0–15. `evaluate` keeps only note messages on that channel. Register `MidiNotesIn` on `MidiPlugin`. Comment: the node selects a channel and still publishes every pitch (`midi`: A MIDI notes node publishes the tick's note events)
- [x] 5.2 Replace the "every channel is published" test: two `MidiNotes` on channels 0 and 9 each publish only their channel; a node on channel 0 ignores channel 9. Add a clamp test (`channel: 20` matches 15). Existing arrival-order / zero-velocity / empty-handle tests seed messages on the node's channel

## 6. `OnMidiNote` (`sway-midi`)

- [x] 6.1 Add `sway-base-nodes` to `sway-midi` dependencies (D2). Manifest comment: this crate depends on the generic signal layer for `Trigger` only; it does not depend on runtime or any other peer domain
- [x] 6.2 `parse_note_name(&str) -> Option<u8>`: scientific pitch, `C4` = 60, `D#1` = `Eb1`, case-insensitive letter, trim whitespace, `C-1` = 0, out of 0–127 is `None`. Unit tests for those cases and for `not-a-note` → `None`
- [x] 6.3 `nodes/on_midi_note.rs`: inlets and outlets as D3. Default `note: "C4"`. No channel field. `evaluate` reads the notes handle, matches `parse_note_name`, publishes matching note-ons to `pressed` and note-offs to `released` in arrival order, `EMPTY` independently when a side has nothing. Missing arena / empty inlet / unparseable name → both `EMPTY`. `state: ()`
- [x] 6.4 Export from `nodes/mod.rs` and `lib.rs`. `MidiPlugin` registers `OnMidiNote`, `OnMidiNoteIn`, `OnMidiNoteOut` — **not** `Trigger` (D2, `architecture`: a domain does not leak registration)
- [x] 6.5 Node tests: `C4` + MIDI 60 note-on → one pressed Trigger, released empty; matching note-off → the reverse; MIDI 64 vs authored `C4` → both empty; `D#1` matches that pitch; two matching note-ons → two pressed Triggers in order; `not-a-note` both empty; unconnected inlet both empty; no arena both empty (`midi`: An OnMidiNote node converts notes into pressed and released triggers)
- [x] 6.6 Chain test: `MidiNotes` (channel 0) → `OnMidiNote` (`C4`) → `Timer.trigger` with a driven `time` — a matching note-on zeros the Timer in the same tick (`nodes`: An envelope is a Timer into a CurveSampler — the first hop)
- [x] 6.7 `cargo test -p sway-midi`

## 7. Demo and document fixtures

- [x] 7.1 Rewrite `crates/sway-app/assets/demo.sway.ron`: `lfoA`, `lfoB`, `spriteOsc`, `spriteOsc2` become `CurveSampler` with piecewise keys; `lfoB.amplitude` becomes a `Math` multiply. Header comments follow
- [x] 7.2 Update `sway-document` tests that name kind `"Oscillator"` (v4 doc fixtures) to `"CurveSampler"`
- [x] 7.3 `cargo test -p sway-document`

## 8. Docs and verify

- [x] 8.1 `docs/architecture.md`: crate-list row for `sway-base-nodes` names CurveSampler, Timer, Trigger (not Oscillator / Envelope); `sway-midi` lists `OnMidiNote` and a channel-filtered `MidiNotes`; §3 converter wording — `OnMidiNote` matches a note name, `Trigger` lives in `sway-base-nodes`, MIDI depends on that crate one way. Supporting-crates paragraph matches
- [x] 8.2 `docs/roadmap.md`: remove the note-to-event converter item and the CurveSampler/Envelope/Oscillator item (this change implements both)
- [x] 8.3 **Verify `sway-graph` and `sway-events` are untouched**: `git diff --stat -- crates/sway-graph crates/sway-events` is empty
- [x] 8.4 Confirm `sway-midi` does not depend on `sway-runtime` and `sway-base-nodes` does not depend on `sway-midi` (`architecture`: A converter depends on the generic layer, not the other way around)
- [x] 8.5 `cargo test -p sway-base-nodes -p sway-midi -p sway-document`, then `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
