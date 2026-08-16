# Sway — MIDI and transport redesign

**Date:** 2026-08-16
**Status:** Accepted
**Architecture:** [`docs/architecture.md`](../../architecture.md) is updated
when this ships. Until then this spec is the authority on MIDI/transport.

## Goal

Replace the current MIDI stack — a Bevy `Time<Transport>` integrator, a
mach-to-fixed offset filter, and an LFO that secretly reads the playhead —
with a JUCE-shaped split:

- Bevy-agnostic messages and clock math
- A Bevy plugin that is only a queue plus required resources (including the playhead)
- Beat phase that **snaps to the MIDI clock grid** and cannot drift

## Why the current stack goes

MIDI is split across four crates and does not match `architecture.md`.

`sway-midi` parses bytes and fits a line to pulses. `sway-nodes` owns
`MidiPlugin`, drains the inbox, and advances `Time<Transport>`. `sway-graph`
owns that clock so the engine is not MIDI-free. `sway-app` maps CoreMIDI host
time onto `Time<Fixed>` with a min-filter (`MidiClockOffset`) because the two
clocks walk independently.

Musical position is then **integrated**: each tick adds a duration of beats.
Start/SPP move an origin because `Time::advance_by` cannot rewind. Unlocked,
the system freewheels at the last tempo — drift by policy. Collapsed
`host_time == 0` stamps can lock tempo to frame rate, not 120 BPM.

The LFO reads `Time<Transport>` internally, so a generic oscillator cannot
take wall-clock or any other timebase.

JUCE does not do any of that. A plugin **reads** `ppqPosition` at the start of
the block. It does not count `0xF8` for phase, and it does not add `dt * bpm`.
Sway is the host, so it must *produce* that snapshot from MIDI clock; graph
nodes must only *read* it.

## Decisions

| ID | Decision |
|---|---|
| D1 | Two crates: `sway-midi-core` (no Bevy) and `sway-midi` (Bevy plugin). Today's `sway-midi` is renamed to `sway-midi-core`. |
| D2 | Musical position is `ppq = pulse_index / 24 + clamped interpolation`. Integer pulses are the grid. Tempo smoothing never moves the grid. |
| D3 | Dropout **holds** the last pulse. No skip-count, no freewheel. |
| D4 | `Transport` is a Bevy resource in `sway-midi`, replaced each tick. `Time<Transport>` is deleted. Core has no `Transport` type. |
| D5 | `Oscillator` in `sway-nodes` takes time as a wired float. It does not depend on MIDI. |
| D6 | `MidiTime` in `sway-midi` writes `FloatOut = ppq` as `f32`. That is the beat-time source. |
| D7 | `FloatOut` and `Vec3Out` move to `sway-graph` so `MidiTime` can write an outlet without depending on `sway-nodes`. |

## Crate layout

```
sway-midi-core     no Bevy
  CoreMIDI IO, StreamParser
  MidiMessage, HostTime
  PulseClock                     // push + ppq/bpm/playing/locked getters

sway-midi          Bevy plugin
  MidiInbox, TickMidi, MidiRx
  MidiClock                      // Resource: PulseClock + tick host window
  Transport                      // Resource: the playhead snapshot
  MusicalTime, MidiTime node
  feed → drain → clock → midi time, before graph_tick

sway-nodes         Oscillator (time/period/shape/phase/amplitude)
                   no sway-midi / sway-midi-core dependency

sway-graph         no beat clock, no Transport types
                   FloatOut, Vec3Out (D7)

sway-app           open_input + add_plugins(MidiPlugin)
sway-editor        readout from Res<Transport>
```

`sway-midi` depends on `sway-midi-core`, `sway-graph`, and Bevy. It re-exports
`open_input`, `list_sources`, `list_destinations`, and
`VIRTUAL_DESTINATION_NAME`, so `sway-app` depends only on `sway-midi`.
`sway-nodes` depends on neither MIDI crate.

## Data flow

```
CoreMIDI thread                     sway-midi-core              sway-midi (FixedUpdate)
───────────────                     ──────────────              ──────────────────────
read_proc                           MidiMessage                 feed → MidiInbox
  StreamParser                      PulseClock                  drain → TickMidi
  send (HostTime, msg) ───────────► (pure, stepped             clock → Res<Transport>
                                    with host timestamps)       MidiTime → FloatOut
                                                                graph_tick
                                                                Oscillator reads TimeFrom
```

The MIDI thread only sends. `PulseClock` is stepped with **host timestamps**,
not `Time<Fixed>`. `MidiClockOffset` is deleted.

`tick_end_host` is `host_time_now()` at drain. `tick_start_host` is the
previous tick's end. Drain every inbox event with `t <= tick_end_host`.
Lookahead beyond 0.5 s stays queued (DAW schedule); it is not collapsed to
now. `host_time == 0` means `tick_end_host`.

Catch-up (several `FixedUpdate`s in one frame): the first tick of the burst
takes the pending MIDI; later ticks in the same burst see an empty window and
the same host time, so `ppq` holds. Musical time follows the master, not the
fixed-tick accumulator.

## `sway-midi-core` types

### `MidiMessage`

Typed enum. Timestamp travels beside it as `HostTime` (mach ticks, convertible
to seconds with the existing `host_time_to_secs`).

```
NoteOn { channel, note, velocity }
NoteOff { channel, note, velocity }
Control { channel, cc, value }
Clock | Start | Continue | Stop
SongPosition { sixteenths: u16 }
Other { status, data1, data2 }
```

Zero-velocity note-on is `NoteOff` at the typed layer (same rule as today's
`note_message`). `StreamParser` is unchanged; `MidiMessage::from_bytes` sits
on top. SysEx stays dropped. `Other` is queued for nodes and ignored by
`PulseClock`.

`MidiEvent` / `RawMidi` as public types go away.

### `PulseClock`

The only object that turns clock messages into position and tempo. It has no
Bevy types. Tests and the plugin read it through getters (`ppq(t)`, `bpm()`,
`playing()`, `locked(t)`, `beats_per_bar()`). There is no `Transport` here.

```
frac   = (t - t_last) / secs_per_pulse
ppq(t) = pulse_index / 24 + frac.clamp(0.0, 1.0 - f64::EPSILON)
```

The fractional pulse is always in `[0, 1)`: interpolation never reaches the
next integer before that pulse arrives.

At each accepted `Clock` **while playing**: `pulse_index += 1`, `t_last = t`,
`ppq = pulse_index / 24` (exact snap).

The tempo smoother is a **separate** pulse train: every `Clock` feeds it,
playing or not, so BPM still tracks while stopped. Its inferred indices are
not `pulse_index` and must not move `ppq`.

| Message | Effect |
|---|---|
| Start | `playing = true`, `pulse_index = 0`, `ppq = 0`, `t_last = t` |
| Continue | `playing = true`, keep index and `ppq` |
| Stop | `playing = false`, freeze `ppq` |
| SongPosition | `pulse_index = sixteenths * 6`, `ppq = sixteenths / 4` |
| Clock while playing | increment, snap, tempo |
| Clock while stopped | tempo only; do not increment `pulse_index`; do not change `ppq` |

**Hold (D3).** Interpolation never crosses the next integer pulse. If the next
clock is overdue (`frac >= 1`), freeze at the clamp; do not invent pulses; do
not freewheel. A one-second cable drop holds the picture on the last grid
cell. The next real Clock / Start / SPP is the only thing that moves the
integer again.

**Tempo** is today's windowed least-squares slope (`WINDOW_PULSES = 48`,
`MIN_SAMPLES = 8`), used only as `secs_per_pulse`. Until the fit locks,
`secs_per_pulse` is the 120 BPM default (`0.5 / 24`). Duplicate timestamps
(`t <= last clock time`) are ignored by both the position grid and the tempo
smoother. The public `ClockEstimator` position API (`beats_at`) is deleted.

`locked` is true when a clock arrived within one expected pulse period of
`t`. Overdue interpolation does not clear playing; it clears `locked`.

## `sway-midi` plugin

The plugin does not estimate tempo or parse bytes. It owns the ECS types.

### `Transport`

The playhead. A snapshot resource, not a clock. Default: stopped, 120 BPM,
4/4, `ppq = 0`, unlocked.

```
ppq: f64              // pulse_index/24 + clamped frac; quarter notes
bpm: f64              // tempo estimate only
playing: bool
locked: bool          // a clock arrived within one expected pulse period
beats_per_bar: u32    // authored; MIDI clock has no time signature
```

`ppq` is MIDI/JUCE quarters. In 4/4 that is beats passed. Time signature does
not change the unit; it only affects `MusicalTime` display (bar wrapping).

`MusicalTime` (bar.beat.sixteenth, 1-based) lives next to it. Pure function of
`ppq` and `beats_per_bar`. The editor readout uses it.

### `MidiClock`

Plugin state: a `PulseClock` plus the host-time tick window (`tick_start_host`).
Not a newtype — it is the drain/clock system's memory.

**Resources:** `MidiRx`, `MidiInbox`, `TickMidi`, `Transport`, `MidiClock`.

**`MidiInput`** stays owned outside the world (today: `main`'s stack) so Drop
still disposes CoreMIDI before the sender is freed. The plugin takes the
`Receiver`.

**`FixedUpdate`, all before `graph_tick`:**

1. **feed** — `try_recv` into `MidiInbox`. No `Time<Fixed>` mapping.
2. **drain** — events with `t <= tick_end_host` become `TickMidi` entries
   `(offset_secs, MidiMessage)`. Offset = `t - tick_start_host`, clamped to
   `[0, tick_dt_host]`.
3. **clock** — `MidiClock.clock.push` for Clock/Start/Continue/Stop/SPP, then
   copy getters into `Res<Transport>` at `tick_end_host`.
4. **midi time** — every `MidiTime` entity: `FloatOut = Transport.ppq as f32`
   (`set_if_neq`).

`sway-app`'s `feed_midi` and `sway-nodes::MidiPlugin` are deleted.
`WiresPlugin` stops inserting `Time<Transport>`.

`TickMidi` remains for M9 (`MidiNote` / `MidiCC`). This spec does not add
those nodes.

### `MidiTime`

Zero-inlet authorable source. `#[require(FloatOut, EditorPos)]`. Palette name
`"MidiTime"`. A system, not a behaviour: it depends only on `Res<Transport>`,
so it runs before the tick (architecture §2 behaviour table).
`MidiTime → Oscillator.time` therefore lands in the same tick via `TimeFrom`.

`MidiTime` uses `sway_graph::EditorPos` and `sway_graph::FloatOut` (D7).
`TimeFrom` is a `sway-nodes` wire (`FloatOut → Oscillator.time`). `MidiTime`
only writes `FloatOut`; it does not register `TimeFrom`. `sway-midi` does not
depend on `sway-nodes`.

## Oscillator (`sway-nodes`)

Replace `Lfo`. Same wave function as `lfo_value` already is.

```
Oscillator { time, period, shape, phase, amplitude }
```

Phase is `fract(time / period + phase)` when `period > 0`, else authored
`phase`. `period` is in **units of the time inlet**, not “beats”. Wire
`MidiTime` and `period = 4` → one cycle per four MIDI quarters. A future
seconds source would make it Hz.

Unwired `time` stays at the authored float (default `0`) and the oscillator
holds still.

Wires: `TimeFrom` (`FloatOut → Oscillator.time`), keep `AmplitudeFrom`
(`FloatOut → Oscillator.amplitude`).

`sway-nodes` drops its `sway-midi` dependency. `beat.rs` free functions that
take `Time<Transport>` / origin-based beat ranges switch to `(prev_ppq, ppq,
playing, beats_per_bar)`. A relocate (Start/SPP) is a `ppq` jump: do not emit
every skipped boundary; reset boundary state. `BeatTrigger` as an authorable
node stays M9.

## Editor

`capture_transport` reads `Res<sway_midi::Transport>`. Position string is
`MusicalTime::from_ppq(ppq, beats_per_bar)`. Missing resource → default STOP
/ 120 / `001.1.1` / unlocked, same as today. `sway-editor` depends on
`sway-midi` for that resource type.

Inspector: `Lfo.beats` becomes `Oscillator.period`. Palette: `"Lfo"` →
`"Oscillator"`, plus `"MidiTime"`.

## Error handling

- Parser / FFI failures stay as today (OSStatus on open, skip SysEx, skip
  stray data bytes).
- Non-finite host times are ignored by `PulseClock`.
- `set_if_neq` on `MidiTime` / `Oscillator` so equal values do not dirty
  downstream work.
- `f32` `FloatOut` for `MidiTime`: at 120 BPM, two hours is ~14400 beats;
  sub-millisecond visual precision remains. Accepted.
- Tick drain is infallible: a NaN host conversion must not panic (same
  `max`/`min` discipline as today's `map_timestamp`).

## Testing

**`sway-midi-core` (no Bevy)**

- Parser tests stay.
- Packet-list / virtual-destination tests stay, asserting `MidiMessage` not
  raw triples.
- `PulseClock`: 24 clocks → `ppq` += 1 exactly at pulse instants; tempo
  ~120; interpolation between pulses is in `[0, 1/24)`; overdue clock holds
  (one second of silence does not advance `ppq`); Start zeros; SPP of 8
  sixteenths → `ppq = 2`; Stop then Continue resumes the same `ppq`; clocks
  while stopped update `bpm` only; duplicate timestamps do not increment
  index; no skip-count across a 3-pulse gap (hold, then next clock is
  `+1`, not `+4`).

**`sway-midi`**

- Feed/drain: two stamped messages keep order; `host_time == 0` maps to now;
  a far-future stamp stays in the inbox.
- `MidiTime` writes `ppq` before the graph tick.
- Plugin inserts `Res<Transport>` and does not insert `Time<Transport>`.

**`sway-nodes`**

- `Oscillator` at `time = 0`, `phase = 0.25`, sine → `1.0` with no MIDI
  plugin in the app.
- Amplitude wire still `set_if_neq`.
- One-tick `TimeFrom` chain is an integration test that adds `WiresPlugin` +
  `MidiPlugin` (or a stub `FloatOut` source). The oscillator itself never
  instantiates MIDI.

**Editor / traces**

- Transport bar snapshot reads the new resource.
- Golden traces that assumed freewheel-on-dropout are rewritten to hold.
- Traces that assumed `Lfo` self-clocking from `Time<Transport>` wire
  `MidiTime` or pass an authored `time`.

## Migration (code)

| Remove | Replacement |
|---|---|
| crate `sway-midi` (old name) | `sway-midi-core` |
| `MidiEvent`, `RawMidi` | `MidiMessage` |
| `ClockEstimator::beats_at` | `PulseClock` getters |
| `MidiClockOffset`, `feed_midi` | plugin feed/drain |
| `sway-nodes::MidiPlugin` | `sway-midi::MidiPlugin` |
| `Time<Transport>`, `TransportState`, `TransportTime` in `sway-graph` | `sway-midi::Transport` |
| `FloatOut` / `Vec3Out` in `sway-nodes` | `sway-graph` |
| `Lfo` | `Oscillator` |
| freewheel path in `advance_transport` | hold |

`architecture.md` §4 transport ownership, §5 ownership table, §7 schedule,
§8 crate list: MIDI IO + playhead live in `sway-midi-core` / `sway-midi`;
the graph stays MIDI-free. M9's "move MIDI nodes into `sway-midi`" is this
crate split plus `MidiTime`; `MidiNote` / `BeatTrigger` / `Envelope` remain
M9.

CoreMIDI virtual destination, unique ID, advance-schedule, and `--midi`
filter are unchanged.

## Out of scope

- `MidiNote`, `MidiCC`, `BeatTrigger` authorable nodes, `sway-events` (M9)
- Windows / Linux MIDI backends
- Device hotplug
- Ableton Link
- A seconds/wall-clock time node (the inlet exists; the node can wait)
- Changing the 120 Hz graph tick

## Open, deliberately closed here

- Interpolation clamp is `min(frac, 1 - f64::EPSILON)`, not “snap back to the
  last integer” (that would hitch backward every overdue pulse).
- `beats_per_bar` stays authored on `Transport`, not on `MidiTime`. One
  playhead, one bar length, editor and `MusicalTime` agree.
- `PulseClock` is not a PLL. The grid is the pulse count; the smoother is
  tempo only. If hitching between pulses is visible later, add a PLL inside
  `PulseClock` without changing the snapshot API.
