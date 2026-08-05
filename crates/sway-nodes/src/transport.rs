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

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use sway_graph::{Transport, TransportState, TransportTime, WiresPlugin};

    use crate::{MidiInbox, MidiPlugin, RawMidi};

    const TICK_HZ: f64 = 120.0;

    fn transport_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((WiresPlugin, MidiPlugin));
        app.update();
        app
    }

    fn raw(status: u8) -> RawMidi {
        RawMidi {
            status,
            data1: 0,
            data2: 0,
        }
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
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);

        assert!(
            (bpm(&app) - 120.0).abs() < 0.5,
            "locked to {} BPM",
            bpm(&app)
        );
        assert!(app.world().resource::<Time<Transport>>().transport().locked);
    }

    #[test]
    fn beats_advance_one_per_half_second_at_120_bpm() {
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 8.0);
        run_until(&mut app, 2.0);

        // Two seconds of transport at 120 BPM is four beats, within the one
        // tick of quantization a mid-tick Start costs.
        assert!(
            (beats(&app) - 4.0).abs() < 0.1,
            "advanced {} beats",
            beats(&app)
        );
    }

    #[test]
    fn a_stopped_transport_does_not_advance_however_many_pulses_arrive() {
        let mut app = transport_app();
        // No Start: pulses set the tempo but must not scroll the visuals.
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);

        assert_eq!(
            app.world().resource::<Time<Transport>>().state(),
            TransportState::Stopped
        );
        assert_eq!(beats(&app), 0.0);
        assert!(
            (bpm(&app) - 120.0).abs() < 0.5,
            "tempo is still tracked while stopped"
        );
    }

    #[test]
    fn stop_freezes_beat_time_and_continue_resumes_it_where_it_stopped() {
        // The two pulse trains must not overlap in time: two pulses at
        // nearly the same instant are one pulse index apart and no time
        // apart, which collapses the fit's slope. The first train therefore
        // stops at t=1.0, exactly where the transport does.
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 2.0); // pulses over [0.0, 1.0)
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(1.0, raw(sway_midi::STOP));
        run_until(&mut app, 1.5);
        let frozen = beats(&app);

        run_until(&mut app, 0.5);
        assert_eq!(beats(&app), frozen, "a stopped transport must not advance");

        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(2.0, raw(sway_midi::CONTINUE));
        queue_clock(&mut app, 2.0, 120.0, 4.0); // pulses over [2.0, 4.0)
        run_until(&mut app, 1.0);
        assert!(beats(&app) > frozen, "continue resumes");
        assert!(
            beats(&app) < frozen + 2.5,
            "continue resumes, it does not restart"
        );
    }

    #[test]
    fn start_puts_the_playhead_back_at_the_top() {
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 8.0);
        run_until(&mut app, 2.0);
        assert!(beats(&app) > 3.0);

        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(2.0, raw(sway_midi::START));
        app.update();
        assert!(
            beats(&app) < 0.1,
            "Start is position zero, got {}",
            beats(&app)
        );
    }

    #[test]
    fn a_song_position_pointer_repositions_in_sixteenths() {
        let mut app = transport_app();
        // SPP counts sixteenths: 8 sixteenths is two beats.
        app.world_mut().resource_mut::<MidiInbox>().push(
            0.0,
            RawMidi {
                status: sway_midi::SONG_POSITION,
                data1: 8,
                data2: 0,
            },
        );
        app.update();
        assert!((beats(&app) - 2.0).abs() < 0.05, "got {}", beats(&app));
    }

    #[test]
    fn a_clock_dropout_freewheels_at_the_last_tempo() {
        // The chosen dropout policy: never freeze the screen. A cable glitch
        // costs drift, not a stopped visual.
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
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
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 4.0);
        run_until(&mut app, 2.0);
        run_until(&mut app, 1.0); // dropout
        let before = beats(&app);

        queue_clock(&mut app, 3.0, 120.0, 4.0);
        run_until(&mut app, 0.2);

        let advanced = beats(&app) - before;
        assert!(advanced >= 0.0, "beats never run backwards");
        assert!(
            advanced < 1.0,
            "re-locking must not jump a fit's origin into position: {advanced}"
        );
    }

    #[test]
    fn a_tempo_change_is_followed() {
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        let end = queue_clock(&mut app, 0.0, 120.0, 4.0);
        queue_clock(&mut app, end, 90.0, 6.0);
        run_until(&mut app, 6.0);

        assert!(
            (bpm(&app) - 90.0).abs() < 1.0,
            "followed to {} BPM",
            bpm(&app)
        );
    }

    #[test]
    fn the_system_runs_between_the_inbox_drain_and_the_graph_tick() {
        // Ordering, asserted rather than assumed: a node reading beat time in
        // its tick must see this tick's advance, not the previous one's.
        let mut app = transport_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, raw(sway_midi::START));
        queue_clock(&mut app, 0.0, 120.0, 2.0);
        app.update();
        assert_eq!(
            app.world().resource::<Time<Transport>>().state(),
            TransportState::Playing,
            "a Start drained this tick must take effect this tick"
        );
    }
}
