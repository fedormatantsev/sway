## Why

`MidiNotes` publishes the tick's note occurrences on one channel, but nothing in the app reads them: there is still no generic event other domains can consume, and no node that turns a chosen pitch into one. Envelope and Oscillator meanwhile are two special-case time nodes that should be the same thing — a curve sampled at a time — with a timer that resets on trigger covering the envelope case.

## What Changes

- Add a unit **`Trigger`** payload in `sway-base-nodes`: the generic occurrence other domains fire and consume. This is the D11 follow-up from `add-event-channels` — the payload lives with the generic signal nodes, not in `sway-events` and not in a new crate.
- **`MidiNotes` selects by channel.** It gains a `channel` inlet (protocol 0–15, same rule as `MidiCc`) and publishes only that channel's note-on and note-off messages. It still does not pick a pitch.
- Add an **`OnMidiNote`** node in `sway-midi` that reads `MidiNotes`'s outlet, matches an authored **note name string** (`D#1`, `C4`), and publishes `pressed` / `released` as `Trigger` batches. It has no channel inlet.
- Add a **`Timer`** node in `sway-base-nodes`: inlets `time: f32` and `trigger` (`Trigger` handle). It accumulates elapsed time in the inlet's units and resets to zero on any trigger. `MidiTime` can drive `time`.
- Add a **`CurveSampler`** node in `sway-base-nodes` with inlets `time: f32` and `keys`. Time is clamped to the keys' x-range (not wrapped). Envelope becomes Timer (reset on trigger) → CurveSampler.
- **BREAKING:** remove the `Oscillator` and `Envelope` node kinds. Documents that name them will not load. Rewrite the demo graph's oscillators as `CurveSampler` nodes. ADSR-as-a-gated-node goes away: sustain-while-held and release-on-gate-off are authored as graph (a second sampler on `released`, or a curve that ends at zero), not as Envelope inlets.

## Capabilities

### New Capabilities

- (none)

### Modified Capabilities

- `nodes`: add the `Trigger` payload, the `Timer` node, and the `CurveSampler` node; remove `Oscillator` and `Envelope`; a base node whose inlet is a handle may resolve it through the occurrence arena
- `midi`: `MidiNotes` filters by channel; add `OnMidiNote` — the converter that picks a pitch by note name and fires generic `Trigger`s, without any other domain naming a MIDI note type
- `architecture`: `sway-base-nodes` is the generic signal layer other node domains may depend on for shared vocabulary (`Trigger`); peer domains still must not depend on each other
- `events`: a domain that converts into the generic `Trigger` vocabulary may depend on `sway-base-nodes` for that payload; the occurrence crate remains the mechanism, and peer domains still do not depend on each other

## Impact

- **`sway-base-nodes`**: depends on `sway-events`; gains `Trigger`, `Timer`, `CurveSampler`; drops `Oscillator` and `Envelope`. `BaseNodesPlugin` registers the new kinds, `Trigger`, and `EventHandle<Trigger>`.
- **`sway-midi`**: depends on `sway-base-nodes` so `OnMidiNote` can name `Trigger`. `MidiNotes` gains a channel inlet. Gains `OnMidiNote` with a string `note` inlet; `MidiPlugin` registers the new parts.
- **`sway-app`**: demo document rewritten (`Oscillator` → `CurveSampler`). Plugin order already adds both domains.
- **`sway-document`**: no format change. Tests that name `Oscillator` as a kind string follow the demo.
- **`sway-events`**, **`sway-graph`**: untouched. The handle and arena already do this job.
- **`docs/architecture.md`** and **`docs/roadmap.md`**: crate-layout and §3 producer/converter wording; the two backlog items this change implements come off the roadmap.
- Out of scope: a curve inspector, sub-tick retrigger using `NoteEvent.offset`, a held velocity outlet, event-driven scheduling, a dedicated merge node (variadic `Vec<EventHandle<Trigger>>` already merges), named waveforms, and wrap/modulo on CurveSampler.
