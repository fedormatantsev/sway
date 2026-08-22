use std::collections::VecDeque;

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::{Res, ResMut};
use crossbeam_channel::Receiver;
use sway_events::RegisterEventHandle;
use sway_graph::graph::RegisterNodeKind;

use crate::{
    MidiControls, MidiMessage, PulseClock, TimedMidi, Transport, host_time_now, host_time_to_secs,
};

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
        // The whole domain: the transport's resources and systems *and* the
        // node kinds that read them. A host adds this and nothing else.
        app.register_node_kind::<crate::nodes::MidiTime>()
            .register_type::<crate::nodes::MidiTimeOut>()
            .register_node_kind::<crate::nodes::MidiCc>()
            .register_type::<crate::nodes::MidiCcIn>()
            .register_type::<crate::nodes::MidiCcOut>()
            // The notes node brings its payload and its handle with it: a
            // host that adds `MidiPlugin` registers neither on the domain's
            // behalf (`midi`: MIDI note events live in the MIDI domain).
            .register_node_kind::<crate::nodes::MidiNotes>()
            .register_type::<crate::nodes::MidiNotesOut>()
            .register_type::<crate::nodes::NoteEvent>()
            .register_event_handle::<crate::nodes::NoteEvent>()
            .insert_resource(MidiRx(self.rx.clone()))
            .init_resource::<MidiInbox>()
            .init_resource::<TickMidi>()
            .init_resource::<MidiClock>()
            .init_resource::<MidiControls>()
            .init_resource::<Transport>()
            .add_systems(
                FixedUpdate,
                (feed_inbox, drain_and_clock)
                    .chain()
                    .before(sway_graph::GraphTickSet),
            );
    }
}

fn feed_inbox(rx: Res<MidiRx>, mut inbox: ResMut<MidiInbox>) {
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
    mut controls: ResMut<MidiControls>,
) {
    let tick_end = host_time_to_secs(host_time_now());
    let tick_start = if midi_clock.tick_start_host == f64::NEG_INFINITY {
        tick_end
    } else {
        midi_clock.tick_start_host
    };

    tick_midi.events = drain_window(&mut inbox.events, tick_start, tick_end);
    apply_controls(&mut controls, &tick_midi.events);
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

/// Folds this tick's Control Changes into the session snapshot, in arrival
/// order — so the last matching message of a burst is the value every
/// `MidiCc` node reads when the graph ticks (design D1).
pub fn apply_controls(controls: &mut MidiControls, events: &[(f32, MidiMessage)]) {
    for &(_, message) in events {
        if let MidiMessage::Control { channel, cc, value } = message {
            controls.set(channel, cc, value);
        }
    }
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
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use crossbeam_channel::Receiver;
    use sway_graph::GraphPlugin;
    use sway_graph::graph::testing::read_field;
    use sway_graph::graph::{Graph, Node, Part};

    use super::{
        MidiClock, MidiInbox, MidiPlugin, TickMidi, apply_tick, drain_window, feed_receiver,
    };
    use crate::{MidiControls, MidiMessage, TimedMidi, Transport};

    const SPP_120: f64 = 0.5 / 24.0;

    fn plugin_app(rx: Receiver<TimedMidi>) -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, MidiPlugin { rx }));
        app.update();
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
        assert!(app.world().get_resource::<MidiControls>().is_some());
    }

    /// Pushes messages straight into the inbox, stamped now, and runs the
    /// tick that drains them.
    fn tick_with(app: &mut App, messages: impl IntoIterator<Item = MidiMessage>) {
        let now = crate::host_time_to_secs(crate::host_time_now());
        let mut inbox = app.world_mut().resource_mut::<MidiInbox>();
        for message in messages {
            inbox.events.push_back((now, message));
        }
        app.update();
    }

    #[test]
    fn an_inbox_control_lands_in_the_snapshot() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = plugin_app(rx);

        tick_with(
            &mut app,
            [MidiMessage::Control {
                channel: 0,
                cc: 1,
                value: 127,
            }],
        );

        let controls = app.world().resource::<MidiControls>();
        assert_eq!(controls.get(0.0, 1.0), 127, "the raw value, not scaled");
    }

    #[test]
    fn a_later_control_overwrites_the_same_cell() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = plugin_app(rx);

        tick_with(
            &mut app,
            [
                MidiMessage::Control {
                    channel: 0,
                    cc: 1,
                    value: 10,
                },
                MidiMessage::Control {
                    channel: 0,
                    cc: 1,
                    value: 20,
                },
            ],
        );

        assert_eq!(app.world().resource::<MidiControls>().get(0.0, 1.0), 20);
    }

    #[test]
    fn a_control_on_another_cell_leaves_the_first_alone() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = plugin_app(rx);
        tick_with(
            &mut app,
            [MidiMessage::Control {
                channel: 0,
                cc: 1,
                value: 64,
            }],
        );

        tick_with(
            &mut app,
            [MidiMessage::Control {
                channel: 2,
                cc: 3,
                value: 99,
            }],
        );

        let controls = app.world().resource::<MidiControls>();
        assert_eq!(controls.get(0.0, 1.0), 64, "held across the tick");
        assert_eq!(controls.get(2.0, 3.0), 99);
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

    #[test]
    fn a_clock_dropout_holds_ppq() {
        let mut events = vec![(0.0, MidiMessage::Start)];
        events.extend((1..=96).map(|i| (i as f32 * SPP_120 as f32, MidiMessage::Clock)));
        let mut clock = crate::PulseClock::new();
        apply_tick(&mut clock, &events, 0.0);
        let t_last = 96.0 * SPP_120;
        let before = clock.ppq(t_last);

        assert!(
            (clock.ppq(t_last + 1.0) - before).abs() < 1.0 / 24.0 + 1e-9,
            "dropout must hold, not freewheel"
        );
    }

    #[test]
    fn a_tempo_change_is_followed() {
        let spp_90 = (60.0 / 90.0) / 24.0;
        let mut events = vec![(0.0, MidiMessage::Start)];
        events.extend((1..=96).map(|i| (i as f32 * SPP_120 as f32, MidiMessage::Clock)));
        let start_90 = 96.0 * SPP_120;
        events
            .extend((1..=96).map(|i| ((start_90 + i as f64 * spp_90) as f32, MidiMessage::Clock)));
        let mut clock = crate::PulseClock::new();
        apply_tick(&mut clock, &events, 0.0);

        assert!(
            (clock.bpm() - 90.0).abs() < 1.0,
            "followed to {} BPM",
            clock.bpm()
        );
    }

    #[test]
    fn a_node_added_after_the_message_still_publishes_it() {
        // `midi`: a new node sees the session's last matching value — the
        // snapshot is what makes that true, not the node's own state.
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = plugin_app(rx);
        tick_with(
            &mut app,
            [MidiMessage::Control {
                channel: 0,
                cc: 1,
                value: 127,
            }],
        );

        let node = app
            .world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(crate::MidiCc {
                inlets: crate::MidiCcIn {
                    channel: 0.0,
                    cc: 1.0,
                },
                ..Default::default()
            }));
        app.update();

        let graph = app.world().resource::<Graph>();
        assert_eq!(read_field::<f32>(graph, node, Part::Outlets, "out"), 1.0);
    }

    /// The notes node needs the arena as well as the domain, which is what
    /// `sway-app` wires up: `EventsPlugin` beside `GraphPlugin`.
    fn notes_app(rx: Receiver<TimedMidi>) -> (App, sway_graph::graph::NodeId) {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, sway_events::EventsPlugin, MidiPlugin { rx }));
        app.update();
        let node = app
            .world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(crate::MidiNotes::default()));
        (app, node)
    }

    fn published_notes(app: &App, node: sway_graph::graph::NodeId) -> Vec<crate::NoteEvent> {
        let handle = read_field::<sway_events::EventHandle<crate::NoteEvent>>(
            app.world().resource::<Graph>(),
            node,
            Part::Outlets,
            "notes",
        );
        app.world()
            .get_non_send::<sway_events::EventArena>()
            .and_then(|arena| arena.read(handle))
            .map(|batch| batch.iter().copied().collect())
            .unwrap_or_default()
    }

    #[test]
    fn one_plugin_puts_the_notes_node_in_the_palette_and_ticks_it() {
        // `midi`: One plugin is the whole domain — and the host registered
        // neither the payload nor its handle on the domain's behalf.
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, sway_events::EventsPlugin, MidiPlugin { rx }));
        app.update();

        let registry = app
            .world()
            .resource::<bevy_ecs::reflect::AppTypeRegistry>()
            .clone();
        let path = {
            let read = registry.read();
            assert!(
                read.get_with_type_path("sway_midi::nodes::midi_notes::NoteEvent")
                    .is_some(),
                "the payload came with the plugin"
            );
            *sway_graph::graph::registered_node_kinds(&read)
                .iter()
                .find(|kind| kind.ends_with("::MidiNotes"))
                .expect("the notes node is in the palette")
        };

        // Created from the palette by type path, exactly as the editor does.
        let node = {
            let read = registry.read();
            app.world_mut()
                .resource_mut::<Graph>()
                .create(&read, path)
                .expect("a registered node kind")
        };
        // No MIDI hardware, so no note ever arrives.
        app.update();

        assert_eq!(
            read_field::<sway_events::EventHandle<crate::NoteEvent>>(
                app.world().resource::<Graph>(),
                node,
                Part::Outlets,
                "notes",
            ),
            sway_events::EventHandle::EMPTY,
            "it ticks without MIDI hardware present"
        );
    }

    #[test]
    fn the_notes_node_publishes_the_ticks_notes_through_the_real_path() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let (mut app, node) = notes_app(rx);

        tick_with(
            &mut app,
            [
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 0,
                },
            ],
        );

        let notes = published_notes(&app, node);
        assert_eq!(notes.len(), 2, "both messages arrived as occurrences");
        assert!(notes[0].on && notes[0].note == 60);
        assert!(!notes[1].on, "in arrival order");
    }

    #[test]
    fn notes_do_not_survive_their_tick() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let (mut app, node) = notes_app(rx);
        tick_with(
            &mut app,
            [MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            }],
        );
        assert_eq!(published_notes(&app, node).len(), 1);

        app.update();

        assert_eq!(
            read_field::<sway_events::EventHandle<crate::NoteEvent>>(
                app.world().resource::<Graph>(),
                node,
                Part::Outlets,
                "notes",
            ),
            sway_events::EventHandle::EMPTY,
            "a second tick with no input leaves the empty handle"
        );
        assert!(
            published_notes(&app, node).is_empty(),
            "and the first tick's notes are not published again"
        );
    }

    #[test]
    fn inbox_start_updates_the_transport_snapshot() {
        let (_tx, rx) = crossbeam_channel::unbounded();
        let mut app = plugin_app(rx);
        let now = crate::host_time_to_secs(crate::host_time_now());
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .events
            .push_back((now, MidiMessage::Start));

        app.update();

        let transport = app.world().resource::<Transport>();
        assert!(transport.playing);
        assert_eq!(transport.ppq, 0.0);
    }
}
