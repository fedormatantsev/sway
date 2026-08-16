use std::collections::VecDeque;

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::{Res, ResMut};
use crossbeam_channel::Receiver;

use crate::{MidiMessage, PulseClock, TimedMidi, Transport, host_time_now, host_time_to_secs};

#[derive(Resource)]
pub struct MidiClock {
    pub clock: PulseClock,
    pub tick_start_host: f64,
}

impl Default for MidiClock {
    fn default() -> Self {
        Self {
            clock: PulseClock::new(),
            tick_start_host: f64::NEG_INFINITY,
        }
    }
}

#[derive(Resource)]
pub struct MidiRx(pub Receiver<TimedMidi>);

#[derive(Resource, Default)]
pub struct MidiInbox {
    pub events: VecDeque<(f64, MidiMessage)>,
}

#[derive(Resource, Default)]
pub struct TickMidi {
    pub events: Vec<(f32, MidiMessage)>,
}

pub struct MidiPlugin {
    pub rx: Receiver<TimedMidi>,
}

impl Plugin for MidiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MidiRx(self.rx.clone()))
            .init_resource::<MidiInbox>()
            .init_resource::<TickMidi>()
            .init_resource::<MidiClock>()
            .init_resource::<Transport>()
            .add_systems(
                FixedUpdate,
                (feed_midi, drain_and_clock)
                    .chain()
                    .before(sway_graph::graph_tick),
            );
    }
}

fn feed_midi(rx: Res<MidiRx>, mut inbox: ResMut<MidiInbox>) {
    let now = host_time_to_secs(host_time_now());
    feed_receiver(&rx.0, &mut inbox.events, now);
}

pub(crate) fn feed_receiver(
    rx: &Receiver<TimedMidi>,
    inbox: &mut VecDeque<(f64, MidiMessage)>,
    now: f64,
) {
    for event in rx.try_iter() {
        let timestamp = if event.host_time == 0 {
            now
        } else {
            host_time_to_secs(event.host_time)
        };
        inbox.push_back((timestamp, event.message));
    }
}

fn drain_and_clock(
    mut inbox: ResMut<MidiInbox>,
    mut tick_midi: ResMut<TickMidi>,
    mut midi_clock: ResMut<MidiClock>,
    mut transport: ResMut<Transport>,
) {
    let tick_end = host_time_to_secs(host_time_now());
    let tick_start = if midi_clock.tick_start_host == f64::NEG_INFINITY {
        tick_end
    } else {
        midi_clock.tick_start_host
    };

    tick_midi.events = drain_window(&mut inbox.events, tick_start, tick_end);
    apply_tick(&mut midi_clock.clock, &tick_midi.events, tick_start);
    *transport = Transport {
        ppq: midi_clock.clock.ppq(tick_end),
        bpm: midi_clock.clock.bpm(),
        playing: midi_clock.clock.playing(),
        locked: midi_clock.clock.locked(tick_end),
        beats_per_bar: midi_clock.clock.beats_per_bar(),
    };
    midi_clock.tick_start_host = tick_end;
}

pub fn drain_window(
    inbox: &mut VecDeque<(f64, MidiMessage)>,
    tick_start: f64,
    tick_end: f64,
) -> Vec<(f32, MidiMessage)> {
    let dt = (tick_end - tick_start).max(0.0);
    let mut due = Vec::new();
    let mut future = VecDeque::new();

    while let Some((timestamp, message)) = inbox.pop_front() {
        if timestamp <= tick_end {
            // `f64::clamp` panics if bounds cross. `max`/`min` also make a
            // non-finite offset harmless instead of taking down the tick.
            let offset = (timestamp - tick_start).max(0.0).min(dt);
            due.push((offset as f32, message));
        } else {
            future.push_back((timestamp, message));
        }
    }
    *inbox = future;
    due
}

pub fn apply_tick(clock: &mut PulseClock, events: &[(f32, MidiMessage)], tick_start: f64) {
    for &(offset, message) in events {
        clock.push(tick_start + f64::from(offset), message);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use bevy_app::App;
    use bevy_time::{Fixed, Time, TimePlugin};
    use crossbeam_channel::Receiver;
    use sway_graph::WiresPlugin;

    use super::{
        MidiClock, MidiInbox, MidiPlugin, TickMidi, apply_tick, drain_window, feed_receiver,
    };
    use crate::{MidiMessage, TimedMidi, Transport};

    const SPP_120: f64 = 0.5 / 24.0;

    fn plugin_app(rx: Receiver<TimedMidi>) -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .add_plugins((WiresPlugin, MidiPlugin { rx }));
        app
    }

    #[test]
    fn the_plugin_inserts_transport_and_clock_resources() {
        let (tx, rx) = crossbeam_channel::unbounded();
        drop(tx);
        let app = plugin_app(rx);

        assert!(app.world().get_resource::<Transport>().is_some());
        assert!(app.world().get_resource::<MidiClock>().is_some());
        assert!(app.world().get_resource::<MidiInbox>().is_some());
        assert!(app.world().get_resource::<TickMidi>().is_some());
        assert!(
            app.world().get_resource::<Time<Transport>>().is_none(),
            "Transport is a snapshot resource, not a Bevy clock"
        );
    }

    #[test]
    fn feed_drains_every_event_into_the_inbox_in_order() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(TimedMidi {
            host_time: 0,
            message: MidiMessage::Start,
        })
        .unwrap();
        tx.send(TimedMidi {
            host_time: 0,
            message: MidiMessage::Clock,
        })
        .unwrap();
        let mut inbox = VecDeque::new();

        feed_receiver(&rx, &mut inbox, 7.0);

        assert_eq!(
            inbox.into_iter().collect::<Vec<_>>(),
            vec![(7.0, MidiMessage::Start), (7.0, MidiMessage::Clock)]
        );
    }

    #[test]
    fn zero_host_time_maps_to_now() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(TimedMidi {
            host_time: 0,
            message: MidiMessage::Stop,
        })
        .unwrap();
        let mut inbox = VecDeque::new();

        feed_receiver(&rx, &mut inbox, 123.25);

        assert_eq!(inbox.pop_front(), Some((123.25, MidiMessage::Stop)));
    }

    #[test]
    fn a_far_future_stamp_stays_in_the_inbox() {
        let mut inbox = VecDeque::from([(0.25, MidiMessage::Start), (10.0, MidiMessage::Clock)]);

        let tick = drain_window(&mut inbox, 0.0, 1.0);

        assert_eq!(tick, vec![(0.25, MidiMessage::Start)]);
        assert_eq!(
            inbox.into_iter().collect::<Vec<_>>(),
            vec![(10.0, MidiMessage::Clock)]
        );
    }

    #[test]
    fn drain_clamps_old_events_to_the_start_of_the_window() {
        let mut inbox = VecDeque::from([(-2.0, MidiMessage::Start)]);

        let tick = drain_window(&mut inbox, 1.0, 2.0);

        assert_eq!(tick, vec![(0.0, MidiMessage::Start)]);
    }

    #[test]
    fn start_then_twenty_four_clocks_set_playing_and_ppq() {
        let mut events = vec![(0.0, MidiMessage::Start)];
        events.extend((1..=24).map(|i| (i as f32 * SPP_120 as f32, MidiMessage::Clock)));
        let mut clock = crate::PulseClock::new();

        apply_tick(&mut clock, &events, 0.0);

        assert!(clock.playing());
        assert!((clock.ppq(24.0 * SPP_120) - 1.0).abs() < 1e-6);
    }
}
