# MIDI and transport redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `Time<Transport>` integrator with a pulse-grid `PulseClock`, a Bevy `Transport` snapshot in `sway-midi`, a generic `Oscillator`, and a `MidiTime` float source.

**Architecture:** `sway-midi-core` (today's `sway-midi`) stays Bevy-free: CoreMIDI, `MidiMessage`, `PulseClock`. `sway-midi` is the Bevy plugin: inbox, tick slice, `MidiClock`, `Transport` resource, `MidiTime`. `Oscillator` in `sway-nodes` takes a wired `time` float and does not depend on MIDI. `FloatOut`/`Vec3Out` move to `sway-graph` so `MidiTime` can write an outlet without depending on `sway-nodes`.

**Tech Stack:** Rust 2024, Bevy `=0.19.0`, CoreMIDI (macOS), `crossbeam-channel`.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-08-16-midi-redesign-design.md`. That file wins if anything here disagrees.
- **Bevy is pinned at `=0.19.0`.** Do not bump it.
- **`sway-graph` must not depend on MIDI** (architecture §5). Task 4 is the only graph change that adds types, and they are outlets, not MIDI.
- **`sway-nodes` must not depend on `sway-midi` or `sway-midi-core` after Task 9.**
- **A wire must never write an equal value.** `set_if_neq` on every `FloatOut` write (`MidiTime`, `Oscillator`) and every new wire (`TimeFrom`).
- **Position is `pulse_index / 24` plus clamped interpolation.** No skip-count, no freewheel (D2, D3).
- **CoreMIDI virtual destination, unique ID `'SWAY'`, advance-schedule, and `--midi` filter are unchanged.**
- **Commit at the end of every task.** `cargo test --workspace` must be green (`RUST_TEST_THREADS=1` is already in `.cargo/config.toml`).
- **M9 is out of scope:** do not add `MidiNote`, `MidiCC`, `BeatTrigger` as authorable nodes.

**Reference documents:**
- Spec: `docs/superpowers/specs/2026-08-16-midi-redesign-design.md`
- Architecture: `docs/architecture.md`

## File structure

**`sway-midi-core`** (rename of `crates/sway-midi`):

| File | Responsibility |
|---|---|
| `src/message.rs` (new) | `MidiMessage`, `TimedMidi`, `from_bytes` |
| `src/clock.rs` (new) | `PulseClock` |
| `src/transport.rs` | Keep `ClockEstimator` as tempo-only until Task 9; delete `beats_at` in Task 9 |
| `src/input.rs` | `read_proc` sends `TimedMidi` |
| `src/parser.rs`, `src/ffi.rs` | Unchanged behaviour |

**`sway-midi`** (new crate):

| File | Responsibility |
|---|---|
| `src/lib.rs` | Plugin, re-exports of core IO |
| `src/plugin.rs` | `MidiPlugin`, feed/drain/clock systems, resources |
| `src/transport.rs` | `Transport`, `MusicalTime` |
| `src/midi_time.rs` | `MidiTime` component + write system |

**`sway-graph`:** `src/outlets.rs` (`FloatOut`, `Vec3Out`); delete `src/transport.rs` in Task 9.

**`sway-nodes`:** `Lfo` → `Oscillator` + `TimeFrom`; delete `src/midi.rs`, `src/transport.rs`, `src/outputs.rs` once unused.

---

### Task 1: `MidiMessage` and `TimedMidi`

Typed messages on the CoreMIDI channel. Zero-velocity note-on is `NoteOff`.

**Files:**
- Create: `crates/sway-midi/src/message.rs`
- Modify: `crates/sway-midi/src/lib.rs`, `crates/sway-midi/src/input.rs`
- Modify tests in `crates/sway-midi/src/lib.rs` that construct `MidiEvent`

**Interfaces:**
- Consumes: `StreamParser::push` → `(u8, u8, u8)`.
- Produces:
  ```rust
  pub enum MidiMessage {
      NoteOn { channel: u8, note: u8, velocity: u8 },
      NoteOff { channel: u8, note: u8, velocity: u8 },
      Control { channel: u8, cc: u8, value: u8 },
      Clock, Start, Continue, Stop,
      SongPosition { sixteenths: u16 },
      Other { status: u8, data1: u8, data2: u8 },
  }
  impl MidiMessage {
      pub fn from_bytes(status: u8, data1: u8, data2: u8) -> Self;
  }
  pub struct TimedMidi {
      pub host_time: u64,
      pub message: MidiMessage,
  }
  pub fn open_input(filter: &str, tx: Sender<TimedMidi>) -> Result<MidiInput, OSStatus>;
  ```

- [ ] **Step 1: Write the failing tests**

Add `crates/sway-midi/src/message.rs` with tests only (types commented out will not compile — write the tests in `message.rs` `mod tests` and empty stubs that fail assertions, or write tests first that call `MidiMessage::from_bytes`).

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_velocity_note_on_is_note_off() {
        assert_eq!(
            MidiMessage::from_bytes(0x90, 60, 0),
            MidiMessage::NoteOff { channel: 0, note: 60, velocity: 0 }
        );
    }

    #[test]
    fn note_on_carries_channel() {
        assert_eq!(
            MidiMessage::from_bytes(0x91, 64, 100),
            MidiMessage::NoteOn { channel: 1, note: 64, velocity: 100 }
        );
    }

    #[test]
    fn clock_start_stop_continue_and_spp() {
        assert_eq!(MidiMessage::from_bytes(0xF8, 0, 0), MidiMessage::Clock);
        assert_eq!(MidiMessage::from_bytes(0xFA, 0, 0), MidiMessage::Start);
        assert_eq!(MidiMessage::from_bytes(0xFB, 0, 0), MidiMessage::Continue);
        assert_eq!(MidiMessage::from_bytes(0xFC, 0, 0), MidiMessage::Stop);
        assert_eq!(
            MidiMessage::from_bytes(0xF2, 8, 0),
            MidiMessage::SongPosition { sixteenths: 8 }
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sway-midi from_bytes -- --nocapture`

Expected: compile error, `MidiMessage` not found.

- [ ] **Step 3: Implement `from_bytes` and switch the channel**

```rust
impl MidiMessage {
    pub fn from_bytes(status: u8, data1: u8, data2: u8) -> Self {
        match status {
            0xF8 => Self::Clock,
            0xFA => Self::Start,
            0xFB => Self::Continue,
            0xFC => Self::Stop,
            0xF2 => {
                let sixteenths = u16::from(data2) << 7 | u16::from(data1);
                Self::SongPosition { sixteenths }
            }
            s if s & 0xF0 == 0x90 => {
                let channel = s & 0x0F;
                if data2 == 0 {
                    Self::NoteOff { channel, note: data1, velocity: 0 }
                } else {
                    Self::NoteOn { channel, note: data1, velocity: data2 }
                }
            }
            s if s & 0xF0 == 0x80 => Self::NoteOff {
                channel: s & 0x0F,
                note: data1,
                velocity: data2,
            },
            s if s & 0xF0 == 0xB0 => Self::Control {
                channel: s & 0x0F,
                cc: data1,
                value: data2,
            },
            s => Self::Other { status: s, data1, data2 },
        }
    }
}
```

In `read_proc`, after `parser.push`, send `TimedMidi { host_time, message: MidiMessage::from_bytes(status, data1, data2) }`. Replace every `MidiEvent` / `Sender<MidiEvent>` with `TimedMidi`. Keep `MidiEvent` as a type alias **only if** a compile forces it; prefer deleting it in this task.

Update `crates/sway-app/src/midi_feed.rs` to match `TimedMidi` and map `MidiMessage` back into `RawMidi` for the old inbox (temporary): a `Clock` is `RawMidi { status: 0xF8, data1: 0, data2: 0 }`, etc. This shim dies in Task 9.

- [ ] **Step 4: Run tests**

Run: `cargo test -p sway-midi && cargo test --workspace`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-midi crates/sway-app/src/midi_feed.rs
git commit -m "$(cat <<'EOF'
feat(midi): typed MidiMessage on the CoreMIDI channel.

EOF
)"
```

---

### Task 2: `PulseClock`

Position is pulse-index / 24. Tempo is still the windowed least-squares slope. Hold on dropout. `ClockEstimator` stays public for the old `advance_transport` path.

**Files:**
- Create: `crates/sway-midi/src/clock.rs`
- Modify: `crates/sway-midi/src/lib.rs` (`mod clock; pub use clock::PulseClock;`)

**Interfaces:**
- Consumes: `MidiMessage`, `ClockEstimator::push_pulse` / `secs_per_pulse` / `bpm`.
- Produces:
  ```rust
  pub struct PulseClock { /* private */ }
  impl PulseClock {
      pub fn new() -> Self;
      pub fn push(&mut self, t: f64, message: MidiMessage);
      pub fn ppq(&self, t: f64) -> f64;
      pub fn bpm(&self) -> f64;          // estimator, else 120
      pub fn playing(&self) -> bool;
      pub fn locked(&self, t: f64) -> bool;
      pub fn beats_per_bar(&self) -> u32; // default 4
  }
  ```

- [ ] **Step 1: Write the failing tests** in `clock.rs`

Use `SPP_120 = 0.5 / 24.0`. Helper: `fn play(clock: &mut PulseClock, n: usize, spp: f64, start: f64) -> f64` that `push`es `Start` at `start` then `n` `Clock`s.

Required tests (names are the spec):

```rust
#[test]
fn twenty_four_clocks_advance_ppq_by_one_at_pulse_instants() {
    let mut c = PulseClock::new();
    c.push(0.0, MidiMessage::Start);
    let mut t = 0.0;
    for i in 1..=24 {
        t += SPP_120;
        c.push(t, MidiMessage::Clock);
        assert!((c.ppq(t) - i as f64 / 24.0).abs() < 1e-12);
    }
}

#[test]
fn interpolation_stays_inside_the_current_pulse() {
    let mut c = PulseClock::new();
    c.push(0.0, MidiMessage::Start);
    c.push(SPP_120, MidiMessage::Clock);
    let mid = SPP_120 * 1.5;
    let ppq = c.ppq(mid);
    assert!(ppq > 1.0 / 24.0);
    assert!(ppq < 2.0 / 24.0);
}

#[test]
fn one_second_of_silence_does_not_advance_ppq() {
    let mut c = PulseClock::new();
    c.push(0.0, MidiMessage::Start);
    let mut t = 0.0;
    for _ in 0..24 {
        t += SPP_120;
        c.push(t, MidiMessage::Clock);
    }
    let held = c.ppq(t);
    assert!((c.ppq(t + 1.0) - held).abs() < 1.0 / 24.0 + 1e-9);
    // Must not freewheel two beats.
    assert!(c.ppq(t + 1.0) < held + 0.05);
}

#[test]
fn start_zeros_ppq() { /* Start after some clocks → ppq(t) == 0 */ }

#[test]
fn spp_eight_sixteenths_is_two_beats() {
    let mut c = PulseClock::new();
    c.push(0.0, MidiMessage::SongPosition { sixteenths: 8 });
    assert!((c.ppq(0.0) - 2.0).abs() < 1e-12);
}

#[test]
fn stop_then_continue_resumes_the_same_ppq() { /* ... */ }

#[test]
fn clocks_while_stopped_update_bpm_only() { /* playing false, ppq frozen, bpm ~120 */ }

#[test]
fn duplicate_timestamps_do_not_increment_index() { /* two Clocks at same t; ppq unchanged */ }

#[test]
fn a_three_pulse_gap_does_not_skip_count() {
    // After lock, skip 3 * SPP_120, next Clock is +1/24 not +4/24.
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sway-midi --lib clock`

Expected: `PulseClock` not found.

- [ ] **Step 3: Implement `PulseClock`**

State: `estimator: ClockEstimator`, `pulse_index: u64`, `t_last: Option<f64>` (last Start/SPP/playing-Clock), `last_clock_t: Option<f64>` (any Clock, for dupes + `locked`), `playing: bool`, `frozen_ppq: f64`, `beats_per_bar: u32` default 4.

- Non-finite `t`: return.
- Duplicate: if `last_clock_t` is `Some(prev)` and `t <= prev` and message is `Clock`, return (do not increment). Apply the same `t <= last_clock_t` guard for any Clock.
- `Clock` while playing: `estimator.push_pulse(t); pulse_index += 1; t_last = Some(t); frozen_ppq = pulse_index as f64 / 24.0`.
- `Clock` while stopped: `estimator.push_pulse(t)` only.
- `Start`: `playing = true; pulse_index = 0; t_last = Some(t); frozen_ppq = 0.0`.
- `Continue`: `playing = true` (keep index / `t_last` / `frozen_ppq`).
- `Stop`: `playing = false; frozen_ppq = self.ppq(t)`.
- `SongPosition`: `pulse_index = sixteenths as u64 * 6; t_last = Some(t); frozen_ppq = sixteenths as f64 / 4.0`.

```rust
pub fn ppq(&self, t: f64) -> f64 {
    if !self.playing {
        return self.frozen_ppq;
    }
    let Some(t_last) = self.t_last else {
        return self.frozen_ppq;
    };
    let spp = self.estimator.secs_per_pulse().unwrap_or(0.5 / 24.0);
    let frac = if spp > 0.0 { (t - t_last) / spp } else { 0.0 };
    let frac = frac.clamp(0.0, 1.0 - f64::EPSILON);
    self.pulse_index as f64 / 24.0 + frac
}

pub fn locked(&self, t: f64) -> bool {
    let Some(last) = self.last_clock_t else { return false };
    let spp = self.estimator.secs_per_pulse().unwrap_or(0.5 / 24.0);
    t - last <= spp
}

pub fn bpm(&self) -> f64 {
    self.estimator.bpm().unwrap_or(120.0)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sway-midi --lib clock`

Expected: PASS. Then `cargo test --workspace`.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-midi/src/clock.rs crates/sway-midi/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(midi): PulseClock snaps ppq to the MIDI pulse grid.

EOF
)"
```

---

### Task 3: Rename to `sway-midi-core` and add the `sway-midi` plugin crate

**Files:**
- Rename: `crates/sway-midi` → `crates/sway-midi-core` (`git mv`)
- Create: `crates/sway-midi/Cargo.toml`, `crates/sway-midi/src/lib.rs`
- Modify: workspace `Cargo.toml` members + `[workspace.dependencies]`
- Modify: `crates/sway-nodes/Cargo.toml`, `crates/sway-app/Cargo.toml` (app depends on `sway-midi` only)

**Interfaces:**
- Consumes: existing `sway-midi` public API, now crate `sway-midi-core`.
- Produces: `sway-midi` re-exports `open_input`, `list_sources`, `list_destinations`, `VIRTUAL_DESTINATION_NAME`, `host_time_now`, `host_time_to_secs`, `TimedMidi`, `MidiMessage`, `PulseClock`.

- [ ] **Step 1: Rename and fix the workspace**

```bash
git mv crates/sway-midi crates/sway-midi-core
```

In `crates/sway-midi-core/Cargo.toml` set `name = "sway-midi-core"`.

Workspace `Cargo.toml`:

```toml
members = [..., "crates/sway-midi-core", "crates/sway-midi", ...]
sway-midi-core = { path = "crates/sway-midi-core" }
sway-midi = { path = "crates/sway-midi" }
```

`sway-nodes` depends on `sway-midi-core` (still needs `ClockEstimator` / `CLOCK` until Task 9). Replace `sway_midi::` with `sway_midi_core::` in `crates/sway-nodes`.

- [ ] **Step 2: Create the plugin crate as a re-export shim**

`crates/sway-midi/Cargo.toml`:

```toml
[package]
name = "sway-midi"
edition.workspace = true
version.workspace = true

[dependencies]
sway-midi-core.workspace = true
sway-graph.workspace = true
bevy.workspace = true
bevy_app.workspace = true
bevy_ecs.workspace = true
bevy_reflect.workspace = true
bevy_time.workspace = true
crossbeam-channel.workspace = true
```

`crates/sway-midi/src/lib.rs`:

```rust
pub use sway_midi_core::{
    TimedMidi, MidiInput, MidiMessage, PulseClock, VIRTUAL_DESTINATION_NAME,
    host_time_now, host_time_to_secs, list_destinations, list_sources, open_input,
};
```

`sway-app` keeps `sway-midi.workspace = true` and keeps calling `sway_midi::open_input`. Point `midi_feed.rs` at `sway_midi::TimedMidi`.

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor(midi): split sway-midi-core from the Bevy plugin crate.

EOF
)"
```

---

### Task 4: Move `FloatOut` and `Vec3Out` to `sway-graph`

**Files:**
- Create: `crates/sway-graph/src/outlets.rs`
- Modify: `crates/sway-graph/src/lib.rs`, `crates/sway-graph/src/test_wires.rs` (use `crate::FloatOut`, delete the duplicate struct)
- Modify: `crates/sway-nodes/src/outputs.rs` — delete file; re-export from graph in `lib.rs` if anything still wants `sway_nodes::FloatOut` this task (`pub use sway_graph::{FloatOut, Vec3Out};`)
- Modify every `use crate::outputs::FloatOut` in `sway-nodes` to `use sway_graph::FloatOut`

**Interfaces:**
- Consumes: nothing from MIDI.
- Produces: `sway_graph::FloatOut(pub f32)`, `sway_graph::Vec3Out(pub Vec3)` with the same `Component + Reflect + Default + PartialEq` derives as today.

- [ ] **Step 1: Write a compile/unit check in graph**

In `outlets.rs` tests:

```rust
#[test]
fn float_out_is_a_copy_partial_eq_component() {
    assert_eq!(FloatOut(1.0), FloatOut(1.0));
    assert_ne!(FloatOut(1.0), FloatOut(2.0));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-graph float_out_is_a_copy`

Expected: `FloatOut` not in `sway-graph` (or the test module not compiled).

- [ ] **Step 3: Move the types**

Copy the structs from `crates/sway-nodes/src/outputs.rs`. `Vec3Out` uses `bevy_math::Vec3` (already a graph dep). `pub use outlets::{FloatOut, Vec3Out};` from `lib.rs`. `test_wires.rs` deletes its own `FloatOut` and uses `crate::FloatOut`. Nodes re-export for one task if editor tests use `sway_nodes::FloatOut` (`snapshot.rs` does).

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph crates/sway-nodes
git commit -m "$(cat <<'EOF'
refactor(graph): move FloatOut and Vec3Out into sway-graph.

EOF
)"
```

---

### Task 5: `Oscillator` replaces `Lfo`

Generic oscillator. Time is a float field, default `0`. No `Time<Transport>`.

**Files:**
- Modify: `crates/sway-nodes/src/osc.rs`, `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/src/lfo.rs` (`lfo_value` can stay as a Hz helper; oscillator uses `sync_lfo_value` / `wave`)
- Modify: editor tests that spawn `Lfo` / assert field `"beats"` (`crates/sway-editor/src/snapshot.rs`, `palette.rs`)
- Modify: `crates/sway-app/assets/demo.sway.ron` **only the component name and fields** (`"Oscillator": (time: 0.0, period: 8.0, ...)`). Do **not** wire `MidiTime` yet (Task 7). Cubes will hold still until then — acceptable for this commit if `cargo test` document tests are updated to `Oscillator`. If `demo_document.rs` only checks `FloatOut` on the LFO entity, switch the type query to `Oscillator`.

**Interfaces:**
- Consumes: `sway_graph::FloatOut`, `field_wire!`.
- Produces:
  ```rust
  pub struct Oscillator {
      pub time: f32,
      pub period: f32,
      pub shape: Waveform,
      pub phase: f32,
      pub amplitude: f32,
  }
  // Default: time 0, period 4, Sine, phase 0, amplitude 1
  pub struct TimeFrom(pub Entity); // wire name "time"
  pub fn oscillator_behaviour(world: &mut World, entity: Entity, ctx: &TickCtx);
  ```

- [ ] **Step 1: Write the failing test** in `osc.rs`

```rust
#[test]
fn oscillator_at_phase_quarter_is_one_with_no_midi() {
    let mut app = slice_app(); // same helper as today, WiresPlugin + WireNodesPlugin, no MidiPlugin
    let e = app.world_mut().spawn(Oscillator {
        time: 0.0,
        period: 4.0,
        shape: Waveform::Sine,
        phase: 0.25,
        amplitude: 1.0,
    }).id();
    app.update();
    assert_eq!(app.world().get::<FloatOut>(e).map(|o| o.0), Some(1.0));
}
```

Rename the amplitude-wire test to use `Oscillator`. Keep the one-tick chain test: `Oscillator` A → `AmplitudeFrom` → B, B.time authored `0`, B.phase `0.25`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sway-nodes oscillator_at_phase_quarter`

Expected: `Oscillator` not found.

- [ ] **Step 3: Implement**

```rust
pub fn oscillator_behaviour(world: &mut World, entity: Entity, _ctx: &TickCtx) {
    let Some(osc) = world.get::<Oscillator>(entity).copied() else { return };
    let p = if osc.period > 0.0 {
        (osc.time as f64 / osc.period as f64 + osc.phase as f64).rem_euclid(1.0) as f32
    } else {
        osc.phase.rem_euclid(1.0)
    };
    let value = wave(osc.shape, p) * osc.amplitude;
    if let Some(mut out) = world.get_mut::<FloatOut>(entity) {
        out.set_if_neq(FloatOut(value));
    }
}
```

`field_wire!(TimeFrom / DrivesTime, FloatOut => Oscillator, "time", |t| &mut t.time, |s| s.0);`

Keep `AmplitudeFrom` targeting `Oscillator.amplitude`.

`WireNodesPlugin`: `register_behaviour::<Oscillator>`, `register_wire::<TimeFrom>`, `register_authorable::<Oscillator>(app, "Oscillator")`. Remove `Lfo`.

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`

Expected: PASS. Palette tests that list `"Lfo"` must expect `"Oscillator"`.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-nodes crates/sway-editor crates/sway-app
git commit -m "$(cat <<'EOF'
feat(nodes): Oscillator takes time as a wired float.

EOF
)"
```

---

### Task 6: Bevy plugin — inbox, drain, `MidiClock`, `Transport`

**Files:**
- Create: `crates/sway-midi/src/plugin.rs`, `crates/sway-midi/src/transport.rs`
- Modify: `crates/sway-midi/src/lib.rs`

**Interfaces:**
- Consumes: `PulseClock`, `TimedMidi`, `host_time_now`, `host_time_to_secs`, `sway_graph::graph_tick`.
- Produces:
  ```rust
  #[derive(Resource)]
  pub struct Transport {
      pub ppq: f64,
      pub bpm: f64,
      pub playing: bool,
      pub locked: bool,
      pub beats_per_bar: u32,
  }
  impl Default for Transport { /* stopped, 120 BPM, 4/4, ppq 0, unlocked */ }

  pub struct MusicalTime { pub bar: u32, pub beat: u32, pub sixteenth: u32, pub bar_phase: f32 }
  impl MusicalTime {
      pub fn from_ppq(ppq: f64, beats_per_bar: u32) -> Self; // copy of today's from_beats
  }

  #[derive(Resource)]
  pub struct MidiClock {
      pub clock: PulseClock,
      pub tick_start_host: f64, // seconds; NEG_INFINITY on first tick
  }
  #[derive(Resource)]
  pub struct MidiRx(pub Receiver<TimedMidi>);
  #[derive(Resource, Default)]
  pub struct MidiInbox { pub events: VecDeque<(f64, MidiMessage)> }
  #[derive(Resource, Default)]
  pub struct TickMidi { pub events: Vec<(f32, MidiMessage)> }

  pub struct MidiPlugin { pub rx: Receiver<TimedMidi> }
  ```

Copy `MusicalTime` + `Display` + tests from `crates/sway-graph/src/transport.rs`, renaming `from_beats` → `from_ppq`. Leave the graph copy in place until Task 9.

- [ ] **Step 1: Write failing plugin tests** in `plugin.rs`

```rust
#[test]
fn the_plugin_inserts_transport() {
    let (tx, rx) = crossbeam_channel::unbounded();
    drop(tx);
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(120.0))
        .add_plugins((WiresPlugin, MidiPlugin { rx }));
    assert!(app.world().get_resource::<Transport>().is_some());
    assert!(app.world().get_resource::<MidiClock>().is_some());
}

#[test]
fn feed_drains_every_event_into_the_inbox() { /* two TimedMidi; after update TickMidi or inbox has both in order */ }

#[test]
fn zero_host_time_maps_to_now() { /* host_time 0 */ }

#[test]
fn a_far_future_stamp_stays_in_the_inbox() {
    // host_time corresponding to now + 10s must not appear in TickMidi this tick.
}

#[test]
fn start_then_clocks_set_playing_and_ppq() {
    // send Start + 24 clocks stamped with increasing host times (use host_time_now
    // plus a synthetic path: push straight into MidiInbox in the test to avoid
    // mach conversion). Prefer inserting MidiInbox events in seconds and running
    // only drain+clock by making those systems pub(crate).
}
```

For deterministic clock tests, **do not** go through CoreMIDI. Insert into `MidiInbox` in seconds and call `drain_inbox` + `tick_clock` (pub(crate)).

```rust
#[test]
fn drain_then_clock_snaps_ppq() {
    let mut app = plugin_app(); // MidiPlugin + WiresPlugin + Fixed 120 Hz
    {
        let mut inbox = app.world_mut().resource_mut::<MidiInbox>();
        inbox.events.push_back((0.0, MidiMessage::Start));
        for i in 1..=24 {
            inbox.events.push_back((i as f64 * SPP_120, MidiMessage::Clock));
        }
    }
    // Force MidiClock.tick_start_host so drain sees 0..1s as due.
    app.world_mut().resource_mut::<MidiClock>().tick_start_host = 0.0;
    // One update may not cover 0.5s of pulses at 120 Hz; run ~60 updates
    // OR drain with a fake tick_end_host. Easier: unit-test drain/clock
    // functions with explicit tick_end_host arguments.
}
```

Make the systems take host times from `MidiClock` + `host_time_now()`. For tests, set `MidiClock.tick_start_host` and add a test-only `tick_end_override: Option<f64>` **or** pass seconds on the inbox and implement `drain(tick_start, tick_end)` as a pure function:

```rust
pub fn drain_window(
    inbox: &mut VecDeque<(f64, MidiMessage)>,
    tick_start: f64,
    tick_end: f64,
) -> Vec<(f32, MidiMessage)>;
```

```rust
pub fn apply_tick(clock: &mut PulseClock, events: &[(f32, MidiMessage)], tick_start: f64) {
    for &(offset, ref msg) in events {
        clock.push(tick_start + offset as f64, msg.clone());
    }
}
```

`MidiMessage` must be `Clone`. Derive it in Task 1 if not already.

Test `drain_window` and `apply_tick` + `PulseClock` without `host_time_now`. The Bevy systems are thin wrappers.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sway-midi`

Expected: `MidiPlugin` / `drain_window` not found.

- [ ] **Step 3: Implement**

`drain_window`: retain events with `t > tick_end`; for `t <= tick_end`, push `( (t - tick_start).clamp(0.0, dt) as f32, msg )` where `dt = tick_end - tick_start`. NaN-safe: use `max`/`min` not `clamp` if bounds can cross (copy the comment from today's `map_timestamp`).

Feed system: `now = host_time_to_secs(host_time_now())`; convert `host_time == 0` to `now`; otherwise `host_time_to_secs`. Push `(t, message)` onto `MidiInbox`.

Clock system: `tick_end = now`; `tick_start = midi_clock.tick_start_host`; if `tick_start` is `-inf`, `tick_start = tick_end` (empty first window, or treat all due). Drain; `apply_tick`; `*transport = Transport { ppq: clock.ppq(tick_end), bpm: clock.bpm(), playing: clock.playing(), locked: clock.locked(tick_end), beats_per_bar: clock.beats_per_bar() }`; `midi_clock.tick_start_host = tick_end`.

Schedule:

```rust
.add_systems(
    FixedUpdate,
    (feed_midi, drain_and_clock)
        .chain()
        .before(sway_graph::graph_tick),
)
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sway-midi && cargo test --workspace`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-midi
git commit -m "$(cat <<'EOF'
feat(midi): Bevy plugin publishes a Transport snapshot from PulseClock.

EOF
)"
```

---

### Task 7: `MidiTime` node

**Files:**
- Create: `crates/sway-midi/src/midi_time.rs`
- Modify: `crates/sway-midi/src/plugin.rs` (register + system)
- Modify: `crates/sway-app/assets/demo.sway.ron` — add `midiTime` entity; wire `"time": "midiTime"` on both oscillators
- Modify: `crates/sway-app/tests/demo_document.rs`
- Modify: `crates/sway-app/src/main.rs` — `add_plugins(sway_midi::MidiPlugin { rx })` **in addition to** the old `MidiPlugin` until Task 9, **or** switch now if `sway_nodes::MidiPlugin` can coexist (two inboxes). **Switch the app to `sway_midi::MidiPlugin` in this task** and keep `sway_nodes::MidiPlugin` only if traces still need it. Traces live in `sway-nodes` and still use the old plugin — leave them until Task 9. **App: replace** `sway_nodes::MidiPlugin` + `feed_midi` with `sway_midi::MidiPlugin { rx }`. Delete `midi_feed` from the app schedule; keep the file until Task 9 if tests still compile, otherwise move those tests into `sway-midi` (already done in Task 6) and delete `crates/sway-app/src/midi_feed.rs`.

**Interfaces:**
- Consumes: `Res<Transport>`, `sway_graph::{FloatOut, EditorPos}`.
- Produces:
  ```rust
  #[derive(Component, Reflect, Default)]
  #[require(FloatOut, EditorPos)]
  pub struct MidiTime;
  // system write_midi_time before graph_tick, after drain_and_clock
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn midi_time_writes_ppq_before_the_graph_tick() {
    let mut app = plugin_app();
    let e = app.world_mut().spawn(MidiTime).id();
    app.world_mut().resource_mut::<Transport>().ppq = 3.25;
    app.update();
    assert!((app.world().get::<FloatOut>(e).unwrap().0 - 3.25).abs() < 1e-5);
}
```

Also a one-tick chain in `sway-midi` tests: `MidiTime` → `TimeFrom` → `Oscillator` requires `WireNodesPlugin` and would make `sway-midi` depend on `sway-nodes`. **Do not.** Test `MidiTime` writes `FloatOut` only. Oscillator one-tick with a stub `FloatOut` source stays in `sway-nodes` (already in Task 5).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sway-midi midi_time_writes`

Expected: `MidiTime` not found.

- [ ] **Step 3: Implement**

```rust
pub fn write_midi_time(transport: Res<Transport>, mut q: Query<&mut FloatOut, With<MidiTime>>) {
    let value = FloatOut(transport.ppq as f32);
    for mut out in &mut q {
        out.set_if_neq(value);
    }
}
```

Register `MidiTime` with `register_authorable::<MidiTime>(app, "MidiTime")`. System in the same `FixedUpdate` chain, after `drain_and_clock`, before `graph_tick`.

Demo RON:

```
Entity(id: "midiTime", components: { "MidiTime": (), "EditorPos": ((-700.0, 100.0)) }),
Entity(id: "lfoA", components: { "Oscillator": (time: 0.0, period: 8.0, shape: Sine, phase: 0.0, amplitude: 0.5), ... },
       wires: { "time": "midiTime" }),
Entity(id: "lfoB", ..., wires: { "amplitude": "lfoA", "time": "midiTime" }),
```

`main.rs`: construct `(tx, rx)`, `open_input`, `add_plugins(sway_midi::MidiPlugin { rx })`, remove `feed_midi` / `MidiRx` / `MidiClockOffset` / `sway_nodes::MidiPlugin`.

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-midi crates/sway-app
git commit -m "$(cat <<'EOF'
feat(midi): MidiTime writes beat ppq onto FloatOut.

EOF
)"
```

---

### Task 8: Editor reads `Res<Transport>`

**Files:**
- Modify: `crates/sway-editor/Cargo.toml` (add `sway-midi`)
- Modify: `crates/sway-editor/src/snapshot.rs` (`capture_transport`)
- Modify: `crates/sway-editor/src/snapshot.rs` tests that mutate `Time<Transport>`

**Interfaces:**
- Consumes: `sway_midi::{Transport, MusicalTime}`.
- Produces: same `TransportView`; `position` from `MusicalTime::from_ppq(ppq, beats_per_bar).to_string()`.

- [ ] **Step 1: Write the failing test change**

In `the_snapshot_carries_the_transport_readout`, insert `Transport { playing: true, bpm: 128.0, ppq: 17.5, locked: true, beats_per_bar: 4 }` instead of `Time<Transport>`. Expect `position == "005.2.3"` (same as `MusicalTime::from_beats(17.5, 4)` today).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sway-editor the_snapshot_carries_the_transport_readout`

Expected: FAIL — still reads `Time<Transport>`, resource missing → default STOP.

- [ ] **Step 3: Implement `capture_transport`**

```rust
fn capture_transport(world: &World) -> TransportView {
    let Some(t) = world.get_resource::<sway_midi::Transport>() else {
        return TransportView::default();
    };
    TransportView {
        playing: t.playing,
        bpm: t.bpm as f32,
        position: MusicalTime::from_ppq(t.ppq, t.beats_per_bar).to_string(),
        locked: t.locked,
    }
}
```

Drop `TransportTime` import. `bevy_time` may remain unused — remove from `Cargo.toml` if nothing else needs it.

- [ ] **Step 4: Run tests**

Run: `cargo test -p sway-editor && cargo test --workspace`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-editor
git commit -m "$(cat <<'EOF'
feat(editor): transport bar reads the MIDI Transport snapshot.

EOF
)"
```

---

### Task 9: Delete the old stack

Remove `Time<Transport>`, `sway-nodes` MIDI plugin, `ClockEstimator::beats_at`, app `midi_feed`.

**Files:**
- Delete: `crates/sway-graph/src/transport.rs`
- Delete: `crates/sway-nodes/src/midi.rs`, `crates/sway-nodes/src/transport.rs`
- Delete: `crates/sway-app/src/midi_feed.rs` if still present
- Modify: `crates/sway-graph/src/lib.rs`, `crates/sway-graph/src/run.rs` (do not `init_resource::<Time<Transport>>`, do not register `Transport`/`TransportState`)
- Modify: `crates/sway-nodes/src/lib.rs` (no `midi`/`transport` modules, no `MidiPlugin`, drop `sway-midi-core` from `Cargo.toml`)
- Modify: `crates/sway-midi-core/src/transport.rs` — delete `beats_at` / `generation` if nothing else uses them; keep `push_pulse` + `secs_per_pulse` + `bpm` for `PulseClock`
- Modify: any remaining `use sway_graph::{Transport, TransportTime}`

**Interfaces:**
- Consumes: Task 6 `Transport` resource.
- Produces: graph crate with no beat clock.

- [ ] **Step 1: Write the failing assertion** in `crates/sway-graph/src/run.rs` tests

Replace `the_wires_plugin_inserts_the_transport_clock` (in `transport.rs`, deleted) with a test on `WiresPlugin`:

```rust
#[test]
fn the_wires_plugin_does_not_insert_a_beat_clock() {
    let mut app = App::new();
    app.add_plugins(WiresPlugin);
    // Type is gone; this test just proves WiresPlugin builds.
    assert!(app.world().get_resource::<GraphOrder>().is_some());
}
```

Delete the old test that expected `Time<Transport>`.

- [ ] **Step 2: Run `cargo test --workspace` to collect errors**

Expected: missing `MidiPlugin` / `Time<Transport>` in traces and graph tests.

- [ ] **Step 3: Fix callers**

`crates/sway-nodes/tests/traces.rs`: drive `PulseClock` / `sway_midi::MidiPlugin` **or** drop transport assertions and feed authored `Oscillator.time`. Spec: traces that assumed freewheel are rewritten to hold; traces that assumed `Lfo` self-clocking pass authored `time` or wire `MidiTime`.

Practical approach for this task: traces that tick `Time<Transport>` switch to `sway_midi::MidiPlugin` with a dummy `Receiver` and push `MidiInbox` events as `MidiMessage`. Add `sway-midi` as a **dev-dependency** of `sway-nodes` for traces only — **forbidden** if it becomes a normal dependency. Prefer moving `tests/traces.rs` transport cases into `crates/sway-midi` tests, and keep node traces MIDI-free (authored oscillator time).

Do that: **move** clock/transport golden traces to `crates/sway-midi/tests/` or `plugin.rs` tests. Leave `sway-nodes/tests/traces.rs` covering math/envelope/oscillator without a playhead.

`beat.rs`: change `beat_pulses` to:

```rust
pub fn beat_pulses(
    state: &mut BeatTriggerState,
    division: Division,
    playing: bool,
    beats_per_bar: u32,
    prev_ppq: Option<f64>,
    ppq: f64,
    dt: f32,
) -> Vec<BeatPulse>
```

If `!playing` or `ppq` jumped backward or jumped by more than `1.0` (relocate), reset state and return empty. Otherwise `start = prev_ppq.unwrap_or(ppq)`, `end = ppq`, `advanced = end - start`. Build `Beat` with the same bar/beat/sixteenth math as `MusicalTime::from_ppq` inlined (copy the arithmetic; do not depend on `sway-midi`). Drop `use sway_graph::MusicalTime`.

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`

Expected: PASS. `sway-nodes` Cargo.toml has no `sway-midi` / `sway-midi-core`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
refactor: remove Time<Transport> and the nodes MIDI plugin.

EOF
)"
```

---

### Task 10: Docs and leftover names

**Files:**
- Modify: `docs/architecture.md` §4 transport ownership, §5 table, §7 schedule, §8 crate list
- Modify: `docs/superpowers/specs/2026-08-16-midi-redesign-design.md` status → Accepted
- Modify: `docs/superpowers/specs/2026-07-25-sway-design.md` M9 note: crate split + `MidiTime` are done; `MidiNote` / `BeatTrigger` / `Envelope` remain
- Grep: `Time<Transport>`, `MidiEvent`, `RawMidi`, `ClockEstimator::beats_at`, `Lfo`, `feed_midi`, `MidiClockOffset`

**Interfaces:** none.

- [ ] **Step 1: Grep for leftovers**

Run: `rg -n "Time<Transport>|MidiEvent|RawMidi|MidiClockOffset|struct Lfo|feed_midi|beats_at" --glob '!docs/superpowers/plans/**' --glob '!docs/superpowers/reports/**'`

Expected: only historical docs, or this spec/plan.

- [ ] **Step 2: Patch `architecture.md`**

- Crate list: `sway-midi-core` (IO, messages, `PulseClock`); `sway-midi` (plugin, `Transport`, `MidiTime`).
- Ownership: beat / transport clock → `sway-midi`. Graph stays MIDI-free.
- Schedule: `FixedUpdate` feed → drain → sample `Transport` → write `MidiTime` → graph tick.
- Delete claims that `sway-midi` owns MIDI nodes other than `MidiTime`.

- [ ] **Step 3: Run tests**

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add docs crates
git commit -m "$(cat <<'EOF'
docs: record the MIDI crate split and pulse-grid playhead.

EOF
)"
```

---

## Self-review

**Spec coverage**

| Spec item | Task |
|---|---|
| D1 two crates | 3 |
| D2 pulse grid | 2 |
| D3 hold, no skip-count | 2 |
| D4 `Transport` in `sway-midi` | 6 |
| D5 `Oscillator` | 5 |
| D6 `MidiTime` | 7 |
| D7 `FloatOut` in graph | 4 |
| Host-time inbox, no `MidiClockOffset` | 6, 7 |
| Editor readout | 8 |
| Delete `Time<Transport>` | 9 |
| `architecture.md` | 10 |
| CoreMIDI destination unchanged | 1 (no behaviour change) |
| M9 nodes not added | — |

**Type names used throughout:** `MidiMessage`, `TimedMidi`, `PulseClock`, `Transport`, `MidiClock`, `MidiInbox`, `TickMidi`, `MidiPlugin`, `MidiTime`, `Oscillator`, `TimeFrom`, `MusicalTime::from_ppq`.
