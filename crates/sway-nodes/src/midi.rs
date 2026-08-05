//! MIDI buffering and pure message parsing.

use std::collections::VecDeque;

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawMidi {
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

#[derive(Reflect, Default, Debug, Clone, PartialEq, Eq)]
pub struct NoteMsg {
    pub note: u8,
    pub velocity: u8,
}

#[derive(Resource, Default)]
pub struct MidiInbox {
    pub events: VecDeque<(f64, RawMidi)>,
}

impl MidiInbox {
    pub fn push(&mut self, at: f64, message: RawMidi) {
        self.events.push_back((at, message));
    }
}

#[derive(Resource, Default)]
pub struct TickMidi {
    pub events: Vec<(f32, RawMidi)>,
}

pub fn drain_inbox(
    time: bevy_ecs::system::Res<Time<Fixed>>,
    mut inbox: bevy_ecs::system::ResMut<MidiInbox>,
    mut tick_midi: bevy_ecs::system::ResMut<TickMidi>,
) {
    let dt = time.delta_secs();
    let tick_start = time.elapsed_secs_f64() - dt as f64;
    let tick_end = tick_start + dt as f64;
    tick_midi.events.clear();
    inbox.events.retain(|&(event_time, message)| {
        if event_time <= tick_end {
            tick_midi.events.push((
                (event_time - tick_start).clamp(0.0, dt as f64) as f32,
                message,
            ));
            false
        } else {
            true
        }
    });
}

pub fn note_message(
    message: RawMidi,
    channel: u8,
    note_lo: u8,
    note_hi: u8,
) -> Option<(bool, NoteMsg)> {
    if message.status & 0x0f != channel || message.data1 < note_lo || message.data1 > note_hi {
        return None;
    }
    let note = NoteMsg {
        note: message.data1,
        velocity: message.data2,
    };
    match (message.status & 0xf0, message.data2) {
        (0x90, velocity) if velocity > 0 => Some((true, note)),
        (0x80, _) | (0x90, 0) => Some((false, note)),
        _ => None,
    }
}

pub fn cc_value(message: RawMidi, channel: u8, cc: u8) -> Option<f32> {
    (message.status & 0xf0 == 0xb0 && message.status & 0x0f == channel && message.data1 == cc)
        .then(|| message.data2 as f32 / 127.0)
}

/// MIDI-clock support retained independently of graph authoring types.
pub struct MidiPlugin;

impl Plugin for MidiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MidiInbox>()
            .init_resource::<TickMidi>()
            .init_resource::<crate::TransportClock>()
            .add_systems(
                FixedUpdate,
                (drain_inbox, crate::advance_transport.after(drain_inbox))
                    .before(sway_graph::graph_tick),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_velocity_note_on_is_note_off() {
        let parsed = note_message(
            RawMidi {
                status: 0x90,
                data1: 60,
                data2: 0,
            },
            0,
            0,
            127,
        );
        assert_eq!(parsed.map(|event| event.0), Some(false));
    }

    #[test]
    fn channel_and_range_filters_are_applied() {
        let message = RawMidi {
            status: 0x91,
            data1: 64,
            data2: 100,
        };
        assert!(note_message(message, 0, 60, 72).is_none());
        assert!(note_message(message, 1, 60, 72).is_some());
    }
}
