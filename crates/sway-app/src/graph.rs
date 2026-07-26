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
    if let Some(rx) = world.get_resource::<MidiRx>() {
        while let Ok(e) = rx.0.try_recv() {
            // Note-on with non-zero velocity. Many devices spell note-off as
            // note-on with velocity 0.
            if e.status & 0xF0 == 0x90 && e.data2 > 0 {
                notes.push(e);
            }
        }
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs();
    let mut state = world.resource_mut::<GraphState>();
    for e in notes {
        state.level = e.data2 as f32 / 127.0;
    }
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
}
