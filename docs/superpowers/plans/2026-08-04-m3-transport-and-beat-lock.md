# M3 — Transport and Beat Lock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ingest MIDI clock at 24 ppqn, turn it into a drift-corrected beat position exposed as `Time<Transport>`, and make three nodes and the editor read it — so visuals stay beat-locked through tempo changes and clock dropouts.

**Architecture:** Four layers, each testable alone. (1) `sway-midi` gains a real byte-stream parser, because the current fixed three-byte stride silently drops every one-byte System Real-Time message — including clock. (2) `sway-midi` gains a *pure* phase estimator: a windowed least-squares fit over `(pulse index, arrival time)` pairs, which yields period and phase with no tuning constants and is drift-corrected by construction. (3) `sway-graph` gains `Time<Transport>`, a Bevy clock whose elapsed time is measured in **beats** and which knows nothing about MIDI. (4) `sway-nodes` gains the one system that joins them — `advance_transport`, in `FixedUpdate` between `drain_inbox` and `graph_tick` — plus three transport-aware nodes. The editor reads the clock as a fourth consumer of M2c's snapshot.

**Tech Stack:** Rust 2024, bevy 0.19 subcrates (`bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform`), CoreMIDI via hand-written FFI, masonry (editor).

**Parent spec:** `docs/superpowers/specs/2026-07-25-sway-design.md` §2.6, §2.7, §2.9, §2.11, §5 (M3). There is no separate M3 design document; this plan carries the design, and Task 12 records what it got wrong.

## Global Constraints

- `sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform` only. **Not** the `bevy` facade, **not** `bevy_render`, **not** `sway-midi`. The manifest is the only place this is enforced. `Time<Transport>` therefore contains no MIDI vocabulary at all — no pulses, no status bytes.
- `sway-editor` may depend on `sway-graph`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform`. **Not** `bevy`, `bevy_render`, `wgpu`, `vello`, `imaging_vello`, **not** `sway-nodes`.
- `sway-midi`'s parser and estimator must stay free of every Bevy dependency. The crate's current dependency list is `crossbeam-channel` and nothing else; the estimator adds nothing to it.
- The tick is infallible. Nothing added here may panic inside `graph_tick`, `drain_inbox` or `advance_transport` for any input, including a NaN period, a zero timestep, or a 20-minute freeze.
- **Beats never run backwards.** `Time::advance_by` panics on a negative `Duration` and `advance_to` panics when asked to move backwards. Every phase correction is applied as a non-negative delta; a correction that would rewind stalls for one tick instead.
- Nodes derive time-varying values from absolute time, never by accumulating per tick (parent §2.2). This holds for beat time exactly as it does for wall time: `SyncLfo` reads a beat position, it does not integrate one.
- Use `reflect_clone()`, never `to_dynamic()`, for any arena value that must later downcast to its concrete type.
- Enum defaults are the first variant (asserted by `sway-nodes`' existing `enum_defaults_are_the_first_variants` test — extend it, do not work around it).
- Tick rate stays 120 Hz (`sway-app`'s `TICK_HZ`). Choosing it is still open (parent §7) and is not this milestone's job.
- Clippy gate for this work: `cargo clippy -p sway-midi -p sway-graph -p sway-nodes -p sway-editor -p sway-app --all-targets -- -D warnings`. `cargo clippy --workspace` was already red on `main` before this milestone; do not attribute pre-existing debt here.
- Any timing measurement runs with `--test-threads=1` and times the system directly, never `App::update()` (parent §7).

## Expected build state

Every task is additive. `cargo test --workspace` must pass at the end of each one. There is no flip window in this milestone.

## Design decisions this plan makes

These are decisions, recorded here because no design document holds them. Task 12 revisits each.

1. **Windowed least-squares regression, not a PLL.** The estimator fits a line to the last 48 `(pulse index, arrival time)` pairs — two beats. Slope is seconds-per-pulse; the fit's inverse is beat position. No gain constants, exact on a clean train, and the settling behaviour after a tempo change is a property of the window length rather than of two numbers tuned by ear. The cost is about one window of lag on an abrupt tempo jump, which for a hardware sequencer changing tempo is inaudible and invisible.
2. **Dropouts freewheel indefinitely.** When pulses stop, beat time keeps advancing at the last estimated period; when they return, phase re-locks by tracking the fit again. A cable glitch is invisible and the screen never freezes. Unbounded drift while the clock is away is the accepted cost, and it is bounded in practice by the clock coming back.
3. **Missed pulses are inferred, not counted.** The estimator indexes pulses by a running counter, so a dropped pulse would otherwise shear the fit. On each pulse it infers how many pulse periods elapsed since the last one and advances the index by that much. A gap longer than one beat abandons the fit and re-locks from scratch, bumping a generation counter so the transport knows not to difference across the seam.
4. **`Time<Transport>` lives in `sway-graph`, the estimator in `sway-midi`, the wiring in `sway-nodes`.** Parent §3 puts the estimator in `sway-midi`; parent §2.9 puts beat time under `bevy_time`. The clock type carries no MIDI, so the editor — which may not depend on `sway-nodes` — can read it.
5. **Position is `elapsed − origin`, not a resettable clock.** `Time<T>` is monotone by construction. `Transport::origin_beats` records where the last Start (or Song Position Pointer) put the origin, so a reposition moves the origin rather than rewinding the clock.
6. **`beats_per_bar` belongs to the transport, not to a node.** MIDI clock carries no time signature, so bars are authored. One global value, because the editor readout and every node must agree on where a bar starts.
7. **Clock pulses arriving while stopped set the tempo but do not advance position.** Devices that free-run their clock are common; treating a pulse as an implicit Start would make a stopped DAW scroll the visuals.
8. **The MIDI epoch bridge becomes a min-filtered offset tracker.** `Time<Fixed>` falls behind real time by up to one timestep normally and by an unbounded amount whenever `max_delta` drops ticks, so an epoch sampled once at first drain drifts monotonically. Sampling `host_now − fixed_elapsed` every drain and taking the *minimum* over a sliding window recovers the true offset from a signal whose noise is one-sided — the same argument NTP's min filter rests on.

## File structure

**`crates/sway-midi/src/`** — no Bevy, no `App`, no `World`.

| File | Responsibility |
|---|---|
| `parser.rs` | **new** — `StreamParser`: a real MIDI byte-stream parser (running status, one-byte System Real-Time, two-byte channel messages, SysEx skip, System Common), plus the status-byte constants |
| `transport.rs` | **new** — `ClockEstimator`: the pure windowed-regression phase estimator. Pulses in, period and beat position out |
| `input.rs` | `read_proc` walks packets and feeds every byte through `StreamParser` instead of striding three bytes at a time |
| `lib.rs` | re-exports |

**`crates/sway-graph/src/`**

| File | Responsibility |
|---|---|
| `transport.rs` | **new** — `Transport` (the clock context), `TransportState`, `MusicalTime`, the `TransportTime` extension trait on `Time<Transport>`. No MIDI |
| `tick.rs` | `GraphPlugin` inserts `Time<Transport>` and registers its reflected types |

**`crates/sway-nodes/src/`**

| File | Responsibility |
|---|---|
| `transport.rs` | **new** — `TransportClock` (the estimator's home in the world) and `advance_transport`, the one system joining MIDI bytes to `Time<Transport>` |
| `beat.rs` | **new** — the three transport-aware node types: `TransportTime`, `SyncLfo`, `BeatTrigger`, plus `Division` and the `Beat` event payload |
| `lfo.rs` | the waveform evaluation is extracted to `pub(crate) fn wave(...)` so `SyncLfo` shares it rather than copying it |
| `midi.rs` | `SignalNodesPlugin` registers the three new node types, the resource, and the system's schedule position |

**`crates/sway-editor/src/`**

| File | Responsibility |
|---|---|
| `transport_bar.rs` | **new** — the `TransportBar` widget: state, BPM, bar.beat.16th |
| `snapshot.rs` | `WorldSnapshot` gains `transport: TransportView`, read from `Time<Transport>` |
| `lib.rs` | `graph_root` puts the bar above the three panes; `apply_snapshot` feeds it |

**`crates/sway-app/src/`**

| File | Responsibility |
|---|---|
| `midi_feed.rs` | `MidiTimeEpoch` becomes `MidiClockOffset`: a min-filtered, per-drain offset with a monotone enqueue clamp |
| `demo_graph.rs` | the demo graph becomes beat-locked |

**`crates/sway-nodes/tests/`** — `traces.rs` grows a clock-train generator and four transport trace cases.

---

### Task 1: A real MIDI byte-stream parser

`read_proc` currently walks each packet in fixed three-byte strides, guarded by `while i + 2 < len`. Every System Real-Time message is one byte, so a packet carrying a clock pulse (`0xF8`, length 1) never enters that loop at all: **MIDI clock is currently discarded before it reaches the app.** Two-byte messages and running status are misparsed the same way. Nothing downstream in this milestone can be built until bytes are parsed properly.

The parser is per-packet, not per-connection. CoreMIDI's `MIDIPacket` carries complete messages (SysEx is the documented exception, and is skipped), so per-packet state is correct — and it avoids two sources sharing one running-status register, which a connection-scoped parser would get wrong for the virtual destination and a hardware port feeding the same callback.

**Files:**
- Create: `crates/sway-midi/src/parser.rs`
- Modify: `crates/sway-midi/src/input.rs:39-86` (`read_proc`)
- Modify: `crates/sway-midi/src/lib.rs:1-7`
- Modify: `crates/sway-app/src/midi_feed.rs:44-47` (delete the per-message `eprintln!`)

**Interfaces:**
- Produces: `StreamParser::new()`, `StreamParser::push(&mut self, byte: u8) -> Option<(u8, u8, u8)>`, and the constants `CLOCK`, `START`, `CONTINUE`, `STOP`, `SONG_POSITION`. Task 5 matches on those constants; Task 2 receives the pulses this produces.
- Consumes: nothing.

- [ ] **Step 1: Write the failing tests**

Create `crates/sway-midi/src/parser.rs` with only the test module and an empty `impl`, so the tests compile against names that do not work yet. Write the whole file's tests now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds a byte slice and collects every completed message.
    fn parse(bytes: &[u8]) -> Vec<(u8, u8, u8)> {
        let mut parser = StreamParser::new();
        bytes.iter().filter_map(|&b| parser.push(b)).collect()
    }

    #[test]
    fn a_lone_clock_byte_is_a_message() {
        // THE M0 BUG. A packet holding one 0xF8 never entered the old
        // three-byte stride, so MIDI clock reached the app never.
        assert_eq!(parse(&[CLOCK]), vec![(CLOCK, 0, 0)]);
    }

    #[test]
    fn real_time_bytes_interrupt_a_message_without_corrupting_it() {
        // System Real-Time may appear between any two bytes of another
        // message and must not disturb it — this is why a stride parser
        // cannot be patched to handle clock.
        assert_eq!(
            parse(&[0x90, 60, CLOCK, 100]),
            vec![(CLOCK, 0, 0), (0x90, 60, 100)]
        );
    }

    #[test]
    fn running_status_repeats_the_previous_status() {
        assert_eq!(
            parse(&[0x90, 60, 100, 62, 90]),
            vec![(0x90, 60, 100), (0x90, 62, 90)]
        );
    }

    #[test]
    fn two_byte_messages_complete_after_one_data_byte() {
        // Program Change and Channel Pressure carry one data byte. The old
        // stride would have eaten the following status byte as data.
        assert_eq!(parse(&[0xC0, 5, 0xD0, 64]), vec![(0xC0, 5, 0), (0xD0, 64, 0)]);
    }

    #[test]
    fn a_song_position_pointer_carries_both_data_bytes() {
        // 14-bit, LSB first: 8 sixteenths = two beats.
        assert_eq!(parse(&[SONG_POSITION, 8, 0]), vec![(SONG_POSITION, 8, 0)]);
    }

    #[test]
    fn system_common_clears_running_status() {
        // After a Song Select, a bare data byte is not a note-on.
        assert_eq!(parse(&[0x90, 60, 100, 0xF3, 2, 62, 90]), vec![
            (0x90, 60, 100),
            (0xF3, 2, 0),
        ]);
    }

    #[test]
    fn sysex_is_skipped_and_does_not_swallow_what_follows() {
        assert_eq!(
            parse(&[0xF0, 1, 2, 3, 0xF7, 0x90, 60, 100]),
            vec![(0x90, 60, 100)]
        );
    }

    #[test]
    fn real_time_bytes_pass_through_a_sysex_block() {
        // A clock inside a SysEx dump still has to reach the transport.
        assert_eq!(
            parse(&[0xF0, 1, CLOCK, 2, 0xF7]),
            vec![(CLOCK, 0, 0)]
        );
    }

    #[test]
    fn transport_commands_are_one_byte_messages() {
        assert_eq!(
            parse(&[START, STOP, CONTINUE]),
            vec![(START, 0, 0), (STOP, 0, 0), (CONTINUE, 0, 0)]
        );
    }

    #[test]
    fn a_stray_data_byte_before_any_status_is_dropped() {
        assert_eq!(parse(&[60, 100]), Vec::new());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-midi parser`
Expected: compile error — `StreamParser` and the constants do not exist.

- [ ] **Step 3: Write the parser**

Put this above the test module in `crates/sway-midi/src/parser.rs`:

```rust
//! A MIDI byte-stream parser.
//!
//! M0 walked each `MIDIPacket` in fixed three-byte strides. That is wrong for
//! three separate reasons and each one costs this milestone: System Real-Time
//! messages are one byte and may appear *between the bytes of another
//! message*, Program Change and Channel Pressure carry one data byte, and
//! running status omits repeated status bytes entirely. Clock, start, stop and
//! continue are all System Real-Time, so under the old stride the transport
//! received nothing at all.
//!
//! State is per packet, not per connection: CoreMIDI packets hold complete
//! messages (SysEx excepted, and skipped here), and one parser shared across
//! the input port and the virtual destination would let two sources corrupt
//! each other's running status.

/// MIDI clock, 24 per quarter note.
pub const CLOCK: u8 = 0xF8;
/// Start playback from the beginning.
pub const START: u8 = 0xFA;
/// Resume playback from the current position.
pub const CONTINUE: u8 = 0xFB;
/// Stop playback.
pub const STOP: u8 = 0xFC;
/// Song Position Pointer: 14-bit count of sixteenth notes, LSB first.
pub const SONG_POSITION: u8 = 0xF2;

/// How many data bytes a status byte expects.
fn data_len(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        0xC0 | 0xD0 => 1,
        0xF0 => match status {
            0xF1 | 0xF3 => 1,
            SONG_POSITION => 2,
            _ => 0,
        },
        _ => 0,
    }
}

/// Parses a MIDI byte stream one byte at a time.
#[derive(Debug, Default)]
pub struct StreamParser {
    /// The status whose data bytes are being collected.
    current: Option<u8>,
    /// The last channel status, reused when a data byte arrives with no
    /// status of its own. System Common clears it; System Real-Time does not.
    running: Option<u8>,
    data: [u8; 2],
    have: usize,
    in_sysex: bool,
}

impl StreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one byte. Returns `Some((status, data1, data2))` when a message
    /// completes; `data2` is 0 for messages carrying one data byte.
    pub fn push(&mut self, byte: u8) -> Option<(u8, u8, u8)> {
        // System Real-Time: one byte, may appear anywhere, changes nothing.
        if byte >= 0xF8 {
            return Some((byte, 0, 0));
        }

        if byte >= 0x80 {
            self.have = 0;
            match byte {
                0xF0 => {
                    self.in_sysex = true;
                    self.current = None;
                    self.running = None;
                    return None;
                }
                0xF7 => {
                    self.in_sysex = false;
                    self.current = None;
                    return None;
                }
                _ => {}
            }
            self.in_sysex = false;
            // System Common clears running status; a channel status sets it.
            self.running = (byte < 0xF0).then_some(byte);
            if data_len(byte) == 0 {
                self.current = None;
                return Some((byte, 0, 0));
            }
            self.current = Some(byte);
            return None;
        }

        if self.in_sysex {
            return None;
        }

        let status = self.current.or(self.running)?;
        self.current = Some(status);
        self.data[self.have] = byte;
        self.have += 1;
        if self.have < data_len(status) {
            return None;
        }

        let message = (status, self.data[0], if data_len(status) == 2 { self.data[1] } else { 0 });
        self.have = 0;
        // A channel status stays current (running status); System Common
        // does not repeat.
        if status >= 0xF0 {
            self.current = None;
        }
        Some(message)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-midi parser`
Expected: PASS, 10 tests.

- [ ] **Step 5: Route `read_proc` through the parser**

In `crates/sway-midi/src/lib.rs`, add `pub mod parser;` next to `pub mod input;` and extend the re-export line:

```rust
pub use parser::{CLOCK, CONTINUE, SONG_POSITION, START, STOP, StreamParser};
```

In `crates/sway-midi/src/input.rs`, add `use crate::parser::StreamParser;` and replace the byte loop inside `read_proc` (the block from the `// NOTE: this assumes every message in the stream is exactly three bytes` comment through `i += 3; }`) with:

```rust
            // One parser per packet: CoreMIDI packets hold complete messages,
            // and connection-scoped state would let the hardware port and the
            // virtual destination corrupt each other's running status.
            let mut parser = StreamParser::new();
            for &byte in data {
                if let Some((status, data1, data2)) = parser.push(byte) {
                    // NOTE: `send` on an unbounded channel can allocate (to
                    // grow the internal buffer) and this runs on CoreMIDI's
                    // high-priority real-time thread (see module doc).
                    // Acceptable through M3; revisit if this ever glitches.
                    let _ = tx.send(MidiEvent { status, data1, data2, host_time });
                }
            }
```

Delete the now-stale `let mut i = 0;` and the `while` loop it guarded, and drop `.min(256)`'s companion comment only if it no longer applies (`len` is still clamped — keep that).

- [ ] **Step 6: Stop the ingress log from flooding**

At 24 ppqn and 120 BPM the parser now delivers 48 clock messages per second, and `crates/sway-app/src/midi_feed.rs:44-47` prints one line per message. Delete that `eprintln!` outright — it was M0 debugging output and this task turns it into a flood.

- [ ] **Step 7: Extend the packet-walk test to cover a real-time byte**

The existing `read_proc_parses_multiple_packets` test in `crates/sway-midi/src/lib.rs` pins the alignment-sensitive packet walk. Add a sibling that proves clock survives it — this is the integration half of Step 1's unit tests:

```rust
    /// A one-byte clock packet. The alignment-sensitive `next_packet` walk
    /// and the parser have to agree about a packet whose length is 1, which
    /// is the shape every MIDI clock pulse arrives in.
    #[test]
    fn read_proc_delivers_a_one_byte_clock_packet() {
        let (tx, rx) = crossbeam_channel::unbounded::<MidiEvent>();
        let tx = Box::new(tx);
        let mut buf_u32 = vec![0u32; 1024];
        let buf = unsafe {
            std::slice::from_raw_parts_mut(buf_u32.as_mut_ptr() as *mut u8, buf_u32.len() * 4)
        };
        // SAFETY: same construction as `read_proc_parses_multiple_packets`.
        unsafe {
            let list = buf.as_mut_ptr() as *mut MIDIPacketList;
            (*list).num_packets = 2;

            let p1 = (&raw mut (*list).packet) as *mut MIDIPacket;
            (*p1).time_stamp = 10;
            (*p1).length = 1;
            (*p1).data[0] = crate::parser::CLOCK;

            let p2 = crate::input::next_packet(p1) as *mut MIDIPacket;
            (*p2).time_stamp = 20;
            (*p2).length = 3;
            (*p2).data[0] = 0x90;
            (*p2).data[1] = 60;
            (*p2).data[2] = 100;

            crate::input::read_proc(
                list,
                (&*tx) as *const crossbeam_channel::Sender<MidiEvent> as *mut c_void,
                std::ptr::null_mut(),
            );
        }

        let clock = rx.try_recv().expect("the clock packet must arrive");
        assert_eq!((clock.status, clock.host_time), (crate::parser::CLOCK, 10));
        let note = rx.try_recv().expect("the note packet must arrive");
        assert_eq!((note.status, note.data1, note.data2), (0x90, 60, 100));
        assert!(rx.try_recv().is_err());
    }
```

- [ ] **Step 8: Run the crate's whole suite**

Run: `cargo test -p sway-midi`
Expected: PASS. Then `cargo test --workspace` — PASS; nothing else consumed the old parse behaviour.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-midi crates/sway-app/src/midi_feed.rs
git commit -m "feat(midi): parse the MIDI byte stream, so clock reaches the app"
```

---

### Task 2: The phase estimator

A pure, Bevy-free windowed least-squares fit over `(pulse index, arrival time)`. This is the piece parent §2.9 assigns to us, and the only part of M3 that is genuinely an algorithm rather than wiring, so it is built and tested entirely on its own before anything reads it.

Two properties do the work. The **fit** turns 24 jittery pulses per quarter note into a slope (seconds per pulse) and an intercept (phase), which together map any absolute time to a beat position — drift-corrected by construction, because every new pulse re-fits against the clock the app actually runs on. The **index inference** keeps a dropped pulse from shearing that fit: pulses are indexed by a counter, so a missing pulse would otherwise compress two beats' worth of index into one beat's worth of time.

**Files:**
- Create: `crates/sway-midi/src/transport.rs`
- Modify: `crates/sway-midi/src/lib.rs`

**Interfaces:**
- Produces: `ClockEstimator::{new, reset, push_pulse, secs_per_pulse, secs_per_beat, bpm, beats_at, is_locked, generation}`, `PULSES_PER_QUARTER`. Task 5 owns the only instance of it.
- Consumes: nothing.

- [ ] **Step 1: Write the failing tests**

Create `crates/sway-midi/src/transport.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SPP_120: f64 = 0.5 / PULSES_PER_QUARTER as f64;

    /// A deterministic pseudo-random jitter source. No `rand` dependency:
    /// a golden-trace project cannot afford a non-reproducible test.
    struct Lcg(u64);
    impl Lcg {
        fn next_signed(&mut self, magnitude: f64) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let unit = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
            (unit * 2.0 - 1.0) * magnitude
        }
    }

    fn steady(estimator: &mut ClockEstimator, pulses: usize, spp: f64, start: f64) -> f64 {
        let mut t = start;
        for _ in 0..pulses {
            estimator.push_pulse(t);
            t += spp;
        }
        t
    }

    #[test]
    fn a_clean_train_recovers_its_tempo_exactly() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 48, SPP_120, 0.0);
        assert!((estimator.bpm().expect("locked") - 120.0).abs() < 1e-9);
    }

    #[test]
    fn fewer_than_the_minimum_pulses_is_not_a_lock() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 3, SPP_120, 0.0);
        assert!(!estimator.is_locked());
        assert_eq!(estimator.bpm(), None);
    }

    #[test]
    fn jitter_averages_out_across_the_window() {
        let mut estimator = ClockEstimator::new();
        let mut lcg = Lcg(0x5EED);
        let mut t = 0.0;
        for _ in 0..96 {
            // ±1ms of jitter, which is worse than USB MIDI in practice.
            estimator.push_pulse(t + lcg.next_signed(0.001));
            t += SPP_120;
        }
        let bpm = estimator.bpm().expect("locked");
        assert!((bpm - 120.0).abs() < 1.0, "jittered fit gave {bpm} BPM");
    }

    #[test]
    fn beat_position_advances_one_beat_per_twenty_four_pulses() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 48, SPP_120, 0.0);
        let a = estimator.beats_at(0.0).expect("locked");
        let b = estimator.beats_at(0.5).expect("locked");
        assert!((b - a - 1.0).abs() < 1e-9, "half a second is one beat at 120 BPM");
    }

    #[test]
    fn a_tempo_change_settles_within_one_window() {
        let mut estimator = ClockEstimator::new();
        let end = steady(&mut estimator, 48, SPP_120, 0.0);
        let spp_140 = (60.0 / 140.0) / PULSES_PER_QUARTER as f64;
        steady(&mut estimator, WINDOW_PULSES, spp_140, end);
        let bpm = estimator.bpm().expect("locked");
        assert!((bpm - 140.0).abs() < 1.0, "after one full window: {bpm} BPM");
    }

    #[test]
    fn a_few_dropped_pulses_do_not_shear_the_fit() {
        // The index is inferred from elapsed time, not counted. Without that,
        // three missing pulses would read as the tempo jumping.
        let mut estimator = ClockEstimator::new();
        let mut t = steady(&mut estimator, 48, SPP_120, 0.0);
        t += 3.0 * SPP_120; // three pulses lost in transit
        steady(&mut estimator, 24, SPP_120, t);
        let bpm = estimator.bpm().expect("locked");
        assert!((bpm - 120.0).abs() < 0.5, "dropped pulses moved the tempo to {bpm}");
    }

    #[test]
    fn a_gap_longer_than_a_beat_restarts_the_fit() {
        let mut estimator = ClockEstimator::new();
        let t = steady(&mut estimator, 48, SPP_120, 0.0);
        let before = estimator.generation();
        estimator.push_pulse(t + 3.0); // three seconds of silence
        assert_ne!(estimator.generation(), before, "a long gap must restart the fit");
        assert!(!estimator.is_locked(), "one pulse is not a lock");
    }

    #[test]
    fn reset_bumps_the_generation_and_drops_the_lock() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 48, SPP_120, 0.0);
        let before = estimator.generation();
        estimator.reset();
        assert_ne!(estimator.generation(), before);
        assert!(!estimator.is_locked());
        assert_eq!(estimator.beats_at(1.0), None);
    }

    #[test]
    fn two_pulses_at_the_same_instant_do_not_produce_a_nan() {
        // A duplicated timestamp gives a degenerate fit. The estimator must
        // refuse it rather than hand a NaN period to the tick.
        let mut estimator = ClockEstimator::new();
        for _ in 0..8 {
            estimator.push_pulse(1.0);
        }
        assert!(estimator.bpm().is_none_or(|bpm| bpm.is_finite()));
        assert!(estimator.beats_at(1.0).is_none_or(f64::is_finite));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-midi transport`
Expected: compile error — `ClockEstimator` does not exist.

- [ ] **Step 3: Write the estimator**

Above the test module in `crates/sway-midi/src/transport.rs`:

```rust
//! The 24 ppqn phase estimator: pulses in, tempo and beat position out.
//!
//! Pure. No Bevy, no world, no clock of its own — every time it is handed is
//! absolute seconds on whatever timeline the caller uses, and every answer is
//! in those same seconds. `sway-nodes` owns the one instance and hands it the
//! graph's fixed-clock timeline.
//!
//! Raw pulse timing is too jittery to use directly (parent §2.7), so this
//! fits a line to the last [`WINDOW_PULSES`] `(pulse index, arrival time)`
//! pairs by least squares. The slope is seconds per pulse; the intercept is
//! phase; inverting the line maps any time to a beat position. There are no
//! gains to tune, and drift is corrected by construction because each new
//! pulse re-fits against the caller's own timeline.

use std::collections::VecDeque;

/// MIDI clock resolution: 24 pulses per quarter note.
pub const PULSES_PER_QUARTER: u32 = 24;

/// How many pulses the fit spans — two beats. Long enough to average out
/// jitter, short enough that a tempo change settles in under a second.
pub const WINDOW_PULSES: usize = 48;

/// Below this many samples there is no lock: a fit over three noisy points
/// is worse than freewheeling at the last known tempo.
const MIN_SAMPLES: usize = 8;

/// A gap longer than this many pulse periods abandons the fit. One beat of
/// silence means the clock stopped, not that pulses were dropped, and
/// inferring 24+ missing indices from a stale period is guesswork.
const MAX_GAP_PULSES: f64 = PULSES_PER_QUARTER as f64;

/// Fits tempo and phase to a 24 ppqn pulse train.
#[derive(Debug, Default)]
pub struct ClockEstimator {
    /// `(pulse index, arrival time)`, oldest first, at most `WINDOW_PULSES`.
    samples: VecDeque<(f64, f64)>,
    /// Index to assign the next pulse, before gap inference.
    next_index: f64,
    last_time: Option<f64>,
    /// Current fit: `time = slope * index + intercept`. `None` until the fit
    /// is both long enough and non-degenerate.
    fit: Option<(f64, f64)>,
    generation: u64,
}

impl ClockEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Abandons the current fit. The generation changes, which is how a
    /// caller differencing beat positions across ticks knows not to difference
    /// across the seam.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.next_index = 0.0;
        self.last_time = None;
        self.fit = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Which fit the current answers belong to. Changes on every `reset`,
    /// including the implicit one a long gap causes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Records one clock pulse, arriving at absolute time `t`.
    pub fn push_pulse(&mut self, t: f64) {
        if !t.is_finite() {
            return;
        }

        // Infer how many pulse periods elapsed rather than counting pulses,
        // so a dropped pulse does not shear the fit.
        let index = match (self.last_time, self.fit) {
            (Some(last), Some((slope, _))) if slope > 0.0 => {
                let elapsed = ((t - last) / slope).round().max(1.0);
                if elapsed > MAX_GAP_PULSES {
                    self.reset();
                    0.0
                } else {
                    self.next_index + elapsed - 1.0
                }
            }
            _ => self.next_index,
        };

        self.samples.push_back((index, t));
        if self.samples.len() > WINDOW_PULSES {
            self.samples.pop_front();
        }
        self.next_index = index + 1.0;
        self.last_time = Some(t);
        self.refit();
    }

    fn refit(&mut self) {
        if self.samples.len() < MIN_SAMPLES {
            self.fit = None;
            return;
        }
        let n = self.samples.len() as f64;
        let mean_x = self.samples.iter().map(|&(x, _)| x).sum::<f64>() / n;
        let mean_y = self.samples.iter().map(|&(_, y)| y).sum::<f64>() / n;
        let mut covariance = 0.0;
        let mut variance = 0.0;
        for &(x, y) in &self.samples {
            covariance += (x - mean_x) * (y - mean_y);
            variance += (x - mean_x) * (x - mean_x);
        }
        // A degenerate fit (every pulse at one instant, or a non-positive
        // period) is refused rather than propagated: the tick is infallible
        // and a NaN period would reach `Duration::from_secs_f64`.
        let slope = covariance / variance;
        self.fit = (variance > 0.0 && slope.is_finite() && slope > 0.0)
            .then(|| (slope, mean_y - slope * mean_x));
    }

    /// Whether there is a usable fit.
    pub fn is_locked(&self) -> bool {
        self.fit.is_some()
    }

    /// Estimated pulse period, in seconds.
    pub fn secs_per_pulse(&self) -> Option<f64> {
        self.fit.map(|(slope, _)| slope)
    }

    /// Estimated beat period, in seconds.
    pub fn secs_per_beat(&self) -> Option<f64> {
        self.secs_per_pulse().map(|spp| spp * PULSES_PER_QUARTER as f64)
    }

    pub fn bpm(&self) -> Option<f64> {
        self.secs_per_beat().map(|spb| 60.0 / spb)
    }

    /// Beat position at absolute time `t`, relative to this fit's own origin.
    ///
    /// Only differences between two calls within one [`generation`] are
    /// meaningful — the origin moves whenever the fit restarts.
    ///
    /// [`generation`]: Self::generation
    pub fn beats_at(&self, t: f64) -> Option<f64> {
        let (slope, intercept) = self.fit?;
        Some((t - intercept) / slope / PULSES_PER_QUARTER as f64)
    }
}
```

In `crates/sway-midi/src/lib.rs` add `pub mod transport;` and extend the re-exports:

```rust
pub use transport::{ClockEstimator, PULSES_PER_QUARTER};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-midi transport`
Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-midi
git commit -m "feat(midi): windowed least-squares phase estimator for 24 ppqn clock"
```

---

### Task 3: One clock discipline for MIDI ingress

M2a's epoch bridge samples `host_time_to_secs(host_time_now()) - elapsed` once, at first drain, and never corrects it. Parent §5 names this M3's problem and says replace rather than patch, because it is the same clock-alignment question the phase estimator answers.

The drift is not hypothetical and not symmetric. `Time<Fixed>::elapsed` lags real time by up to one timestep in normal operation, and by an unbounded amount whenever `max_delta` drops ticks under load (parent §2.6). A fixed epoch therefore maps MIDI timestamps ever further into the future, where they wait in the inbox until the existing half-second guard fires and collapses them all to "now" — losing exactly the sub-tick precision the whole event model exists to preserve.

The sample's noise is one-sided: `host_now − fixed_elapsed` is never *below* the true offset and exceeds it by however far into the current timestep the drain happens to land. The minimum over a sliding window is therefore the estimate, not the mean.

**Files:**
- Modify: `crates/sway-app/src/midi_feed.rs` (whole file)
- Modify: `crates/sway-app/src/main.rs:12,211` (the resource's name)

**Interfaces:**
- Produces: `MidiClockOffset` (replacing `MidiTimeEpoch`), and the pure helpers `MidiClockOffset::observe(&mut self, sample: f64) -> f64` and `map_timestamp(host_secs, offset, elapsed, last_enqueued) -> f64`.
- Consumes: nothing from Tasks 1–2 beyond the extra message volume Task 1 now delivers.

- [ ] **Step 1: Write the failing tests**

Replace the test module in `crates/sway-app/src/midi_feed.rs` with this, keeping `zero_host_time_maps_to_current_fixed_elapsed` and `feed_midi_drains_every_event_into_the_inbox` from the existing module unchanged except for the resource rename:

```rust
    #[test]
    fn the_offset_is_the_minimum_of_the_window_not_the_mean() {
        // `host_now - fixed_elapsed` is never below the true offset and
        // overshoots by however far into the timestep the drain landed. The
        // minimum recovers the truth from one-sided noise; the mean does not.
        let mut clock = MidiClockOffset::default();
        for sample in [1.0, 1.008, 1.003, 1.006, 1.001] {
            clock.observe(sample);
        }
        assert!((clock.observe(1.004) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_offset_follows_a_drifting_fixed_clock() {
        // The M2a bug: with a fixed epoch, an event arriving "now" maps
        // further and further into the future as Time<Fixed> falls behind.
        // Only samples inside the window may contribute.
        let mut clock = MidiClockOffset::default();
        for step in 0..(OFFSET_WINDOW * 2) {
            clock.observe(1.0 + step as f64 * 0.001);
        }
        let offset = clock.observe(1.0 + (OFFSET_WINDOW * 2) as f64 * 0.001);
        let stale = 1.0;
        assert!(
            offset > stale + 0.5,
            "a stale sample from before the window must not pin the offset: {offset}"
        );
    }

    #[test]
    fn a_now_event_maps_to_now_however_far_the_fixed_clock_has_drifted() {
        let mut clock = MidiClockOffset::default();
        // 10 seconds of drift accumulated at 1ms per drain.
        for step in 0..OFFSET_WINDOW {
            clock.observe(5.0 + step as f64 * 0.001);
        }
        let offset = clock.observe(5.0 + OFFSET_WINDOW as f64 * 0.001);
        let elapsed = 100.0;
        let host_secs = elapsed + offset; // an event arriving exactly now
        let t = map_timestamp(host_secs, offset, elapsed, f64::NEG_INFINITY);
        assert!((t - elapsed).abs() < 1e-9, "a now-event mapped to {t}, expected {elapsed}");
    }

    #[test]
    fn a_falling_offset_never_reorders_the_inbox() {
        // The offset moves between drains. Two events must never come out
        // in the opposite order to the one they arrived in.
        let first = map_timestamp(10.0, 1.0, 9.0, f64::NEG_INFINITY);
        let second = map_timestamp(10.001, 1.5, 9.0, first);
        assert!(second >= first, "{second} must not precede {first}");
    }

    #[test]
    fn a_far_future_stamp_is_clamped_rather_than_collapsed_to_now() {
        // DAWs stamp ahead of the playhead. Modest lookahead is honoured;
        // a pathological stamp is clamped to the lookahead bound, which
        // preserves ordering where the old "reset it to now" did not.
        let elapsed = 4.0;
        let t = map_timestamp(1000.0, 0.0, elapsed, f64::NEG_INFINITY);
        assert!((t - (elapsed + MAX_LOOKAHEAD)).abs() < 1e-9, "got {t}");
    }

    #[test]
    fn a_clock_pulse_survives_the_bridge_with_its_status_intact() {
        // Task 1 made clock reachable; nothing between the callback and the
        // inbox may filter it out.
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::MidiEvent {
            status: sway_midi::CLOCK,
            data1: 0,
            data2: 0,
            host_time: 0,
        })
        .unwrap();

        let mut app = App::new();
        app.insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(MidiRx(rx))
            .init_resource::<MidiClockOffset>()
            .init_resource::<MidiInbox>()
            .add_systems(PreUpdate, feed_midi);
        app.update();

        let inbox = app.world().resource::<MidiInbox>();
        assert_eq!(inbox.events.len(), 1);
        assert_eq!(inbox.events[0].1.status, sway_midi::CLOCK);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-app midi_feed`
Expected: compile error — `MidiClockOffset`, `map_timestamp`, `OFFSET_WINDOW`, `MAX_LOOKAHEAD` do not exist.

- [ ] **Step 3: Rewrite the bridge**

Replace everything above the test module in `crates/sway-app/src/midi_feed.rs` with:

```rust
//! MIDI ingress: the CoreMIDI channel into the graph's timestamped inbox.
//!
//! One clock discipline, not two (parent §5, M3). CoreMIDI stamps packets in
//! mach-absolute time; the graph reasons in `Time<Fixed>::elapsed`. M2a
//! sampled the offset between them once and never corrected it, which drifts
//! monotonically: `Time<Fixed>` lags real time by up to one timestep normally
//! and by an unbounded amount whenever `max_delta` drops ticks, so a fixed
//! epoch maps arrivals ever further into the future until the lookahead guard
//! collapses them all to "now" and sub-tick precision is gone.
//!
//! The sample `host_now - fixed_elapsed` is never *below* the true offset —
//! it overshoots by however far into the current timestep the drain landed.
//! One-sided noise means the estimator is the minimum over a sliding window,
//! not the mean.

use bevy::prelude::*;
use crossbeam_channel::Receiver;
use std::collections::VecDeque;
use sway_midi::MidiEvent;
use sway_nodes::{MidiInbox, RawMidi};

/// The receiving end of the CoreMIDI channel.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<MidiEvent>);

/// How many drains the offset estimate spans. At 60 fps this is about four
/// seconds — long enough to see past a timestep of sampling noise, short
/// enough to follow real drift.
pub const OFFSET_WINDOW: usize = 240;

/// How far ahead of the current tick a timestamp may sit. DAWs legitimately
/// stamp ahead of the playhead; anything beyond this is clamped rather than
/// collapsed to now, so ordering survives.
pub const MAX_LOOKAHEAD: f64 = 0.5;

/// Tracks the mach-to-fixed offset, and the last timestamp handed to the
/// inbox so a moving offset can never reorder two arrivals.
#[derive(Resource)]
pub struct MidiClockOffset {
    samples: VecDeque<f64>,
    last_enqueued: f64,
}

impl Default for MidiClockOffset {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(OFFSET_WINDOW),
            last_enqueued: f64::NEG_INFINITY,
        }
    }
}

impl MidiClockOffset {
    /// Records one `host_now - fixed_elapsed` sample and returns the current
    /// offset estimate: the minimum over the window.
    pub fn observe(&mut self, sample: f64) -> f64 {
        if self.samples.len() == OFFSET_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        self.samples
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
    }
}

/// Maps one CoreMIDI host timestamp onto the graph's fixed timeline.
///
/// Pure, so the drift and ordering properties are testable without a mach
/// clock. `last_enqueued` is the floor: the offset moves between drains, and
/// two arrivals must never come out in the opposite order.
pub fn map_timestamp(host_secs: f64, offset: f64, elapsed: f64, last_enqueued: f64) -> f64 {
    let t = host_secs - offset;
    t.clamp(last_enqueued.max(f64::MIN), elapsed + MAX_LOOKAHEAD)
        .max(last_enqueued)
}

/// Moves every CoreMIDI callback event into the graph's timestamped inbox.
pub fn feed_midi(
    rx: Res<MidiRx>,
    time: Res<Time<Fixed>>,
    mut clock: ResMut<MidiClockOffset>,
    mut inbox: ResMut<MidiInbox>,
) {
    let elapsed = time.elapsed_secs_f64();
    let sample = sway_midi::host_time_to_secs(sway_midi::host_time_now()) - elapsed;
    let offset = clock.observe(sample);

    while let Ok(event) = rx.0.try_recv() {
        // A zero stamp means "now" — some senders do not stamp at all.
        let t = if event.host_time == 0 {
            elapsed.max(clock.last_enqueued)
        } else {
            map_timestamp(
                sway_midi::host_time_to_secs(event.host_time),
                offset,
                elapsed,
                clock.last_enqueued,
            )
        };
        clock.last_enqueued = t;
        inbox.push(
            t,
            RawMidi {
                status: event.status,
                data1: event.data1,
                data2: event.data2,
            },
        );
    }
}
```

The `clamp` call above panics if its bounds cross, which `last_enqueued = -inf` cannot cause but a stalled clock could; the trailing `.max(last_enqueued)` is what makes the floor authoritative. Write it exactly as shown.

- [ ] **Step 4: Rename the resource at the call site**

In `crates/sway-app/src/main.rs`, change the import on line 12 to `use midi_feed::{MidiClockOffset, MidiRx, feed_midi};` and line 211's `.init_resource::<MidiTimeEpoch>()` to `.init_resource::<MidiClockOffset>()`. Update the two surviving tests in the module to use `MidiClockOffset` too, and delete `host_time_near_now_maps_to_fixed_elapsed_time`'s pre-seeded epoch in favour of `a_now_event_maps_to_now_however_far_the_fixed_clock_has_drifted`, which tests the same property without racing a real mach sample.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-app`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-app
git commit -m "fix(app): track the mach-to-fixed offset instead of sampling it once"
```

---

### Task 4: `Time<Transport>` — the clock

A Bevy clock whose elapsed time is measured in beats (parent §2.7). It contains no MIDI vocabulary, which is what lets `sway-editor` read it without depending on `sway-nodes`, and what keeps `sway-graph`'s manifest honest.

Two things are worth stating before writing it. `Time<T>` is monotone — `advance_by` takes a `Duration` and `advance_to` panics on a rewind — so a Start cannot reset it. **Position is `elapsed − origin_beats`**, and repositioning moves the origin. And `beats_per_bar` lives here rather than on a node because MIDI clock carries no time signature: bars are authored, once, somewhere every reader agrees on.

**Files:**
- Create: `crates/sway-graph/src/transport.rs`
- Modify: `crates/sway-graph/src/lib.rs`
- Modify: `crates/sway-graph/src/tick.rs:210-218` (`GraphPlugin::build`)

**Interfaces:**
- Produces: `Transport { state, secs_per_beat, beats_per_bar, origin_beats, locked }`, `TransportState::{Stopped, Playing}`, `MusicalTime { bar, beat, sixteenth, bar_phase }` with `Display`, and the `TransportTime` extension trait on `Time<Transport>` — `beats()`, `beats_total()`, `bpm()`, `state()`, `is_playing()`, `position()`, `reposition(beats)`. Task 5 advances it, Tasks 6–7 read it, Task 9 displays it.
- Consumes: nothing.

- [ ] **Step 1: Write the failing tests**

Create `crates/sway-graph/src/transport.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    fn clock() -> Time<Transport> {
        Time::new_with(Transport::default())
    }

    #[test]
    fn the_default_transport_is_stopped_at_120_bpm_in_four_four() {
        let time = clock();
        assert_eq!(time.state(), TransportState::Stopped);
        assert!((time.bpm() - 120.0).abs() < 1e-9);
        assert_eq!(time.transport().beats_per_bar, 4);
        assert_eq!(time.beats(), 0.0);
    }

    #[test]
    fn position_is_elapsed_minus_the_origin() {
        let mut time = clock();
        time.advance_by(Duration::from_secs_f64(9.0));
        time.reposition(0.0);
        assert!((time.beats() - 0.0).abs() < 1e-9);
        time.advance_by(Duration::from_secs_f64(2.5));
        assert!((time.beats() - 2.5).abs() < 1e-9);
        assert!((time.beats_total() - 11.5).abs() < 1e-9, "the clock itself never rewinds");
    }

    #[test]
    fn a_reposition_can_move_the_origin_forward_or_back() {
        let mut time = clock();
        time.advance_by(Duration::from_secs_f64(16.0));
        time.reposition(4.0);
        assert!((time.beats() - 4.0).abs() < 1e-9);
        time.reposition(0.0);
        assert!((time.beats() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn musical_time_counts_bars_beats_and_sixteenths_from_one() {
        // Beat 0 is bar 1, beat 1, sixteenth 1 — musicians count from one and
        // the readout has to match what the sequencer shows.
        assert_eq!(
            MusicalTime::from_beats(0.0, 4),
            MusicalTime { bar: 1, beat: 1, sixteenth: 1, bar_phase: 0.0 }
        );
        let at = MusicalTime::from_beats(17.5, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (5, 2, 3));
    }

    #[test]
    fn musical_time_honours_a_three_four_bar() {
        let at = MusicalTime::from_beats(7.0, 3);
        assert_eq!((at.bar, at.beat), (3, 2), "beat 7 is bar 3 beat 2 in 3/4");
    }

    #[test]
    fn musical_time_displays_as_bar_dot_beat_dot_sixteenth() {
        assert_eq!(MusicalTime::from_beats(16.25, 4).to_string(), "005.1.2");
    }

    #[test]
    fn a_negative_position_clamps_to_the_start_rather_than_wrapping() {
        // `reposition` can leave the origin ahead of elapsed for one tick.
        // Bar 0 or a negative sixteenth on the readout is worse than a clamp.
        let at = MusicalTime::from_beats(-3.0, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (1, 1, 1));
    }

    #[test]
    fn bpm_and_secs_per_beat_are_each_other_inverses() {
        let mut time = clock();
        time.context_mut().secs_per_beat = 60.0 / 137.0;
        assert!((time.bpm() - 137.0).abs() < 1e-9);
    }

    #[test]
    fn the_graph_plugin_inserts_the_transport_clock() {
        let mut app = bevy_app::App::new();
        app.add_plugins(crate::tick::GraphPlugin);
        assert!(app.world().get_resource::<Time<Transport>>().is_some());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph transport`
Expected: compile error — `Transport` does not exist.

- [ ] **Step 3: Write the clock**

Above the test module in `crates/sway-graph/src/transport.rs`:

```rust
//! `Time<Transport>` — beat time as a Bevy clock (parent §2.7).
//!
//! `Time<T>` is generic over a clock, so the transport *is* one: its elapsed
//! time is measured in beats and its advance per tick is whatever the phase
//! estimator says. A tempo-synced node takes `Res<Time<Transport>>` and is
//! otherwise an ordinary node; stop is a clock that stops advancing.
//!
//! Nothing here knows about MIDI. `sway-graph` must not learn what a pulse is
//! (parent §2), and the editor — which may not depend on `sway-nodes` — reads
//! this type directly.
//!
//! **The clock never rewinds.** `Time::advance_by` takes a `Duration`, so a
//! Start cannot reset it. Musical position is `elapsed - origin_beats`, and
//! repositioning moves the origin.

use core::fmt;

use bevy_reflect::Reflect;
use bevy_time::Time;

/// Whether beat time is advancing.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    #[default]
    Stopped,
    Playing,
}

/// The transport clock's context.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub state: TransportState,
    /// Best current estimate of one beat's duration, in seconds. Survives a
    /// clock dropout: freewheeling advances at exactly this rate.
    pub secs_per_beat: f64,
    /// Beats in a bar. MIDI clock carries no time signature, so this is
    /// authored — and it lives here rather than on a node so the readout and
    /// every transport-aware node agree about where a bar starts.
    pub beats_per_bar: u32,
    /// The clock's elapsed beats at musical position zero.
    pub origin_beats: f64,
    /// Whether the phase estimator currently has a lock. Purely informational
    /// — freewheeling and locked both advance beat time.
    pub locked: bool,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            state: TransportState::Stopped,
            secs_per_beat: 0.5, // 120 BPM
            beats_per_bar: 4,
            origin_beats: 0.0,
            locked: false,
        }
    }
}

/// Musical position: bars, beats and sixteenths, counted from one.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct MusicalTime {
    pub bar: u32,
    pub beat: u32,
    pub sixteenth: u32,
    /// How far through the bar, in `0.0..1.0`.
    pub bar_phase: f32,
}

impl MusicalTime {
    /// Converts a beat position into bars, beats and sixteenths.
    ///
    /// A negative position clamps to the start: `reposition` can briefly
    /// leave the origin ahead of elapsed, and bar 0 on the readout is worse
    /// than a clamp.
    pub fn from_beats(beats: f64, beats_per_bar: u32) -> Self {
        let per_bar = beats_per_bar.max(1) as f64;
        let beats = beats.max(0.0);
        let in_bar = beats % per_bar;
        Self {
            bar: (beats / per_bar) as u32 + 1,
            beat: in_bar as u32 + 1,
            sixteenth: (in_bar.fract() * 4.0) as u32 + 1,
            bar_phase: (in_bar / per_bar) as f32,
        }
    }
}

impl fmt::Display for MusicalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03}.{}.{}", self.bar, self.beat, self.sixteenth)
    }
}

/// Beat-time accessors on `Time<Transport>`.
pub trait TransportTime {
    fn transport(&self) -> &Transport;
    fn transport_mut(&mut self) -> &mut Transport;
    /// Musical position, in beats since the last reposition. Never negative.
    fn beats(&self) -> f64;
    /// The clock's own elapsed beats, before the origin is subtracted. This
    /// is monotone, and it is what a beat-boundary search works in.
    fn beats_total(&self) -> f64;
    fn bpm(&self) -> f64;
    fn state(&self) -> TransportState;
    fn is_playing(&self) -> bool;
    fn position(&self) -> MusicalTime;
    /// Moves musical position zero so that `beats()` reads `beats`.
    fn reposition(&mut self, beats: f64);
}

impl TransportTime for Time<Transport> {
    fn transport(&self) -> &Transport {
        self.context()
    }

    fn transport_mut(&mut self) -> &mut Transport {
        self.context_mut()
    }

    fn beats(&self) -> f64 {
        (self.beats_total() - self.context().origin_beats).max(0.0)
    }

    fn beats_total(&self) -> f64 {
        self.elapsed_secs_f64()
    }

    fn bpm(&self) -> f64 {
        let spb = self.context().secs_per_beat;
        if spb > 0.0 { 60.0 / spb } else { 0.0 }
    }

    fn state(&self) -> TransportState {
        self.context().state
    }

    fn is_playing(&self) -> bool {
        self.state() == TransportState::Playing
    }

    fn position(&self) -> MusicalTime {
        MusicalTime::from_beats(self.beats(), self.context().beats_per_bar)
    }

    fn reposition(&mut self, beats: f64) {
        let origin = self.beats_total() - beats;
        self.context_mut().origin_beats = origin;
    }
}
```

- [ ] **Step 4: Wire it into `GraphPlugin` and the crate's exports**

In `crates/sway-graph/src/lib.rs` add `pub mod transport;` and:

```rust
pub use transport::{MusicalTime, Transport, TransportState, TransportTime};
```

In `crates/sway-graph/src/tick.rs`, add `use bevy_time::Time;` to the existing `bevy_time` import and extend `GraphPlugin::build`:

```rust
        app.insert_resource(PortArena::new(0))
            .init_resource::<NodeTypeRegistry>()
            .init_resource::<GraphTickCount>()
            .init_resource::<Time<crate::transport::Transport>>()
            .register_type::<crate::edges::EditorPos>()
            .register_type::<crate::transport::Transport>()
            .register_type::<crate::transport::TransportState>()
            .add_systems(FixedUpdate, graph_tick);
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-graph transport`
Expected: PASS, 9 tests. Then `cargo test -p sway-graph` — PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): Time<Transport>, beat time as a Bevy clock"
```

---

### Task 5: `advance_transport` — joining MIDI to the clock

The one system where the three previous tasks meet. It runs in `FixedUpdate` after `drain_inbox` and before `graph_tick` — exactly where parent §2.11's schedule sketch puts "advance `Time<Transport>` from the phase estimator" — and it is the only place in the codebase that turns a status byte into beats.

Three behaviours are worth naming before the code. **Locked, it differences the fit**: `beats_at(tick_end) − beats_at(tick_start)` is a phase-correcting advance, so a slightly wrong period corrects itself rather than accumulating. **Unlocked, it freewheels**: `dt / secs_per_beat` at the last known tempo, which is the dropout policy. And **it never differences across a re-lock**: the estimator's generation changes when the fit restarts, and a changed generation freewheels for that tick rather than jumping to a new fit's arbitrary origin.

**Files:**
- Create: `crates/sway-nodes/src/transport.rs`
- Modify: `crates/sway-nodes/Cargo.toml` (add `sway-midi`)
- Modify: `crates/sway-nodes/src/lib.rs` (add the module)
- Modify: `crates/sway-nodes/src/midi.rs:194-210` (`SignalNodesPlugin`)
- Modify: `Cargo.toml` — nothing; `sway-midi` is already a workspace dependency

**Interfaces:**
- Consumes: `sway_midi::{ClockEstimator, CLOCK, START, CONTINUE, STOP, SONG_POSITION}` (Tasks 1–2), `sway_graph::{Transport, TransportState, TransportTime}` (Task 4), `sway_nodes::TickMidi` (existing).
- Produces: `TransportClock` (the resource holding the estimator) and `advance_transport`. Tasks 6–7's nodes read the clock it advances; Task 8 traces it.

- [ ] **Step 1: Add the dependency**

In `crates/sway-nodes/Cargo.toml`, under `[dependencies]`:

```toml
# The phase estimator (parent §3 puts it in sway-midi). Pure math — this
# pulls in no Bevy and no new transitive dependency.
sway-midi.workspace = true
```

- [ ] **Step 2: Write the failing tests**

Create `crates/sway-nodes/src/transport.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use sway_graph::{GraphPlugin, Transport, TransportState, TransportTime};

    use super::*;
    use crate::{MidiInbox, RawMidi, SignalNodesPlugin};

    const TICK_HZ: f64 = 120.0;
    const SECS_PER_PULSE_120: f64 = 0.5 / 24.0;

    fn transport_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, SignalNodesPlugin));
        app.update();
        app
    }

    fn raw(status: u8) -> RawMidi {
        RawMidi { status, data1: 0, data2: 0 }
    }

    /// Queues `beats` worth of 24 ppqn pulses from `start`, at `bpm`.
    /// Returns the time just past the last pulse.
    fn queue_clock(app: &mut App, start: f64, bpm: f64, beats: f64) -> f64 {
        let spp = (60.0 / bpm) / 24.0;
        let pulses = (beats * 24.0) as usize;
        let mut inbox = app.world_mut().resource_mut::<MidiInbox>();
        for pulse in 0..pulses {
            inbox.push(start + pulse as f64 * spp, raw(sway_midi::CLOCK));
        }
        start + pulses as f64 * spp
    }

    fn run_until(app: &mut App, seconds: f64) {
        for _ in 0..((seconds * TICK_HZ) as usize) {
            app.update();
        }
    }

    fn beats(app: &App) -> f64 {
        app.world().resource::<Time<Transport>>().beats()
    }

    fn bpm(app: &App) -> f64 {
        app.world().resource::<Time<Transport>>().bpm()
    }

    #[test]
    fn a_steady_clock_train_locks_to_its_tempo() {
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);

        assert!((bpm(&app) - 120.0).abs() < 0.5, "locked to {} BPM", bpm(&app));
        assert!(app.world().resource::<Time<Transport>>().transport().locked);
    }

    #[test]
    fn beats_advance_one_per_half_second_at_120_bpm() {
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 8.0);
        run_until(&mut app, 2.0);

        // Two seconds of transport at 120 BPM is four beats, within the one
        // tick of quantization a mid-tick Start costs.
        assert!((beats(&app) - 4.0).abs() < 0.1, "advanced {} beats", beats(&app));
    }

    #[test]
    fn a_stopped_transport_does_not_advance_however_many_pulses_arrive() {
        let mut app = transport_app();
        // No Start: pulses set the tempo but must not scroll the visuals.
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);

        assert_eq!(app.world().resource::<Time<Transport>>().state(), TransportState::Stopped);
        assert_eq!(beats(&app), 0.0);
        assert!((bpm(&app) - 120.0).abs() < 0.5, "tempo is still tracked while stopped");
    }

    #[test]
    fn stop_freezes_beat_time_and_continue_resumes_it_where_it_stopped() {
        // The two pulse trains must not overlap in time: two pulses at
        // nearly the same instant are one pulse index apart and no time
        // apart, which collapses the fit's slope. The first train therefore
        // stops at t=1.0, exactly where the transport does.
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 2.0); // pulses over [0.0, 1.0)
        app.world_mut().resource_mut::<MidiInbox>().push(1.0, raw(sway_midi::STOP));
        run_until(&mut app, 1.5);
        let frozen = beats(&app);

        run_until(&mut app, 0.5);
        assert_eq!(beats(&app), frozen, "a stopped transport must not advance");

        app.world_mut().resource_mut::<MidiInbox>().push(2.0, raw(sway_midi::CONTINUE));
        queue_clock(&mut app, 2.0, 120.0, 4.0); // pulses over [2.0, 4.0)
        run_until(&mut app, 1.0);
        assert!(beats(&app) > frozen, "continue resumes");
        assert!(beats(&app) < frozen + 2.5, "continue resumes, it does not restart");
    }

    #[test]
    fn start_puts_the_playhead_back_at_the_top() {
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 8.0);
        run_until(&mut app, 2.0);
        assert!(beats(&app) > 3.0);

        app.world_mut().resource_mut::<MidiInbox>().push(2.0, raw(sway_midi::START));
        app.update();
        assert!(beats(&app) < 0.1, "Start is position zero, got {}", beats(&app));
    }

    #[test]
    fn a_song_position_pointer_repositions_in_sixteenths() {
        let mut app = transport_app();
        // SPP counts sixteenths: 8 sixteenths is two beats.
        app.world_mut().resource_mut::<MidiInbox>().push(
            0.0,
            RawMidi { status: sway_midi::SONG_POSITION, data1: 8, data2: 0 },
        );
        app.update();
        assert!((beats(&app) - 2.0).abs() < 0.05, "got {}", beats(&app));
    }

    #[test]
    fn a_clock_dropout_freewheels_at_the_last_tempo() {
        // The chosen dropout policy: never freeze the screen. A cable glitch
        // costs drift, not a stopped visual.
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);
        let before = beats(&app);

        // One full second with no pulses at all.
        run_until(&mut app, 1.0);

        let advanced = beats(&app) - before;
        assert!(
            (advanced - 2.0).abs() < 0.1,
            "freewheeling must advance two beats in a second at 120 BPM, got {advanced}"
        );
    }

    #[test]
    fn the_clock_re_locks_after_a_dropout_without_jumping() {
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);
        run_until(&mut app, 1.0); // dropout
        let before = beats(&app);

        queue_clock(&mut app, 3.0, 120.0, 4.0);
        run_until(&mut app, 0.2);

        let advanced = beats(&app) - before;
        assert!(advanced >= 0.0, "beats never run backwards");
        assert!(advanced < 1.0, "re-locking must not jump a fit's origin into position: {advanced}");
    }

    #[test]
    fn a_tempo_change_is_followed() {
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        let end = queue_clock(&mut app, 0.0, 120.0, 4.0);
        queue_clock(&mut app, end, 90.0, 6.0);
        run_until(&mut app, 6.0);

        assert!((bpm(&app) - 90.0).abs() < 1.0, "followed to {} BPM", bpm(&app));
    }

    #[test]
    fn the_system_runs_between_the_inbox_drain_and_the_graph_tick() {
        // Ordering, asserted rather than assumed: a node reading beat time in
        // its tick must see this tick's advance, not the previous one's.
        let mut app = transport_app();
        app.world_mut().resource_mut::<MidiInbox>().push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 2.0);
        app.update();
        assert_eq!(
            app.world().resource::<Time<Transport>>().state(),
            TransportState::Playing,
            "a Start drained this tick must take effect this tick"
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p sway-nodes transport`
Expected: compile error — `advance_transport` and `TransportClock` do not exist.

- [ ] **Step 4: Write the system**

Above the test module in `crates/sway-nodes/src/transport.rs`:

```rust
//! `advance_transport` — the one system joining MIDI clock to beat time.
//!
//! Runs in `FixedUpdate` between `drain_inbox` and `graph_tick`, which is
//! where parent §2.11's schedule puts "advance `Time<Transport>` from the
//! phase estimator". This is the only place in the codebase that turns a
//! status byte into beats: `sway-graph` owns the clock and knows no MIDI,
//! `sway-midi` owns the estimator and knows no world.
//!
//! Locked, the advance is `beats_at(end) - beats_at(start)` — a phase
//! correction, so a slightly wrong period corrects rather than accumulates.
//! Unlocked, it freewheels at the last known tempo, which is the dropout
//! policy: never freeze the screen. It never differences across a re-lock,
//! because a new fit has its own arbitrary origin — the estimator's
//! generation is what makes that detectable.

use core::time::Duration;

use bevy_ecs::resource::Resource;
use bevy_ecs::system::{Res, ResMut};
use bevy_time::{Fixed, Time};
use sway_graph::{Transport, TransportState, TransportTime};
use sway_midi::ClockEstimator;

use crate::TickMidi;

/// The phase estimator's home in the world, plus the bookkeeping that makes
/// differencing safe across ticks.
#[derive(Resource, Default)]
pub struct TransportClock {
    pub estimator: ClockEstimator,
    /// `(generation, beat position)` at the end of the previous tick. A
    /// generation mismatch means the fit restarted, so this tick freewheels
    /// instead of differencing across the seam.
    last: Option<(u64, f64)>,
}

/// Advances `Time<Transport>` by this tick's worth of beats.
pub fn advance_transport(
    tick_midi: Res<TickMidi>,
    fixed: Res<Time<Fixed>>,
    mut clock: ResMut<TransportClock>,
    mut time: ResMut<Time<Transport>>,
) {
    let dt = fixed.delta_secs_f64();
    let tick_end = fixed.elapsed_secs_f64();
    let tick_start = tick_end - dt;

    // A reposition is applied *after* the advance, because the clock it is
    // relative to has not moved yet. The cost is that a Start landing
    // mid-tick puts position zero at the tick boundary rather than at its own
    // sub-tick offset — under 9ms at 120Hz, and the alternative is a second
    // advance path.
    let mut reposition: Option<f64> = None;

    for &(offset, message) in &tick_midi.events {
        match message.status {
            sway_midi::CLOCK => clock.estimator.push_pulse(tick_start + offset as f64),
            sway_midi::START => {
                time.transport_mut().state = TransportState::Playing;
                reposition = Some(0.0);
            }
            sway_midi::CONTINUE => time.transport_mut().state = TransportState::Playing,
            sway_midi::STOP => time.transport_mut().state = TransportState::Stopped,
            sway_midi::SONG_POSITION => {
                // 14-bit, LSB first, counting sixteenths.
                let sixteenths = (u16::from(message.data2) << 7) | u16::from(message.data1);
                reposition = Some(f64::from(sixteenths) / 4.0);
            }
            _ => {}
        }
    }

    // Tempo tracking runs whether or not the transport is playing: a device
    // that free-runs its clock should not have to press play for the readout
    // to be right.
    let generation = clock.estimator.generation();
    if let Some(secs_per_beat) = clock.estimator.secs_per_beat() {
        time.transport_mut().secs_per_beat = secs_per_beat;
    }
    time.transport_mut().locked = clock.estimator.is_locked();

    let position = clock.estimator.beats_at(tick_end);
    let delta = if time.state() == TransportState::Stopped {
        0.0
    } else {
        match (position, clock.last) {
            (Some(now), Some((previous_generation, previous)))
                if previous_generation == generation =>
            {
                // Locked: follow the fit. `max(0.0)` is the no-rewind rule —
                // a backwards phase correction stalls for one tick.
                (now - previous).max(0.0)
            }
            // Unlocked, freshly locked, or re-locked: freewheel.
            _ => dt / time.transport().secs_per_beat.max(f64::MIN_POSITIVE),
        }
    };
    clock.last = position.map(|beats| (generation, beats));

    if delta.is_finite() && delta > 0.0 {
        time.advance_by(Duration::from_secs_f64(delta));
    } else {
        time.advance_by(Duration::ZERO);
    }

    if let Some(beats) = reposition {
        time.reposition(beats);
    }
}
```

- [ ] **Step 5: Register it**

In `crates/sway-nodes/src/lib.rs` add `mod transport;` and `pub use transport::*;` alongside the others.

In `crates/sway-nodes/src/midi.rs`, extend `SignalNodesPlugin::build`:

```rust
        app.init_resource::<MidiInbox>()
            .init_resource::<TickMidi>()
            .init_resource::<crate::TransportClock>()
            .add_systems(
                FixedUpdate,
                (
                    drain_inbox,
                    crate::advance_transport.after(drain_inbox),
                )
                    .before(graph_tick),
            );
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes transport`
Expected: PASS, 10 tests. Then `cargo test --workspace` — PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): advance Time<Transport> from the MIDI clock estimator"
```

---

### Task 6: `TransportTime` and `SyncLfo`

The two value-only transport nodes. Both are pure functions of the beat position the previous task maintains, which is the beat-time restatement of parent §2.2's rule that nodes derive time-varying values from absolute time rather than accumulating.

`TransportTimeNode` has **no inlets at all** — `beats_per_bar` lives on the clock (Task 4's decision 6), so there is nothing left to author. That is a first for this engine and is tested explicitly rather than assumed: `derive_fields`, `prefill_of` and `compile`'s layout all have to handle a zero-field `Inlets` struct.

`SyncLfo` is a separate node type from `LFO` rather than a mode param on it, per parent §2.4. Its waveform evaluation is shared with `LFO` by extracting the existing `match` into a function — the same shape, driven by a beat phase instead of a wall-clock one.

**Files:**
- Create: `crates/sway-nodes/src/beat.rs`
- Modify: `crates/sway-nodes/src/lfo.rs:67-89` (extract `wave`)
- Modify: `crates/sway-nodes/src/lib.rs`
- Modify: `crates/sway-nodes/src/midi.rs` (`SignalNodesPlugin` registrations)

**Interfaces:**
- Consumes: `sway_graph::{Time<Transport>, TransportTime, MusicalTime}` (Task 4), `crate::Waveform` (existing).
- Produces: `TransportTimeNode` with outlets `beats`, `bar`, `beat`, `sixteenth`, `bar_phase`, `bpm`, `playing` (all `f32`), and `SyncLfo` with inlets `beats`, `shape`, `phase`, `amplitude` and outlet `value`. Task 8 traces them; Task 10 wires them into the demo graph.
- Also produces `pub(crate) fn wave(shape: Waveform, phase: f32) -> f32` in `lfo.rs`.

- [ ] **Step 1: Extract the waveform function**

In `crates/sway-nodes/src/lfo.rs`, replace the `let wave = match shape { ... };` block inside `LFO::tick` with a call, and add the function below the `impl NodeType for LFO` block:

```rust
/// One cycle of a waveform, bipolar, at `phase` in `0.0..1.0`.
///
/// Shared with `SyncLfo`: the only difference between a wall-clock LFO and a
/// tempo-synced one is where the phase comes from, and duplicating four lines
/// of trigonometry to say so would be worse than exposing this.
pub(crate) fn wave(shape: Waveform, phase: f32) -> f32 {
    match shape {
        Waveform::Sine => (phase * TAU).sin(),
        Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        Waveform::Saw => 2.0 * phase - 1.0,
        Waveform::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}
```

`LFO::tick`'s last two lines become:

```rust
        let p = (ctx.tick_start * hz as f64 + phase as f64).rem_euclid(1.0) as f32;
        ports.write(Self::OUT_VALUE, wave(shape, p) * amplitude);
```

Run: `cargo test -p sway-nodes lfo`
Expected: PASS — the extraction changes nothing observable.

- [ ] **Step 2: Write the failing tests**

Create `crates/sway-nodes/src/beat.rs` with only this test module (the `mod tests` block will grow again in Task 7):

```rust
#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::entity::Entity;
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use sway_graph::{
        CompiledGraph, GraphNode, GraphPlugin, NodeId, NodeType, NodeTypeRegistry, PortArena,
        Transport, TransportState, TransportTime, compile,
    };

    use super::*;
    use crate::Waveform;

    const TICK_HZ: f64 = 120.0;

    /// Registers the transport node types **without** `SignalNodesPlugin`.
    ///
    /// That plugin also installs `advance_transport`, which would freewheel
    /// the clock underneath every assertion below — at 120 BPM and a 120 Hz
    /// tick that is an extra 1/60 beat per `app.update()`, which turns every
    /// exact count and every phase comparison into an approximation. These
    /// tests are about what the nodes *read*; what advances the clock has its
    /// own suite in `transport.rs`.
    fn beat_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(GraphPlugin);
        sway_graph::register_node_type::<TransportTimeNode>(&mut app);
        sway_graph::register_node_type::<SyncLfo>(&mut app);
        app.update();
        app
    }

    fn node_type_id<N: NodeType>(app: &App) -> sway_graph::NodeTypeId {
        app.world()
            .resource::<NodeTypeRegistry>()
            .id_of(core::any::type_name::<N>())
            .expect("node type registered by beat_app")
    }

    fn compile_graph(app: &mut App) {
        let compiled = compile(app.world_mut()).expect("compiles");
        let slots_len = compiled.slots_len;
        app.world_mut().resource_mut::<PortArena>().resize(slots_len);
        app.world_mut().insert_resource(compiled);
    }

    /// Puts the transport at an exact beat position and lets it run.
    fn play_at(app: &mut App, bpm: f64) {
        let mut time = app.world_mut().resource_mut::<Time<Transport>>();
        time.transport_mut().state = TransportState::Playing;
        time.transport_mut().secs_per_beat = 60.0 / bpm;
        time.reposition(0.0);
    }

    fn out(app: &App, node: Entity, ordinal: u16) -> f32 {
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("compiled");
        let slot = plan.base + plan.field_offsets[ordinal as usize];
        *app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<f32>()
            .expect("outlet is f32")
    }

    fn spawn_transport_time(app: &mut App) -> Entity {
        let node_type = node_type_id::<TransportTimeNode>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(0), node_type },
                TransportTimeInlets::default(),
                TransportTimeState,
            ))
            .id()
    }

    #[test]
    fn a_node_with_no_inlets_compiles_and_ticks() {
        // TransportTimeNode is the first node type in this engine with an
        // empty Inlets struct — beats_per_bar belongs to the clock, not to a
        // node. Field derivation, prefill and the arena layout all have to
        // survive zero inlet fields.
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.update();

        assert!(out(&app, node, TransportTimeNode::OUT_BPM) > 0.0);
    }

    #[test]
    fn transport_time_reports_bar_beat_and_sixteenth_from_one() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);
        // Beat 17.5 is bar 5, beat 2, sixteenth 3 in 4/4.
        app.world_mut().resource_mut::<Time<Transport>>().reposition(17.5);

        app.update();

        assert_eq!(out(&app, node, TransportTimeNode::OUT_BAR), 5.0);
        assert_eq!(out(&app, node, TransportTimeNode::OUT_BEAT), 2.0);
        assert_eq!(out(&app, node, TransportTimeNode::OUT_SIXTEENTH), 3.0);
    }

    #[test]
    fn transport_time_reports_whether_the_transport_is_playing() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);

        app.update();
        assert_eq!(out(&app, node, TransportTimeNode::OUT_PLAYING), 0.0);

        play_at(&mut app, 120.0);
        app.update();
        assert_eq!(out(&app, node, TransportTimeNode::OUT_PLAYING), 1.0);
    }

    #[test]
    fn transport_time_bar_phase_sweeps_zero_to_one_across_a_bar() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.world_mut().resource_mut::<Time<Transport>>().reposition(2.0);
        app.update();
        let half = out(&app, node, TransportTimeNode::OUT_BAR_PHASE);
        assert!((half - 0.5).abs() < 0.02, "two beats into a 4/4 bar is {half}");
    }

    fn spawn_sync_lfo(app: &mut App, beats: f32, shape: Waveform) -> Entity {
        let node_type = node_type_id::<SyncLfo>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(1), node_type },
                SyncLfoInlets { beats, shape, phase: 0.0, amplitude: 1.0 },
                SyncLfoState,
            ))
            .id()
    }

    #[test]
    fn a_sync_lfo_completes_one_cycle_per_period_in_beats() {
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 4.0, Waveform::Saw);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        // A saw over four beats: 0 beats is -1, 2 beats is 0, just under 4
        // beats is nearly +1.
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.0);
        app.update();
        assert!((out(&app, node, SyncLfo::OUT_VALUE) + 1.0).abs() < 0.02);

        app.world_mut().resource_mut::<Time<Transport>>().reposition(2.0);
        app.update();
        assert!(out(&app, node, SyncLfo::OUT_VALUE).abs() < 0.02);
    }

    #[test]
    fn a_sync_lfo_holds_its_phase_when_the_tempo_changes() {
        // The point of tempo sync: at a given beat position the output is the
        // same regardless of how fast the beats went by.
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 2.0, Waveform::Sine);
        compile_graph(&mut app);

        play_at(&mut app, 120.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.5);
        app.update();
        let at_120 = out(&app, node, SyncLfo::OUT_VALUE);

        play_at(&mut app, 174.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.5);
        app.update();
        let at_174 = out(&app, node, SyncLfo::OUT_VALUE);

        assert!((at_120 - at_174).abs() < 1e-5, "{at_120} vs {at_174}");
    }

    #[test]
    fn a_sync_lfo_with_a_zero_or_negative_period_holds_still_rather_than_dividing_by_zero() {
        // The tick is infallible: an authored 0 must not produce NaN.
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 0.0, Waveform::Sine);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.update();

        assert!(out(&app, node, SyncLfo::OUT_VALUE).is_finite());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p sway-nodes beat`
Expected: compile error — `TransportTimeNode` and `SyncLfo` do not exist.

- [ ] **Step 4: Write the two node types**

Above the test module in `crates/sway-nodes/src/beat.rs`:

```rust
//! Transport-aware nodes (parent §5, M3): a beat time base, a tempo-synced
//! oscillator, and a beat-quantised trigger.
//!
//! All three are pure functions of the beat position `advance_transport`
//! maintains. That is parent §2.2's rule — derive from absolute time, never
//! accumulate — restated in beats: a dropped tick, a tempo change and a
//! reposition all leave the output correct, because nothing here remembers
//! where it was last tick.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::Time;
use sway_graph::{MusicalTime, NodeType, PortView, TickCtx, Transport, TransportTime};

use crate::Waveform;
use crate::lfo::wave;

/// Beat time as ports: what bar, beat and sixteenth it is, and how fast.
///
/// No inlets. `beats_per_bar` belongs to `Transport` rather than to this node,
/// because the editor readout and every other transport-aware node have to
/// agree about where a bar starts (Task 4).
#[derive(Reflect, Component, Default)]
pub struct TransportTimeInlets {}

#[derive(Reflect, Default)]
pub struct TransportTimeOutlets {
    /// Musical position, in beats since the last reposition.
    pub beats: f32,
    /// Bar, beat and sixteenth, counted from one, as the sequencer shows them.
    pub bar: f32,
    pub beat: f32,
    pub sixteenth: f32,
    /// How far through the bar, `0.0..1.0`. The one output that is directly
    /// useful as a driver.
    pub bar_phase: f32,
    pub bpm: f32,
    /// 1.0 while playing, 0.0 while stopped.
    pub playing: f32,
}

#[derive(Component, Default)]
pub struct TransportTimeState;

pub struct TransportTimeNode;

impl TransportTimeNode {
    pub const OUT_BEATS: u16 = 0;
    pub const OUT_BAR: u16 = 1;
    pub const OUT_BEAT: u16 = 2;
    pub const OUT_SIXTEENTH: u16 = 3;
    pub const OUT_BAR_PHASE: u16 = 4;
    pub const OUT_BPM: u16 = 5;
    pub const OUT_PLAYING: u16 = 6;
}

impl NodeType for TransportTimeNode {
    type Inlets = TransportTimeInlets;
    type Outlets = TransportTimeOutlets;
    type State = TransportTimeState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("beats", Self::OUT_BEATS),
        ("bar", Self::OUT_BAR),
        ("beat", Self::OUT_BEAT),
        ("sixteenth", Self::OUT_SIXTEENTH),
        ("bar_phase", Self::OUT_BAR_PHASE),
        ("bpm", Self::OUT_BPM),
        ("playing", Self::OUT_PLAYING),
    ];

    fn register(_app: &mut App) {}

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let time = world.resource::<Time<Transport>>();
        let beats = time.beats();
        let at = time.position();
        let bpm = time.bpm();
        let playing = time.is_playing();

        ports.write(Self::OUT_BEATS, beats as f32);
        ports.write(Self::OUT_BAR, at.bar as f32);
        ports.write(Self::OUT_BEAT, at.beat as f32);
        ports.write(Self::OUT_SIXTEENTH, at.sixteenth as f32);
        ports.write(Self::OUT_BAR_PHASE, at.bar_phase);
        ports.write(Self::OUT_BPM, bpm as f32);
        ports.write(Self::OUT_PLAYING, if playing { 1.0f32 } else { 0.0 });
    }
}

/// An oscillator whose period is measured in beats rather than seconds.
///
/// A separate node type from `LFO`, not a mode param on it: a type-selector
/// param is a smell and this is the same argument (parent §2.4). The waveform
/// evaluation is shared, because the only real difference is where phase
/// comes from.
#[derive(Reflect, Component, Default)]
pub struct SyncLfoInlets {
    /// Period, in beats. One bar in 4/4 is 4.0.
    pub beats: f32,
    pub shape: Waveform,
    /// Phase offset, in cycles.
    pub phase: f32,
    pub amplitude: f32,
}

#[derive(Reflect, Default)]
pub struct SyncLfoOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct SyncLfoState;

pub struct SyncLfo;

impl SyncLfo {
    pub const BEATS: u16 = 0;
    pub const SHAPE: u16 = 1;
    pub const PHASE: u16 = 2;
    pub const AMPLITUDE: u16 = 3;
    pub const OUT_VALUE: u16 = 4;
}

impl NodeType for SyncLfo {
    type Inlets = SyncLfoInlets;
    type Outlets = SyncLfoOutlets;
    type State = SyncLfoState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("beats", Self::BEATS),
        ("shape", Self::SHAPE),
        ("phase", Self::PHASE),
        ("amplitude", Self::AMPLITUDE),
        ("value", Self::OUT_VALUE),
    ];

    fn register(app: &mut App) {
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<Waveform>();
    }

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let period: f32 = ports.read(Self::BEATS);
        let shape: Waveform = ports.read(Self::SHAPE);
        let phase: f32 = ports.read(Self::PHASE);
        let amplitude: f32 = ports.read(Self::AMPLITUDE);

        // Absolute beat position, never an accumulator — so a tempo change,
        // a reposition and a dropped tick all leave this correct.
        let beats = world.resource::<Time<Transport>>().beats();
        // An authored zero or negative period holds still rather than
        // dividing: the tick is infallible.
        let p = if period > 0.0 {
            (beats / period as f64 + phase as f64).rem_euclid(1.0) as f32
        } else {
            phase.rem_euclid(1.0)
        };
        ports.write(Self::OUT_VALUE, wave(shape, p) * amplitude);
    }
}
```

- [ ] **Step 5: Register the node types**

In `crates/sway-nodes/src/lib.rs` add `mod beat;` and `pub use beat::*;`.

In `crates/sway-nodes/src/midi.rs`'s `SignalNodesPlugin::build`, after `register_node_type::<Select>(app);`:

```rust
        register_node_type::<crate::TransportTimeNode>(app);
        register_node_type::<crate::SyncLfo>(app);
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes beat`
Expected: PASS, 7 tests.

If `a_node_with_no_inlets_compiles_and_ticks` fails inside `derive_fields` or `prefill_of`, the fix belongs in `sway-graph` and stays in this task: a zero-field `Inlets` struct is a legitimate node shape and the engine has simply never seen one. Do **not** work around it by inventing a filler inlet.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): TransportTime and SyncLfo, beat-locked value nodes"
```

---

### Task 7: `BeatTrigger`

The one transport node that emits events, and the only place in this milestone where sub-tick offsets are computed rather than received. A beat boundary almost never lands on a tick boundary, so the trigger inverts the tick's own beat advance to place each crossing inside the window — which is what lets an envelope downstream start at the correct phase (parent §2.4).

Note a real gap this opens, and do not paper over it: `Events<Beat>` has **no consumer node yet**. `Envelope` takes `Events<NoteMsg>`, and adapting one to the other would be a type-selector smell in event clothing. The trigger is exercised by its own tests and by Task 8's golden traces; whether event payloads want a common shape is a question for Task 12's report, not something to invent here.

**Files:**
- Modify: `crates/sway-nodes/src/beat.rs`
- Modify: `crates/sway-nodes/src/midi.rs` (one more registration)
- Modify: `crates/sway-nodes/src/lib.rs:56-60` (the enum-defaults test)

**Interfaces:**
- Consumes: `sway_graph::{Events, MusicalTime, register_events, Time<Transport>, TransportTime}`.
- Produces: `Division::{Bar, Beat, Eighth, Sixteenth}`, `Beat { bar, beat, sixteenth }`, `BeatTrigger` with inlet `division` and outlet `pulse: Events<Beat>`. Task 8 traces it.

- [ ] **Step 1: Write the failing tests**

First extend the fixture — `beat_app()` registers node types explicitly, so a new type has to be added to it:

```rust
        sway_graph::register_node_type::<BeatTrigger>(&mut app);
```

Then append to the `tests` module in `crates/sway-nodes/src/beat.rs`:

```rust
    fn spawn_beat_trigger(app: &mut App, division: Division) -> Entity {
        let node_type = node_type_id::<BeatTrigger>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(2), node_type },
                BeatTriggerInlets { division },
                BeatTriggerState,
            ))
            .id()
    }

    fn pulses(app: &App, node: Entity) -> Vec<sway_graph::Occurrence<Beat>> {
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("compiled");
        let slot = plan.base + plan.field_offsets[BeatTrigger::OUT_PULSE as usize];
        app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<sway_graph::Events<Beat>>()
            .expect("pulse is Events<Beat>")
            .occurrences
            .clone()
    }

    /// Runs `ticks` ticks with the transport advancing `beats_per_tick`, and
    /// returns every occurrence seen, tick by tick.
    fn collect(app: &mut App, node: Entity, ticks: usize, beats_per_tick: f64) -> Vec<Beat> {
        let mut seen = Vec::new();
        for _ in 0..ticks {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(beats_per_tick));
            }
            app.update();
            seen.extend(pulses(app, node).into_iter().map(|o| o.value));
        }
        seen
    }

    #[test]
    fn a_beat_division_fires_once_per_beat() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        // Four beats' worth, at a tenth of a beat per tick.
        let fired = collect(&mut app, node, 40, 0.1);

        assert_eq!(fired.len(), 4, "four beats, four pulses: {fired:?}");
    }

    #[test]
    fn a_bar_division_fires_once_per_bar() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Bar);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 80, 0.1); // eight beats = two bars
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[1].bar, 3, "the second pulse opens bar 3");
    }

    #[test]
    fn a_sixteenth_division_fires_four_times_per_beat() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 20, 0.1); // two beats
        assert_eq!(fired.len(), 8);
    }

    #[test]
    fn a_pulse_carries_the_musical_position_of_its_boundary() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 40, 0.1);
        assert_eq!(
            (fired[0].bar, fired[0].beat, fired[0].sixteenth),
            (1, 2, 1),
            "the first crossing after position 0 is beat 2"
        );
    }

    #[test]
    fn a_pulse_offset_lands_inside_the_tick_window() {
        // Sub-tick timestamps are the whole point of an event port: an
        // envelope downstream starts at the correct phase (parent §2.4).
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let dt = (1.0 / TICK_HZ) as f32;
        for _ in 0..40 {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(0.1));
            }
            app.update();
            for occurrence in pulses(&app, node) {
                assert!(
                    (0.0..=dt).contains(&occurrence.offset),
                    "offset {} outside [0, {dt}]",
                    occurrence.offset
                );
            }
        }
    }

    #[test]
    fn a_boundary_landing_mid_tick_is_not_placed_at_zero() {
        // Half a beat per tick starting from 0.25 beats puts every boundary
        // squarely inside a window; an implementation that stamped 0.0 would
        // pass every count-based test above and still be wrong.
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.25);

        let mut offsets = Vec::new();
        for _ in 0..8 {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(0.5));
            }
            app.update();
            offsets.extend(pulses(&app, node).into_iter().map(|o| o.offset));
        }

        assert!(!offsets.is_empty());
        assert!(
            offsets.iter().any(|&o| o > 1e-6),
            "every offset was zero — the boundary was not located inside the window"
        );
    }

    #[test]
    fn a_stopped_transport_fires_nothing() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        // No play_at: the transport is stopped and never advances.

        for _ in 0..40 {
            app.update();
            assert!(pulses(&app, node).is_empty());
        }
    }

    #[test]
    fn a_long_freeze_does_not_flood_a_single_tick() {
        // A stalled app resuming, or a reposition far ahead, must not emit
        // thousands of occurrences in one tick.
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.advance_by(core::time::Duration::from_secs_f64(1000.0));
        }
        app.update();

        assert!(
            pulses(&app, node).len() <= MAX_PULSES_PER_TICK,
            "a thousand beats in one tick produced {} occurrences",
            pulses(&app, node).len()
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-nodes beat`
Expected: compile error — `BeatTrigger`, `Division` and `Beat` do not exist.

- [ ] **Step 3: Write the node**

Append to `crates/sway-nodes/src/beat.rs`, above the test module:

```rust
/// How often a [`BeatTrigger`] fires.
///
/// An enum-valued behaviour param, in the same family as `LFO.shape` and
/// `Math.op` — not a type selector. It changes a number, not which node this
/// is (parent §2.4).
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    Bar,
    #[default]
    Beat,
    Eighth,
    Sixteenth,
}

impl Division {
    /// This division's length, in beats.
    pub fn beats(self, beats_per_bar: u32) -> f64 {
        match self {
            Self::Bar => beats_per_bar.max(1) as f64,
            Self::Beat => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
        }
    }
}

/// What a [`BeatTrigger`] emits: the musical position of the boundary it
/// fired on.
#[derive(Reflect, Default, Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub bar: u32,
    pub beat: u32,
    pub sixteenth: u32,
}

/// Ceiling on occurrences per tick. A tick that somehow spans a thousand
/// beats — a stalled app resuming, a reposition far ahead — must not flood
/// every downstream event list; the tick is infallible and this is what makes
/// it so here.
pub const MAX_PULSES_PER_TICK: usize = 64;

#[derive(Reflect, Component, Default)]
pub struct BeatTriggerInlets {
    pub division: Division,
}

#[derive(Reflect, Default)]
pub struct BeatTriggerOutlets {
    pub pulse: sway_graph::Events<Beat>,
}

#[derive(Component, Default)]
pub struct BeatTriggerState;

pub struct BeatTrigger;

impl BeatTrigger {
    pub const DIVISION: u16 = 0;
    pub const OUT_PULSE: u16 = 1;
}

impl NodeType for BeatTrigger {
    type Inlets = BeatTriggerInlets;
    type Outlets = BeatTriggerOutlets;
    type State = BeatTriggerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("division", Self::DIVISION), ("pulse", Self::OUT_PULSE)];

    fn register(app: &mut App) {
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<Division>();
        sway_graph::register_events::<Beat>(app);
    }

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, ctx: &TickCtx) {
        let division: Division = ports.read(Self::DIVISION);

        let (playing, beats_per_bar, end, advanced) = {
            let time = world.resource::<Time<Transport>>();
            (
                time.is_playing(),
                time.transport().beats_per_bar,
                time.beats(),
                time.delta_secs_f64(),
            )
        };
        if !playing || advanced <= 0.0 {
            return;
        }

        let step = division.beats(beats_per_bar);
        let start = (end - advanced).max(0.0);

        // Every multiple of `step` in `(start, end]`. Half-open at the start,
        // so a boundary is never emitted twice across two ticks.
        let first = (start / step).floor() as i64 + 1;
        let last = (end / step).floor() as i64;
        for index in first..=last.min(first + MAX_PULSES_PER_TICK as i64 - 1) {
            let boundary = index as f64 * step;
            // Invert this tick's own advance to place the crossing inside
            // the window. Linear within a tick, which is exact for a steady
            // tempo and within a tick's worth of error otherwise.
            let offset = (ctx.dt as f64 * (boundary - start) / advanced)
                .clamp(0.0, ctx.dt as f64) as f32;
            let at = MusicalTime::from_beats(boundary, beats_per_bar);
            ports.emit(
                Self::OUT_PULSE,
                offset,
                Beat { bar: at.bar, beat: at.beat, sixteenth: at.sixteenth },
            );
        }
    }
}
```

- [ ] **Step 4: Register it and extend the enum-defaults test**

In `crates/sway-nodes/src/midi.rs`'s `SignalNodesPlugin::build`, after the two registrations from Task 6:

```rust
        register_node_type::<crate::BeatTrigger>(app);
```

In `crates/sway-nodes/src/lib.rs`, extend `enum_defaults_are_the_first_variants`:

```rust
        assert_eq!(Division::default(), Division::Beat);
```

`Division::Beat` is deliberately the default and is deliberately *not* the first variant listed — `#[default]` is on it explicitly. If that assertion reads oddly, change the assertion, not the default: firing once per beat is what an author expects from a node called `BeatTrigger`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes`
Expected: PASS, including the 8 new `beat` tests. Then `cargo test --workspace` — PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): BeatTrigger, with sub-tick boundary offsets"
```

---

### Task 8: Golden traces for the transport

M3's exit criterion is "visuals stay locked through recorded traces containing tempo changes and clock dropouts", and that sentence names a golden trace, not a unit test. The existing harness already replays a MIDI trace at a fixed tick rate and asserts bit-identical output (parent §4); this task teaches it to synthesise a 24 ppqn pulse train, and adds the four cases that matter.

The pulse train is generated rather than hand-written: 48 pulses per second at 120 BPM is 480 lines of RON for a ten-second trace, and a generated one can carry deterministic jitter. Jitter comes from a fixed-seed LCG, never from `rand` — a golden trace that is not reproducible is not a golden trace.

**Files:**
- Modify: `crates/sway-nodes/tests/traces.rs`
- Create: `crates/sway-nodes/tests/traces/transport-lock.in.ron`
- Create: `crates/sway-nodes/tests/traces/transport-tempo-change.in.ron`
- Create: `crates/sway-nodes/tests/traces/transport-dropout.in.ron`
- Create: `crates/sway-nodes/tests/traces/beat-trigger.in.ron`
- Create (by blessing): the four matching `.out.ron` files

**Interfaces:**
- Consumes: everything from Tasks 1–7.
- Produces: nothing other code reads.

- [ ] **Step 1: Extend the trace input format**

In `crates/sway-nodes/tests/traces.rs`, add a clock section to `TraceInput` and the generator beside it:

```rust
#[derive(Debug, Deserialize)]
struct TraceInput {
    tick_hz: f64,
    ticks: u32,
    #[serde(default)]
    events: Vec<(f64, MidiEvent)>,
    /// A synthesised 24 ppqn clock train. Generated rather than written out:
    /// ten seconds at 120 BPM is 480 pulses, and a generator can carry
    /// deterministic jitter where a hand-written list cannot.
    #[serde(default)]
    clock: Option<ClockSpec>,
}

#[derive(Debug, Deserialize)]
struct ClockSpec {
    /// When the first pulse arrives.
    start: f64,
    /// `(bpm, beats)` segments, played back to back.
    segments: Vec<(f64, f64)>,
    /// Peak arrival jitter, in seconds. Deterministic — see `Lcg`.
    #[serde(default)]
    jitter: f64,
    /// `(from, to)` in seconds: pulses inside this window are dropped.
    #[serde(default)]
    dropout: Option<(f64, f64)>,
}

/// A deterministic jitter source. `rand` is not an option: a golden trace
/// that is not reproducible is not a golden trace.
struct Lcg(u64);

impl Lcg {
    fn next_signed(&mut self, magnitude: f64) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let unit = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        (unit * 2.0 - 1.0) * magnitude
    }
}

/// Expands a `ClockSpec` into timestamped clock messages.
fn clock_events(spec: &ClockSpec) -> Vec<(f64, RawMidi)> {
    let mut lcg = Lcg(0xC10C_C10C);
    let mut out = Vec::new();
    let mut t = spec.start;
    for &(bpm, beats) in &spec.segments {
        let secs_per_pulse = (60.0 / bpm) / 24.0;
        for _ in 0..((beats * 24.0).round() as usize) {
            let dropped = spec
                .dropout
                .is_some_and(|(from, to)| t >= from && t < to);
            if !dropped {
                out.push((
                    t + lcg.next_signed(spec.jitter),
                    RawMidi { status: sway_midi::CLOCK, data1: 0, data2: 0 },
                ));
            }
            t += secs_per_pulse;
        }
    }
    // Jitter can reorder two adjacent pulses; the inbox is drained in
    // arrival order, so sort here rather than relying on the generator.
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}
```

Add `sway-midi.workspace = true` to `crates/sway-nodes/Cargo.toml` — already done in Task 5 — and in `run_trace`, after the existing `for (time, message) in input.events` loop:

```rust
    if let Some(spec) = &input.clock {
        for (time, message) in clock_events(spec) {
            app.world_mut().resource_mut::<MidiInbox>().push(time, message);
        }
    }
```

- [ ] **Step 2: Teach the harness to snapshot beat events**

Add a variant to `PortKindSpec` and a matching arm in `snapshot_port`:

```rust
    /// A `BeatTrigger`'s pulse outlet.
    BeatEvents(u16),
```

```rust
        PortKindSpec::BeatEvents(ordinal) => {
            let slot = plan.base + plan.field_offsets[ordinal as usize];
            let mut events: Vec<(f32, String)> = arena.values[slot]
                .try_downcast_ref::<Events<Beat>>()
                .expect("traced beat port is Events<Beat>")
                .occurrences
                .iter()
                .map(|occurrence| {
                    (
                        occurrence.offset,
                        format!(
                            "beat({},{},{})",
                            occurrence.value.bar, occurrence.value.beat, occurrence.value.sixteenth
                        ),
                    )
                })
                .collect();
            events.sort_by(|a, b| a.0.total_cmp(&b.0));
            Snapshot::Events(events)
        }
```

- [ ] **Step 3: Write the four graph builders**

Add to `crates/sway-nodes/tests/traces.rs`:

```rust
fn spawn_transport_time(app: &mut App, id: u32) -> Entity {
    let node_type = node_type_id::<TransportTimeNode>(app);
    app.world_mut()
        .spawn((
            GraphNode { id: NodeId(id), node_type },
            TransportTimeInlets::default(),
            TransportTimeState,
        ))
        .id()
}

/// Every transport trace watches the same three ports: what the estimator
/// thinks the tempo is, where the playhead is, and whether it is running.
fn transport_ports(node: Entity) -> Vec<TracedPort> {
    vec![
        TracedPort {
            label: "transport.bpm",
            node,
            kind: PortKindSpec::Continuous(TransportTimeNode::OUT_BPM),
        },
        TracedPort {
            label: "transport.beats",
            node,
            kind: PortKindSpec::Continuous(TransportTimeNode::OUT_BEATS),
        },
        TracedPort {
            label: "transport.playing",
            node,
            kind: PortKindSpec::Continuous(TransportTimeNode::OUT_PLAYING),
        },
    ]
}

fn build_transport_readout(app: &mut App) -> Vec<TracedPort> {
    let node = spawn_transport_time(app, 0);
    compile_graph(app);
    transport_ports(node)
}

fn build_beat_trigger(app: &mut App) -> Vec<TracedPort> {
    let time = spawn_transport_time(app, 0);
    let trigger_type = node_type_id::<BeatTrigger>(app);
    let trigger = app
        .world_mut()
        .spawn((
            GraphNode { id: NodeId(1), node_type: trigger_type },
            BeatTriggerInlets { division: Division::Beat },
            BeatTriggerState,
        ))
        .id();
    compile_graph(app);
    let mut ports = transport_ports(time);
    ports.push(TracedPort {
        label: "beat.pulse",
        node: trigger,
        kind: PortKindSpec::BeatEvents(BeatTrigger::OUT_PULSE),
    });
    ports
}
```

Extend `run_trace`'s dispatch match:

```rust
        "transport-lock" | "transport-tempo-change" | "transport-dropout" => {
            build_transport_readout(&mut app)
        }
        "beat-trigger" => build_beat_trigger(&mut app),
```

And extend the imports at the top of the file with `Beat, BeatTrigger, BeatTriggerInlets, BeatTriggerState, Division, TransportTimeInlets, TransportTimeNode, TransportTimeState`.

- [ ] **Step 4: Write the four inputs**

`crates/sway-nodes/tests/traces/transport-lock.in.ron` — four seconds of steady 120 BPM with realistic jitter, after a Start:

```ron
(
    tick_hz: 120.0,
    ticks: 480,
    events: [
        (0.0, (status: 250, data1: 0, data2: 0)),
    ],
    clock: Some((
        start: 0.0,
        segments: [(120.0, 8.0)],
        jitter: 0.0008,
    )),
)
```

`transport-tempo-change.in.ron` — 120 to 90 BPM mid-trace:

```ron
(
    tick_hz: 120.0,
    ticks: 720,
    events: [
        (0.0, (status: 250, data1: 0, data2: 0)),
    ],
    clock: Some((
        start: 0.0,
        segments: [(120.0, 6.0), (90.0, 6.0)],
        jitter: 0.0008,
    )),
)
```

`transport-dropout.in.ron` — one second of clock lost from t=2:

```ron
(
    tick_hz: 120.0,
    ticks: 720,
    events: [
        (0.0, (status: 250, data1: 0, data2: 0)),
    ],
    clock: Some((
        start: 0.0,
        segments: [(120.0, 12.0)],
        jitter: 0.0008,
        dropout: Some((2.0, 3.0)),
    )),
)
```

`beat-trigger.in.ron`:

```ron
(
    tick_hz: 120.0,
    ticks: 480,
    events: [
        (0.0, (status: 250, data1: 0, data2: 0)),
    ],
    clock: Some((
        start: 0.0,
        segments: [(120.0, 8.0)],
        jitter: 0.0008,
    )),
)
```

`250` is `0xFA`, Start. Keep the decimal — the existing trace files use decimal status bytes and mixing bases in one format is worse than one awkward number.

- [ ] **Step 5: Add the test functions**

```rust
#[test]
fn transport_lock() {
    let actual = run_trace("transport-lock");
    assert_or_bless("transport-lock", &actual);
}

#[test]
fn transport_tempo_change() {
    let actual = run_trace("transport-tempo-change");
    assert_or_bless("transport-tempo-change", &actual);
}

#[test]
fn transport_dropout() {
    let actual = run_trace("transport-dropout");
    assert_or_bless("transport-dropout", &actual);
}

#[test]
fn beat_trigger() {
    let actual = run_trace("beat-trigger");
    assert_or_bless("beat-trigger", &actual);
}

#[test]
fn a_transport_trace_replays_bit_identically() {
    // The exactness claim of parent §2.6, now covering the clock path: the
    // estimator, the offset tracker and the boundary search are all pure
    // functions of the tick sequence.
    let a = run_trace("transport-dropout");
    let b = run_trace("transport-dropout");
    assert_eq!(a, b);
}
```

- [ ] **Step 6: Bless and then *read* the outputs**

Run: `SWAY_BLESS=1 cargo test -p sway-nodes --test traces`
Then: `cargo test -p sway-nodes --test traces` — PASS.

Blessing is not verification. Open each `.out.ron` and check, by eye, that:

- `transport-lock` settles to `bpm` within ±0.5 of 120 by roughly tick 60 (one window is 48 pulses, which is one second at 120 BPM), and `beats` reaches ≈8.0 by tick 480.
- `transport-tempo-change` shows `bpm` moving from 120 toward 90 and settling within ±1 by roughly one window past the change, and `beats` never decreasing.
- `transport-dropout` shows `beats` continuing to climb at ≈2 beats/second right through the gap — **this is the whole milestone**, and a plateau there means freewheeling is broken.
- `beat-trigger` fires exactly once per beat with a non-zero offset at least sometimes, and its `bar`/`beat` counts increase monotonically.

If any of these is wrong, the bug is in Tasks 2, 5 or 7 — fix it there and re-bless. Do not adjust a trace to match a wrong output.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-nodes
git commit -m "test(nodes): golden traces for clock lock, tempo change and dropout"
```

---

### Task 9: The transport readout

M2c deliberately left the readout out — "inventing the display before the thing it displays is backwards" — and named M3 as the milestone that adds it, as a fourth consumer of the same per-frame snapshot. This is that.

A status strip across the top: state, BPM, bar.beat.sixteenth. It reads `Time<Transport>` directly, which is only possible because Task 4 put that type in `sway-graph` — `sway-editor` may not depend on `sway-nodes` (Global Constraints).

**Files:**
- Create: `crates/sway-editor/src/transport_bar.rs`
- Modify: `crates/sway-editor/src/snapshot.rs` (`WorldSnapshot`, `capture`)
- Modify: `crates/sway-editor/src/lib.rs` (`graph_root`, `apply_snapshot`, the tag)
- Modify: `crates/sway-editor/Cargo.toml` (move `bevy_time` into `[dependencies]`)

**Interfaces:**
- Consumes: `sway_graph::{Transport, TransportState, TransportTime}` (Task 4).
- Produces: `TransportView { playing, bpm, position, locked }`, `WorldSnapshot::transport`, `TransportBar`, `TRANSPORT_BAR_TAG`, `TRANSPORT_BAR_HEIGHT`.

- [ ] **Step 1: Write the failing snapshot tests**

Add to the `tests` module of `crates/sway-editor/src/snapshot.rs`:

```rust
    #[test]
    fn the_snapshot_carries_the_transport_readout() {
        use bevy_time::Time;
        use sway_graph::{Transport, TransportTime};

        let mut app = app();
        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.transport_mut().state = sway_graph::TransportState::Playing;
            time.transport_mut().secs_per_beat = 60.0 / 128.0;
            time.transport_mut().locked = true;
            time.advance_by(core::time::Duration::from_secs_f64(17.5));
            time.reposition(17.5);
        }

        let snap = capture(app.world());

        assert!(snap.transport.playing);
        assert!(snap.transport.locked);
        assert!((snap.transport.bpm - 128.0).abs() < 0.01);
        assert_eq!(snap.transport.position, "005.2.3");
    }

    #[test]
    fn a_world_with_no_transport_clock_still_captures() {
        // `capture` degrades rather than panicking (design §2.11), and a
        // world built before `GraphPlugin` ran is exactly that case.
        let world = bevy_ecs::world::World::new();
        let snap = capture(&world);
        assert!(!snap.transport.playing);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-editor snapshot`
Expected: compile error — `WorldSnapshot` has no `transport`.

- [ ] **Step 3: Extend the snapshot**

In `crates/sway-editor/Cargo.toml`, move `bevy_time.workspace = true` from `[dev-dependencies]` into `[dependencies]`.

In `crates/sway-editor/src/snapshot.rs`, add the type, the field and the capture:

```rust
/// The transport, as the status strip needs it. Strings and plain numbers,
/// not the clock itself: the widget layer should not have to know what a
/// `MusicalTime` is.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TransportView {
    pub playing: bool,
    pub bpm: f32,
    /// `bar.beat.sixteenth`, already formatted.
    pub position: String,
    /// Whether the phase estimator has a lock. Freewheeling still plays.
    pub locked: bool,
}

fn capture_transport(world: &World) -> TransportView {
    let Some(time) = world.get_resource::<bevy_time::Time<sway_graph::Transport>>() else {
        return TransportView::default();
    };
    TransportView {
        playing: time.is_playing(),
        bpm: time.bpm() as f32,
        position: time.position().to_string(),
        locked: time.transport().locked,
    }
}
```

Add `pub transport: TransportView,` to `WorldSnapshot`, `use sway_graph::TransportTime;` to the imports, and `transport: capture_transport(world),` to `capture`'s constructor.

Every `WorldSnapshot { ... }` literal in the workspace now misses a field. `WorldSnapshot` derives `Default`, so fix each by adding `..Default::default()` rather than spelling out an empty transport — `crates/sway-editor/src/lib.rs`'s `one_node_snapshot`, `crates/sway-editor/src/scene_tree.rs`'s `tree`, and any others the compiler names.

- [ ] **Step 4: Run to verify the snapshot tests pass**

Run: `cargo test -p sway-editor snapshot`
Expected: PASS.

- [ ] **Step 5: Write the failing widget tests**

Create `crates/sway-editor/src/transport_bar.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{TransportView, WorldSnapshot};
    use masonry::core::DefaultProperties;
    use masonry_testing::TestHarness;

    fn snapshot(playing: bool, bpm: f32, position: &str, locked: bool) -> WorldSnapshot {
        WorldSnapshot {
            transport: TransportView {
                playing,
                bpm,
                position: position.to_string(),
                locked,
            },
            ..Default::default()
        }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<TransportBar> {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), TransportBar::new().prepare());
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_snapshot(&mut bar, &snap);
        });
        harness
    }

    #[test]
    fn a_playing_transport_reads_out_state_tempo_and_position() {
        let harness = harness_with(snapshot(true, 128.02, "005.3.2", true));
        assert_eq!(
            harness.root_widget().fields(),
            vec!["PLAY".to_string(), "128.0 BPM".to_string(), "005.3.2".to_string()]
        );
    }

    #[test]
    fn a_stopped_transport_says_so() {
        let harness = harness_with(snapshot(false, 120.0, "001.1.1", false));
        assert_eq!(harness.root_widget().fields()[0], "STOP");
    }

    #[test]
    fn freewheeling_is_distinguishable_from_locked() {
        // A performer needs to know the clock is gone before they wonder why
        // the visuals are sliding.
        let locked = harness_with(snapshot(true, 120.0, "001.1.1", true));
        let free = harness_with(snapshot(true, 120.0, "001.1.1", false));
        assert_ne!(locked.root_widget().fields()[1], free.root_widget().fields()[1]);
    }

    #[test]
    fn an_unchanged_snapshot_rebuilds_nothing() {
        let snap = snapshot(true, 120.0, "001.1.1", true);
        let mut harness = harness_with(snap.clone());
        let before = harness.root_widget().generation();
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_snapshot(&mut bar, &snap);
        });
        assert_eq!(harness.root_widget().generation(), before);
    }
}
```

- [ ] **Step 6: Run to verify they fail**

Run: `cargo test -p sway-editor transport_bar`
Expected: compile error — `TransportBar` does not exist.

- [ ] **Step 7: Write the widget**

Above the test module in `crates/sway-editor/src/transport_bar.rs`:

```rust
//! `TransportBar` — the transport readout strip.
//!
//! M2c deliberately shipped no transport display, because inventing one
//! before the thing it displays is backwards. This is M3's fourth consumer of
//! the same per-frame `capture(&World)` snapshot, alongside the scene tree,
//! the viewport and the graph canvas.
//!
//! Children are `Label`s rather than painted text, for the reason `SceneTree`
//! gives: `imaging::Painter` takes only pre-shaped glyphs. Rows are rebuilt
//! only when the text actually changes, so a steady transport costs one
//! comparison per frame — and at 120 BPM the position field changes several
//! times a second, so that comparison is what stops this widget rebuilding
//! the world.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PropertiesRef, RegisterCtx, Widget,
    WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

use crate::snapshot::WorldSnapshot;

/// Height of the strip, in logical pixels.
pub const TRANSPORT_BAR_HEIGHT: f64 = 24.0;
/// Left padding and the gap between fields.
const PADDING: f64 = 12.0;
/// Fixed column width per field, so the position does not jitter the layout
/// four times a beat.
const FIELD_WIDTH: f64 = 120.0;

/// The transport readout.
pub struct TransportBar {
    labels: Vec<WidgetPod<Label>>,
    fields: Vec<String>,
    generation: u64,
    playing: bool,
}

impl Default for TransportBar {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportBar {
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            fields: Vec::new(),
            generation: 0,
            playing: false,
        }
    }

    /// The three field strings, in display order. Exposed for tests.
    pub fn fields(&self) -> Vec<String> {
        self.fields.clone()
    }

    /// How many times the fields have actually been rebuilt.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// The three strings a snapshot displays as.
///
/// A freewheeling transport says so in the tempo field rather than in a
/// fourth one: a performer needs to know the clock is gone *before* they
/// wonder why the visuals are sliding, and a `~` prefix reads at a glance.
fn fields_of(snap: &WorldSnapshot) -> Vec<String> {
    let transport = &snap.transport;
    vec![
        if transport.playing { "PLAY" } else { "STOP" }.to_string(),
        if transport.locked {
            format!("{:.1} BPM", transport.bpm)
        } else {
            format!("~{:.1} BPM", transport.bpm)
        },
        transport.position.clone(),
    ]
}

// --- MARK: WIDGETMUT
impl TransportBar {
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let fields = fields_of(snap);
        this.widget.playing = snap.transport.playing;
        if fields == this.widget.fields {
            return;
        }

        for label in this.widget.labels.drain(..) {
            this.ctx.remove_child(label);
        }
        for field in &fields {
            this.widget
                .labels
                .push(Label::new(field.clone()).prepare().to_pod());
        }

        this.widget.fields = fields;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }
}

impl Widget for TransportBar {
    type Action = ();

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for label in &mut self.labels {
            ctx.register_child(label);
        }
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        match (axis, len_req) {
            (_, LenReq::FitContent(space)) => space,
            (_, LenReq::MinContent) => Length::ZERO,
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(TRANSPORT_BAR_HEIGHT),
            (Axis::Horizontal, LenReq::MaxContent) => {
                Length::const_px(PADDING + self.labels.len() as f64 * FIELD_WIDTH)
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, label) in self.labels.iter_mut().enumerate() {
            let x = PADDING + index as f64 * FIELD_WIDTH;
            ctx.run_layout(label, Size::new(FIELD_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(label, Point::new(x, 0.0));
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        painter.fill_rect(
            Rect::new(0.0, 0.0, 4000.0, TRANSPORT_BAR_HEIGHT),
            Color::from_rgb8(30, 32, 38),
        );
        // A one-pixel accent under the state field, green while playing. The
        // strip has to be readable from across a room during a soundcheck.
        painter.fill_rect(
            Rect::new(0.0, TRANSPORT_BAR_HEIGHT - 2.0, PADDING, TRANSPORT_BAR_HEIGHT),
            if self.playing {
                Color::from_rgb8(90, 200, 120)
            } else {
                Color::from_rgb8(90, 92, 100)
            },
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::Label
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        self.labels.iter().map(|label| label.id()).collect()
    }
}
```

- [ ] **Step 8: Put the strip above the three panes**

In `crates/sway-editor/src/lib.rs`:

```rust
pub mod transport_bar;
```

```rust
use crate::transport_bar::{TRANSPORT_BAR_HEIGHT, TransportBar};

/// Reaches the transport readout from `EditorUi::apply_snapshot`.
pub const TRANSPORT_BAR_TAG: WidgetTag<TransportBar> = WidgetTag::named("sway-transport-bar");
```

Update `graph_root`'s doc diagram and its final expression — the existing `Split::new(tree, right)` becomes the *lower* pane of a new outer split:

```rust
    let panes = Split::new(tree, right)
        .split_axis(Axis::Horizontal)
        .split_point_from_start(260.0.px())
        .draggable(true)
        .solid_bar(true)
        .prepare();

    let bar = TransportBar::new().prepare().with_tag(TRANSPORT_BAR_TAG);

    Split::new(bar, panes)
        .split_axis(Axis::Vertical)
        .split_point_from_start(TRANSPORT_BAR_HEIGHT.px())
        .draggable(false)
        .solid_bar(true)
        .prepare()
        .erased()
```

And in `apply_snapshot`, alongside the other two:

```rust
        self.root.edit_widget_with_tag(TRANSPORT_BAR_TAG, |mut bar| {
            TransportBar::apply_snapshot(&mut bar, snap);
        });
```

- [ ] **Step 9: Extend the layout regression test**

`viewport_rect_reflects_its_position_inside_nested_splits` currently asserts the viewport sits right of the tree pane. Add the vertical half, which the new strip is exactly the kind of change that could break:

```rust
        assert!(
            rect.y0 >= crate::transport_bar::TRANSPORT_BAR_HEIGHT,
            "viewport rect {rect:?} must sit below the transport strip"
        );
```

- [ ] **Step 10: Run the crate's suite**

Run: `cargo test -p sway-editor`
Expected: PASS. Then `cargo test --workspace` — PASS.

- [ ] **Step 11: Commit**

```bash
git add crates/sway-editor
git commit -m "feat(editor): transport readout strip above the three panes"
```

---

### Task 10: A beat-locked demo graph

The demo graph currently rotates its group from a wall-clock `LFO` at 0.1 Hz. Beat-locking it is what makes the milestone visible: with a sequencer running, the spin lands on the bar, and pulling the MIDI cable makes it freewheel rather than stop.

`BeatTrigger` is **not** wired in, and that is deliberate: nothing consumes `Events<Beat>` yet (`Envelope` takes `Events<NoteMsg>`), and inventing an adapter to make a demo look complete would be the type-selector smell in event clothing. It is covered by Task 7's tests and Task 8's trace; Task 12 records the gap.

**Files:**
- Modify: `crates/sway-app/src/demo_graph.rs`

**Interfaces:**
- Consumes: `sway_nodes::{SyncLfo, SyncLfoInlets, SyncLfoState, TransportTimeNode, TransportTimeInlets, TransportTimeState}` (Task 6).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/sway-app/src/demo_graph.rs`:

```rust
    #[test]
    fn the_demo_graph_is_beat_locked() {
        use bevy::time::Time;
        use sway_graph::{Transport, TransportState, TransportTime};
        use sway_nodes::SyncLfoState;

        let mut app = app();
        setup_demo_graph(app.world_mut());

        // A stopped transport leaves the rotation where it is; a playing one
        // moves it. That is the property "beat-locked" actually means.
        app.update();
        let stopped = app
            .world_mut()
            .query_filtered::<&Transform, With<GroupState>>()
            .iter(app.world())
            .next()
            .copied()
            .expect("the root group has a Transform");

        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.transport_mut().state = TransportState::Playing;
            time.advance_by(core::time::Duration::from_secs_f64(1.0));
        }
        app.update();

        let playing = app
            .world_mut()
            .query_filtered::<&Transform, With<GroupState>>()
            .iter(app.world())
            .next()
            .copied()
            .expect("the root group has a Transform");

        assert_ne!(
            stopped.rotation, playing.rotation,
            "a beat of transport must turn the group"
        );
        assert_eq!(
            app.world_mut().query::<&SyncLfoState>().iter(app.world()).count(),
            1,
            "the demo drives rotation from the tempo-synced LFO"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p sway-app demo_graph`
Expected: FAIL — no `SyncLfoState` in the world.

- [ ] **Step 3: Rewire the demo graph**

In `crates/sway-app/src/demo_graph.rs`, replace the `lfo` spawn with a `SyncLfo` over one bar, add a `TransportTimeNode` so the graph canvas shows beat time, and update the module's diagram comment:

```rust
    let sync = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<SyncLfo>(world) },
            SyncLfoInlets {
                // One full turn per four-bar phrase — slow enough to read as
                // locked rather than as a spin.
                beats: 16.0,
                shape: sway_nodes::Waveform::Saw,
                phase: 0.0,
                amplitude: core::f32::consts::PI,
            },
            SyncLfoState,
            EditorPos(Vec2::new(20.0, 260.0)),
        ))
        .id();
    let transport = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<TransportTimeNode>(world) },
            TransportTimeInlets::default(),
            TransportTimeState,
            EditorPos(Vec2::new(240.0, 260.0)),
        ))
        .id();
```

Replace the rotation edge with:

```rust
    edge(world, sync, SyncLfo::OUT_VALUE, root, Group::ROTATION_Y, 0);
```

And, so the time base is not a dead node in the picture, drive the material's colour from the bar:

```rust
    edge(world, transport, TransportTimeNode::OUT_BAR_PHASE, rgb, Rgb::G, 0);
```

Update the imports at the top of the file and the doc-comment diagram to match. Delete the now-unused `LFO`/`LfoInlets`/`LfoState` imports — `LFO` remains a registered node type, it is simply no longer in this graph.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p sway-app`
Expected: PASS. Then `cargo test --workspace` — PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-app
git commit -m "feat(app): beat-lock the demo graph to the transport"
```

---

### Task 11: Verify on hardware

M1's finding stands and is not optional: two of that milestone's bugs were invisible to every test and only a GPU exposed them. The clock path has the same shape — a real sequencer's jitter, a real cable, a real dropout — and the traces in Task 8 are synthetic by construction.

**Files:** none.

- [ ] **Step 1: Run the editor against a real clock source**

```bash
cargo run -p sway-app -- --editor --midi ""
```

Set a sequencer (the Octatrack, or Ableton with **MIDI To → Sway** and *Sync* enabled on that output) to send clock, and press play.

Expected, in order:
1. The strip reads `STOP` before play, and the BPM field shows `~120.0` with the tilde — nothing is locked yet.
2. On play it flips to `PLAY`, the tilde disappears within about a second, and the BPM settles within ±0.5 of the sequencer's.
3. The bar.beat.sixteenth counter advances in step with the sequencer's own display, and the two do not drift apart over several minutes.
4. The grid's rotation completes exactly one turn every four bars.

- [ ] **Step 2: Change tempo while it runs**

Move the sequencer's tempo by 20 BPM. Expected: the readout follows within about a second, the position counter does not jump or stall, and the rotation stays phase-continuous — it changes speed, it does not snap.

- [ ] **Step 3: Pull the clock**

Stop the sequencer's clock output (or unplug the cable) for several seconds without stopping the transport. Expected: the tilde returns, the position keeps advancing at the last tempo, and the rotation keeps turning. Reconnect: the tilde clears and the position does not jump backwards.

- [ ] **Step 4: Check the long-session drift the epoch bridge was about**

Leave it running against a steady clock for at least ten minutes with the viewport visible. Expected: notes still land on time and the readout still tracks the sequencer. If MIDI response degrades over the session, the offset tracker is wrong and the bug belongs to Task 3 — a fixed epoch's symptom is exactly this, and it is why M2a's version is being replaced.

- [ ] **Step 5: Record what happened**

Write down the observed BPM figures, the settling times and anything surprising; Task 12 needs numbers, not impressions. If no hardware is available, say so plainly in the findings report rather than implying this was confirmed.

---

### Task 12: Close out

**Files:**
- Create: `docs/superpowers/reports/2026-08-04-m3-transport-findings.md`
- Modify: `docs/superpowers/specs/2026-07-25-sway-design.md` (§5 status, the M3 entry, §7)

- [ ] **Step 1: Run the whole suite and the clippy gate**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo clippy -p sway-midi -p sway-graph -p sway-nodes -p sway-editor -p sway-app --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: Write the findings report**

`docs/superpowers/reports/2026-08-04-m3-transport-findings.md`. Answer these with evidence rather than impressions:

1. **Did windowed regression hold?** State the measured settling time after a tempo change, the residual BPM error under jitter, and whether the 48-pulse window was the right length. If a PLL would have been better, say why — the alternative was considered and rejected, and a reversal should be recorded as one.
2. **Did the freewheel policy hold?** How far did beat position drift over the longest dropout tested, and did re-locking ever produce a visible jump? The generation-guard exists to prevent one; say whether it fired.
3. **Was the min-filtered offset the right shape?** Whether long-session MIDI response actually held up (Task 11, Step 4), and whether `OFFSET_WINDOW` at 240 drains was too short, too long, or irrelevant.
4. **What did the zero-inlet node break?** `TransportTimeNode` is the first node type with an empty `Inlets` struct. Record whatever `derive_fields`, `prefill_of` or `compile` had to be taught, because M4's RON schema will meet the same shape.
5. **`Events<Beat>` has no consumer.** State the question this leaves open: whether event payloads want a common shape, whether `Envelope` should take any event type, or whether a `Quantize` node that delays an existing stream onto beat boundaries is the missing piece. This is the one design question M3 opened and did not answer.
6. **The mid-tick reposition quantization.** A Start landing mid-tick puts position zero at the tick boundary, costing up to one timestep. Say whether that was ever observable against a sequencer's own display, and whether the second advance path it would take to fix is worth it.

Add a "what a later milestone would otherwise rediscover" section covering every API surprise hit along the way — `Time<T>`'s monotonicity constraints, whatever `Split` did with a fixed-height first pane, and anything `bevy_reflect` objected to in `Beat`, `Division` or `Transport`.

- [ ] **Step 3: Update the roadmap**

In `docs/superpowers/specs/2026-07-25-sway-design.md`:

- Line 4's status line: M3 complete, M4 next.
- §5's "Status at 2026-08-03" paragraph: add M3, and date it.
- The M3 entry: mark it **complete**, link the findings report, and add a *Carried forward* line naming whatever this milestone did not discharge — at minimum the `Events<Beat>` consumer gap and the mid-tick reposition quantization.
- §7: if the transport work moved any open question — the tick-rate value now has a second data point, and the estimator is a new thing that runs every tick — add the measurement or say plainly that none was taken.
- Add a `**Revision:**` line at the top if any of §2.7's claims turned out wrong. A design document that records what was believed beforehand and is never corrected afterwards is worse than none.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers
git commit -m "docs: M3 transport and beat lock findings"
```
