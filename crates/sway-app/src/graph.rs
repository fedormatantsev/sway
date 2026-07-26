//! The M0 graph: one hardcoded node. Replaced wholesale by `sway-graph` at M2.

use bevy::prelude::*;
use crossbeam_channel::Receiver;
use sway_midi::MidiEvent;

/// Graph tick rate. Spec §7 leaves the final number to M2 measurement; 120 Hz
/// is comfortably above frame rate and divides evenly into common tempos.
pub const TICK_HZ: f64 = 120.0;

/// How fast `level` falls back to zero, in units per second.
pub const DECAY_PER_SEC: f32 = 2.0;

/// The receiving end of the CoreMIDI channel.
///
/// NOTE: when `graph.rs` moves into `sway-graph` at M2, `MidiRx` stays
/// behind. The spec states the engine layer knows nothing about MIDI, so
/// this resource (and the note-on draining logic in `graph_tick`) is
/// `sway-app`-only scaffolding, not part of what gets lifted.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<MidiEvent>);

/// The entire graph state for M0.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct GraphState {
    /// 0.0 to 1.0, set by note velocity and decaying toward zero.
    pub level: f32,
}

/// The graph tick: one exclusive system in `FixedUpdate` (spec §2.6).
///
/// Drains every MIDI event that arrived since the last tick, applies note-ons,
/// then decays. Decay uses the fixed timestep rather than frame delta, so the
/// result depends only on how many ticks ran.
pub fn graph_tick(world: &mut World) {
    let mut notes: Vec<MidiEvent> = Vec::new();
    // `resource` (panicking), not `get_resource`, for consistency with the
    // other two resources below: all three are inserted together at startup
    // and the system cannot meaningfully run without any of them.
    let rx = world.resource::<MidiRx>();
    while let Ok(e) = rx.0.try_recv() {
        // Note-on with non-zero velocity. Many devices spell note-off as
        // note-on with velocity 0.
        if e.status & 0xF0 == 0x90 && e.data2 > 0 {
            notes.push(e);
        }
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs();
    let mut state = world.resource_mut::<GraphState>();
    for e in notes {
        state.level = e.data2 as f32 / 127.0;
    }
    // M0 SHORTCUT: this accumulates decay by subtracting `dt * DECAY_PER_SEC`
    // each tick, which violates spec §2.2's rule that time-varying values
    // must be derived from absolute time rather than accumulated — under
    // `Time<Fixed>::max_delta` tick-dropping this decays too slowly and
    // diverges silently, and it does not stay correct across pauses. This is
    // deliberately left as-is for M0 (one hardcoded node); the envelope and
    // LFO nodes this file's replacement grows at M2 must derive their values
    // from absolute time instead.
    state.level = (state.level - dt * DECAY_PER_SEC).max(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use crossbeam_channel::{Receiver, Sender};

    fn note_on(vel: u8) -> MidiEvent {
        MidiEvent {
            status: 0x90,
            data1: 60,
            data2: vel,
            host_time: 0,
        }
    }

    /// Headless app running FixedUpdate exactly once per `app.update()`.
    ///
    /// Frame 0 runs no fixed tick — the accumulator is empty until real time
    /// has advanced once — so one warm-up update is burned here.
    fn headless() -> (Sender<MidiEvent>, App) {
        let (tx, rx): (Sender<MidiEvent>, Receiver<MidiEvent>) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(MidiRx(rx))
            .init_resource::<GraphState>()
            .add_systems(FixedUpdate, graph_tick);
        app.update();
        (tx, app)
    }

    fn level(app: &App) -> f32 {
        app.world().resource::<GraphState>().level
    }

    #[test]
    fn warm_up_update_ran_no_ticks() {
        let (_tx, app) = headless();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn note_on_sets_level_then_one_tick_of_decay() {
        let (tx, mut app) = headless();
        tx.send(note_on(127)).unwrap();
        app.update();
        let expected = 1.0 - (1.0 / TICK_HZ as f32) * DECAY_PER_SEC;
        assert!(
            (level(&app) - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            level(&app)
        );
    }

    #[test]
    fn velocity_scales_level() {
        let (tx, mut app) = headless();
        tx.send(note_on(64)).unwrap();
        app.update();
        assert!(level(&app) > 0.4 && level(&app) < 0.55, "got {}", level(&app));
    }

    #[test]
    fn level_decays_to_zero_and_clamps() {
        let (tx, mut app) = headless();
        tx.send(note_on(127)).unwrap();
        for _ in 0..500 {
            app.update();
        }
        assert_eq!(level(&app), 0.0, "decay must clamp, not go negative");
    }

    #[test]
    fn note_off_is_ignored() {
        let (tx, mut app) = headless();
        tx.send(MidiEvent {
            status: 0x80,
            data1: 60,
            data2: 100,
            host_time: 0,
        })
        .unwrap();
        app.update();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn zero_velocity_note_on_is_ignored() {
        // Many devices send note-on with velocity 0 instead of note-off.
        let (tx, mut app) = headless();
        tx.send(note_on(0)).unwrap();
        app.update();
        assert_eq!(level(&app), 0.0);
    }

    #[test]
    fn identical_input_gives_bit_identical_output() {
        let run = || {
            let (tx, mut app) = headless();
            let mut trace = Vec::new();
            for i in 0..40 {
                if i == 0 {
                    tx.send(note_on(100)).unwrap();
                }
                if i == 5 {
                    tx.send(note_on(64)).unwrap();
                }
                app.update();
                trace.push(level(&app).to_bits());
            }
            trace
        };
        assert_eq!(run(), run(), "same input must give bit-identical output");
    }

    /// `identical_input_gives_bit_identical_output` only proves the graph is
    /// deterministic; it would pass just as happily if someone changed the
    /// decay rate or the velocity divisor, since it compares two runs to each
    /// other rather than to a known-good trace. This test pins actual values:
    /// the expected array below was obtained by running this exact sequence
    /// once and recording the resulting `level` at each of the 40 ticks.
    #[test]
    fn trace_matches_stored_expectations() {
        const EXPECTED: [f32; 40] = [
            0.7707349, 0.75406826, 0.7374016, 0.72073495, 0.7040683, 0.48727036, 0.4706037,
            0.45393705, 0.4372704, 0.42060375, 0.4039371, 0.38727045, 0.3706038, 0.35393715,
            0.3372705, 0.32060385, 0.3039372, 0.28727055, 0.2706039, 0.25393724, 0.23727058,
            0.22060391, 0.20393725, 0.18727058, 0.17060392, 0.15393725, 0.13727058, 0.12060392,
            0.10393725, 0.08727059, 0.07060392, 0.053937256, 0.03727059, 0.020603925,
            0.0039372593, 0.0, 0.0, 0.0, 0.0, 0.0,
        ];

        let (tx, mut app) = headless();
        let mut trace = Vec::new();
        for i in 0..40 {
            if i == 0 {
                tx.send(note_on(100)).unwrap();
            }
            if i == 5 {
                tx.send(note_on(64)).unwrap();
            }
            app.update();
            trace.push(level(&app));
        }

        assert_eq!(trace.len(), EXPECTED.len());
        for (i, (got, want)) in trace.iter().zip(EXPECTED.iter()).enumerate() {
            let got_rounded = (got * 1e6).round() / 1e6;
            let want_rounded = (want * 1e6).round() / 1e6;
            assert!(
                (got_rounded - want_rounded).abs() < 1e-6,
                "tick {i}: expected {want_rounded}, got {got_rounded} (raw {got})"
            );
        }
    }
}
