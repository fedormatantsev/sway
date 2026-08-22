## Context

See `proposal.md` — Why. Constraints that shape the approach:

- **`MidiNotes` already publishes `EventHandle<NoteEvent>`.** This change gives it a channel inlet; it still does not pick a pitch. `NoteEvent` stays MIDI vocabulary (`midi`: No other domain names a MIDI note). The converter that crosses the boundary lives in `sway-midi`.
- **Connect legality is exact type.** `OnMidiNote` can name `Trigger` only if `sway-midi` depends on the crate that owns it. `architecture` previously forbade domain→domain; this change revises that so `sway-base-nodes` is the generic signal layer other domains may depend on.
- **A base node is a pure function of inlets and state**, except that resolving a handle inlet reads the occurrence arena (`nodes` modified). Timer still takes time as an inlet — it does not read `Time<Fixed>` or `Transport`.
- **`evaluate(&mut self, world: &World)`** is how `OnMidiNote` and `Timer` reach `EventArena`. No arena is the empty handle / no reset, not a failed evaluation — the same rule `MidiNotes` already uses.
- **The demo document names `Oscillator`.** Removing the kind is a load break; the demo is rewritten in this change. There is no format version bump: the kind string is data, not a schema version.
- **`NoteEvent.offset` is seconds from the tick start.** `MidiTime` is PPQ. Mixing them on a Timer reset would be a unit error, so this change does not forward the offset.

## Goals / Non-Goals

**Goals:**

- One generic payload (`Trigger`) that any domain can fire and any base node can consume, without a new crate.
- A MIDI converter that picks a pitch by note name from a channel-filtered `MidiNotes` stream and speaks only Trigger on its outlets.
- Oscillator and Envelope as graphs over `CurveSampler` and `Timer`, not as node kinds. Named looping waveforms are out of this node: CurveSampler clamps time to its keys.

**Non-Goals:**

- A curve inspector, or any new inspector widget for the key list.
- Sub-tick retrigger from `NoteEvent.offset`.
- A held velocity outlet, an omni-channel `MidiNotes`, a numeric note-number inlet on `OnMidiNote`, or converters for CC / clock / transport events.
- Event-driven scheduling. Every node still evaluates every tick.
- **`Math` gaining a modulo op.** Wrap is not this change.
- Changing `sway-graph` or `sway-events`.

## Decisions

### D1: `Trigger` is a unit struct in `sway-base-nodes`

```rust
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger;
```

It is a payload, not a node. `BaseNodesPlugin` registers `Trigger` and `EventHandle<Trigger>`. Publishing writes `Trigger` values into the arena; the handle on the wire is still `EventHandle<Trigger>`.

*Alternative rejected — `Trigger` in `sway-events`.* The occurrence crate is the mechanism, not a vocabulary owner. Parking a domain payload there would make every future generic event a reason to grow that crate.

*Alternative rejected — a new vocabulary crate.* Correct under the old "neither domain owns it" rule, and oversized for a unit struct the user placed in `sway-base-nodes`.

### D2: `sway-midi` depends on `sway-base-nodes`

`OnMidiNote` names `EventHandle<Trigger>` on its outlets. That is a domain→domain edge, deliberately: `sway-base-nodes` is the generic signal layer, not a peer of `sway-midi`. `sway-base-nodes` does not depend on `sway-midi`. `sway-runtime` and `sway-midi` still do not depend on each other.

`MidiPlugin` registers `OnMidiNote` and its parts. It does **not** register `Trigger` — that would leak another domain's type. Isolated `sway-midi` tests that need the handle in a registry call `register_event_handle::<Trigger>()` themselves, as `MidiNotes` tests already do for `NoteEvent`.

*Alternative rejected — host-only wiring, with `OnMidiNote` in `sway-app`.* The converter reads `NoteEvent`; `midi` already requires that converter to live in the MIDI domain.

### D3: `MidiNotes` selects the channel; `OnMidiNote` selects the pitch by name

`MidiNotes` gains `MidiNotesIn { channel: f32 }` (default 0), truncated toward zero and clamped to 0–15, the same rule as `MidiCc`. `evaluate` keeps only `NoteOn`/`NoteOff` whose channel matches. It still publishes every pitch on that channel — one `MidiNotes` still feeds every `OnMidiNote` that listens to it. Several channels are several `MidiNotes` nodes.

```rust
pub struct OnMidiNoteIn {
    pub notes: EventHandle<NoteEvent>,
    pub note: String, // default "C4"
}
pub struct OnMidiNoteOut {
    pub pressed: EventHandle<Trigger>,
    pub released: EventHandle<Trigger>,
}
```

`OnMidiNote` has **no channel inlet.** Matching is `parse_note_name(&note) == Some(event.note)`. The parser is scientific pitch: letter `A–G` (case-insensitive), optional `#` or `b`, integer octave (negative allowed, so `C-1` is MIDI 0); MIDI 60 is `C4`; `D#1` and `Eb1` are the same number. Surrounding whitespace is trimmed. Unparseable names and names outside 0–127 match nothing and do not fail evaluation.

Matching note-ons publish one `Trigger` each on `pressed`, matching note-offs on `released`, arrival order, two independent `publish` calls. Unmatched notes, an unconnected inlet, or a missing arena each leave `EMPTY` on both outlets.

`note` is a `String` because a performer authors a pitch, not a MIDI integer. It is authored-only: connect legality is exact-type, and nothing in the graph produces a note-name string. Several pitches are several `OnMidiNote` nodes on one `MidiNotes`.

`state: ()`. Velocity is dropped; a later node can hold it as an `f32` if a patch needs it.

*Alternative rejected — `note: f32` like `MidiCc.cc`.* Driveable, but the user-facing value is `D#1`, not 27.

*Alternative rejected — channel on `OnMidiNote` as well.* Then every converter repeats the same inlet, and a `MidiNotes` that still published every channel would force every consumer to filter. Channel belongs on the producer that already sits on the MIDI drain.

### D4: Timer latches an origin against the time inlet

```rust
pub struct TimerIn {
    pub time: f32,
    pub trigger: Vec<EventHandle<Trigger>>,
}
pub struct TimerState {
    pub origin: f32,
    pub primed: bool,
}
```

`trigger` is `Vec<EventHandle<Trigger>>` so several sources merge by the ordinary variadic rule (`events`: Several trigger sources merge on one inlet). Any occurrence on any handle resets.

On first evaluate, if not yet primed, `origin = time` and `primed = true`, so a Timer dropped onto a live `MidiTime` starts at 0 rather than jumping to the transport position. Any Trigger occurrence relatches `origin = time` on that same tick. Outlet is `(time - origin).max(0.0)` so a transport rewind does not go negative.

Unconnected `trigger` is an empty `Vec` — never resets, keeps accumulating. No arena: same, because every read is no occurrences.

*Alternative rejected — accumulate `dt = time - last_time`.* Equivalent while time is monotonic, worse on a rewind (negative dt) and an extra state field for the same origin-subtraction semantics.

*Alternative rejected — `origin` defaults to 0 with no prime.* A Timer created mid-song with `MidiTime` at 128 would outlet 128 until the first note, which is not "elapsed since the node began".

### D5: `CurveSampler` is piecewise keys at a clamped time

Inlets are `time: f32` and `keys` (`CurveKeys`, an opaque `Vec<Vec2>` so the engine does not treat them as a variadic inlet). No `period`, `phase`, `amplitude`, or named `shape`. `state: ()`.

Evaluate sorts keys by `x`, clamps `time` to `[min_x, max_x]`, and linearly interpolates. Empty keys yield 0. One key yields that y for every time. Time is never wrapped.

`Envelope` and `Oscillator` kinds, `adsr_unscaled`, `EnvelopeParams`, and `Waveform` / `CurveShape` are deleted. An envelope is `OnMidiNote.pressed → Timer.trigger`, `MidiTime → Timer.time`, `Timer.out → CurveSampler.time` with a piecewise curve. Sustain-while-held and release-on-note-off are a second sampler on `released`, not inlets on this node. Looping named waveforms are not this node.

The demo's former sine LFOs and saws are one-shot piecewise ramps that hold after the last key. Amplitude modulation is a `Math` multiply.

*Alternative rejected — wrap when `period > 0`, with named Sine/Saw shapes.* That re-created Oscillator as a mode of this node. Clamp-only keeps one job: sample the authored keys.

*Alternative rejected — wrap time in the keys' x-range.* The user required clamp, not wrap.

### D6: Do not forward `NoteEvent.offset`

Reset is tick-quantized. Applying a seconds offset to a PPQ time inlet would be a unit error, and Trigger is specified as a unit payload. The offset stays on `NoteEvent` for a later converter that speaks a time-aware payload.

## Risks / Trade-offs

- **[Risk] `sway-midi` → `sway-base-nodes` looks like the domain→domain edge architecture forbade.** → Mitigation: specified as a one-way generic-layer exception; peer domains still cannot depend on each other; recorded in `architecture` and `events` deltas.
- **[Trade-off] Gated ADSR is gone.** A one-shot curve from trigger is not sustain-while-held. → Accepted: that is the unification the roadmap asked for; a second sampler on `released` is the authored form of release.
- **[Trade-off] Piecewise keys are a raw `Vec<Vec2>` in the inspector.** → Accepted: composite inspector widgets are a separate backlog item.
- **[Trade-off] The demo no longer loops.** CurveSampler clamps; sine/saw LFOs become one-shot ramps that hold. → Accepted.
- **[Risk] Documents that still name `Oscillator` or `Envelope` will not load.** → Mitigation: no such document ships except the demo, which this change rewrites. No format version bump.
- **[Trade-off] One `OnMidiNote` is one pitch; one `MidiNotes` is one channel.** Patches that retrigger on any note of a channel need one converter per pitch. → Accepted: the string inlet is the authored form of that choice.
- **[Trade-off] A note-name string cannot be driven by the graph.** → Accepted: exact-type connect legality has nothing that produces `String` pitches, and the author types `D#1` in the inspector.

## Migration Plan

1. Land the new kinds and `Trigger` beside `Oscillator` / `Envelope`.
2. Rewrite `crates/sway-app/assets/demo.sway.ron` (`lfoA`, `lfoB`, `spriteOsc`, `spriteOsc2`) to `CurveSampler` with piecewise keys. Amplitude on `lfoB` becomes a `Math` multiply. Edges onto `time` / `out` stay except the removed amplitude inlet.
3. Delete `Oscillator` and `Envelope` and their tests; retarget any test that constructed them (`math.rs` chain, document kind-string fixtures, unique-short-name list).
4. Update `docs/architecture.md` crate list and §3 converter wording; remove the two backlog items this change implements from `docs/roadmap.md`.

No rollback path for saved documents that have already been rewritten. `Oscillator` / `Envelope` do not remain as aliases.

## Open Questions

None. Channel default 0, note-name default `C4` with scientific pitch (`C4` = MIDI 60), origin latching, piecewise hold-last, and the midi→base-nodes edge are decided above; changing any of them would change the specs.
