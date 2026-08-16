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
use sway_midi::{MidiMessage, TimedMidi};
use sway_nodes::{MidiInbox, RawMidi};

/// The receiving end of the CoreMIDI channel.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<TimedMidi>);

fn raw_midi(message: MidiMessage) -> RawMidi {
    match message {
        MidiMessage::NoteOn { channel, note, velocity } => RawMidi {
            status: 0x90 | channel,
            data1: note,
            data2: velocity,
        },
        MidiMessage::NoteOff { channel, note, velocity } => RawMidi {
            status: 0x80 | channel,
            data1: note,
            data2: velocity,
        },
        MidiMessage::Control { channel, cc, value } => RawMidi {
            status: 0xB0 | channel,
            data1: cc,
            data2: value,
        },
        MidiMessage::Clock => RawMidi { status: 0xF8, data1: 0, data2: 0 },
        MidiMessage::Start => RawMidi { status: 0xFA, data1: 0, data2: 0 },
        MidiMessage::Continue => RawMidi { status: 0xFB, data1: 0, data2: 0 },
        MidiMessage::Stop => RawMidi { status: 0xFC, data1: 0, data2: 0 },
        MidiMessage::SongPosition { sixteenths } => RawMidi {
            status: 0xF2,
            data1: (sixteenths & 0x7F) as u8,
            data2: ((sixteenths >> 7) & 0x7F) as u8,
        },
        MidiMessage::Other { status, data1, data2 } => RawMidi { status, data1, data2 },
    }
}

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
    let upper = elapsed + MAX_LOOKAHEAD;
    // `f64::clamp` panics if either bound is NaN or if the bounds cross.
    // Both happen in practice: a NaN `elapsed` (an upstream NaN period) makes
    // `upper` NaN, and a rewound `Time<Fixed>` can put `last_enqueued` ahead
    // of `upper`. `max`/`min` never panic and ignore NaN operands (returning
    // the other one), so this chain degrades to "keep the floor" instead of
    // aborting the tick. `last_enqueued > upper` still means `.min(upper)`
    // pulls the value down, so the trailing `.max(last_enqueued)` is what
    // makes the floor win in that case too.
    t.max(last_enqueued).min(upper).max(last_enqueued)
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
            raw_midi(event.message),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use sway_nodes::MidiInbox;

    #[test]
    fn feed_midi_drains_every_event_into_the_inbox() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::TimedMidi {
            host_time: 1,
            message: sway_midi::MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        })
        .unwrap();
        tx.send(sway_midi::TimedMidi {
            host_time: 2,
            message: sway_midi::MidiMessage::NoteOff {
                channel: 0,
                note: 60,
                velocity: 0,
            },
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
        assert_eq!(inbox.events.len(), 2);
        assert_eq!(inbox.events[0].1.status, 0x90);
        assert_eq!(inbox.events[1].1.status, 0x80);
        assert!(inbox.events[1].0 > inbox.events[0].0);
    }

    #[test]
    fn zero_host_time_maps_to_current_fixed_elapsed() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::TimedMidi {
            host_time: 0,
            message: sway_midi::MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        })
        .unwrap();

        let mut fixed = Time::<Fixed>::from_hz(120.0);
        fixed.advance_by(Duration::from_secs_f64(7.0));
        let mut app = App::new();
        app.insert_resource(fixed)
            .insert_resource(MidiRx(rx))
            .init_resource::<MidiClockOffset>()
            .init_resource::<MidiInbox>()
            .add_systems(PreUpdate, feed_midi);
        app.update();

        let mapped = app.world().resource::<MidiInbox>().events[0].0;
        assert!(
            (mapped - 7.0).abs() < 1e-9,
            "zero host_time must mean now; got {mapped}"
        );
    }

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
        // DEVIATION (brief's test math): with OFFSET_WINDOW=240 and a
        // 0.001/step increment, the window can carry at most (WINDOW + 1) *
        // 0.001 ≈ 0.241 of drift above `stale` once it has fully rolled past
        // the first sample — the brief's `stale + 0.5` threshold is above
        // the maximum sample ever pushed (1.48) and can never be satisfied.
        // 0.2 is the largest round margin the sliding window can actually
        // demonstrate here, while still proving the stale sample (1.0) does
        // not pin the offset.
        assert!(
            offset > stale + 0.2,
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
    fn a_nan_elapsed_does_not_panic_and_holds_the_floor() {
        // The tick is infallible: a NaN period upstream must not reach
        // `f64::clamp`'s bounds and abort the tick. With `elapsed` NaN, the
        // upper bound is undefined, so the floor (`last_enqueued`) wins.
        let last_enqueued = 5.0;
        let t = map_timestamp(1.0, 1.0, f64::NAN, last_enqueued);
        assert!(!t.is_nan(), "got NaN instead of a sane fallback");
        assert!((t - last_enqueued).abs() < 1e-12, "got {t}, expected the floor {last_enqueued}");
    }

    #[test]
    fn crossed_bounds_from_a_rewound_clock_does_not_panic_and_holds_the_floor() {
        // A rewound Time<Fixed> without a matching MidiClockOffset reset can
        // put last_enqueued ahead of elapsed + MAX_LOOKAHEAD. The tick must
        // still not panic, and ordering still demands the floor wins.
        let last_enqueued = 1000.0;
        let t = map_timestamp(10.0, 1.0, 0.0, last_enqueued);
        assert!(
            (t - last_enqueued).abs() < 1e-12,
            "got {t}, expected the floor {last_enqueued}"
        );
    }

    #[test]
    fn a_clock_pulse_survives_the_bridge_with_its_status_intact() {
        // Task 1 made clock reachable; nothing between the callback and the
        // inbox may filter it out.
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::TimedMidi {
            host_time: 0,
            message: sway_midi::MidiMessage::Clock,
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

    #[test]
    fn typed_messages_map_back_to_raw_midi() {
        let cases = [
            (sway_midi::MidiMessage::Clock, RawMidi { status: 0xF8, data1: 0, data2: 0 }),
            (sway_midi::MidiMessage::Start, RawMidi { status: 0xFA, data1: 0, data2: 0 }),
            (sway_midi::MidiMessage::Continue, RawMidi { status: 0xFB, data1: 0, data2: 0 }),
            (sway_midi::MidiMessage::Stop, RawMidi { status: 0xFC, data1: 0, data2: 0 }),
            (
                sway_midi::MidiMessage::SongPosition { sixteenths: 0x108 },
                RawMidi { status: 0xF2, data1: 8, data2: 2 },
            ),
            (
                sway_midi::MidiMessage::NoteOn { channel: 3, note: 64, velocity: 100 },
                RawMidi { status: 0x93, data1: 64, data2: 100 },
            ),
            (
                sway_midi::MidiMessage::NoteOff { channel: 4, note: 65, velocity: 12 },
                RawMidi { status: 0x84, data1: 65, data2: 12 },
            ),
            (
                sway_midi::MidiMessage::Control { channel: 5, cc: 7, value: 99 },
                RawMidi { status: 0xB5, data1: 7, data2: 99 },
            ),
            (
                sway_midi::MidiMessage::Other { status: 0xE1, data1: 2, data2: 3 },
                RawMidi { status: 0xE1, data1: 2, data2: 3 },
            ),
        ];

        for (message, expected) in cases {
            let actual = raw_midi(message);
            assert_eq!(
                (actual.status, actual.data1, actual.data2),
                (expected.status, expected.data1, expected.data2)
            );
        }
    }
}
