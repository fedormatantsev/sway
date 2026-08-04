//! MIDI ingress: the CoreMIDI channel into the graph's timestamped inbox.
//!
//! Moved out of the throwaway `bridge.rs` at M2b unchanged — this is ingress,
//! not the temporary cube graph (design §9). M2a's open finding travels with
//! it: the epoch is sampled at first drain, and long-session mach-versus-fixed
//! drift is uncorrected. That is M3's, with the transport.

use bevy::prelude::*;
use crossbeam_channel::Receiver;
use sway_midi::MidiEvent;
use sway_nodes::{MidiInbox, RawMidi};

/// The receiving end of the CoreMIDI channel.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<MidiEvent>);

/// Offset from mach-absolute seconds to the graph's fixed-clock epoch.
#[derive(Resource, Default)]
pub struct MidiTimeEpoch(Option<f64>);

/// Moves every CoreMIDI callback event into the graph's timestamped inbox.
pub fn feed_midi(
    rx: Res<MidiRx>,
    time: Res<Time<Fixed>>,
    mut epoch: ResMut<MidiTimeEpoch>,
    mut inbox: ResMut<MidiInbox>,
) {
    let elapsed = time.elapsed_secs_f64();
    while let Ok(event) = rx.0.try_recv() {
        let epoch = *epoch.0.get_or_insert_with(|| {
            sway_midi::host_time_to_secs(sway_midi::host_time_now()) - elapsed
        });
        // DAWs (Ableton) often stamp packets ahead of the audio playhead. A
        // zero stamp means "now". Pathological far-future stamps would sit in
        // the inbox forever; clamp those to the current fixed elapsed time.
        let mut t = if event.host_time == 0 {
            elapsed
        } else {
            sway_midi::host_time_to_secs(event.host_time) - epoch
        };
        if t > elapsed + 0.5 {
            t = elapsed;
        }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use sway_nodes::MidiInbox;

    #[test]
    fn host_time_near_now_maps_to_fixed_elapsed_time() {
        let host_time = sway_midi::host_time_now();
        let elapsed = 42.0;
        // Pre-seed the epoch from the same stamp so the mapping is exact algebra,
        // not a race between send-time and first-drain host_time_now() samples.
        let epoch = sway_midi::host_time_to_secs(host_time) - elapsed;

        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::MidiEvent {
            status: 0x90,
            data1: 60,
            data2: 100,
            host_time,
        })
        .unwrap();

        let mut fixed = Time::<Fixed>::from_hz(120.0);
        fixed.advance_by(Duration::from_secs_f64(elapsed));
        let mut app = App::new();
        app.insert_resource(fixed)
            .insert_resource(MidiRx(rx))
            .insert_resource(MidiTimeEpoch(Some(epoch)))
            .init_resource::<MidiInbox>()
            .add_systems(PreUpdate, feed_midi);
        app.update();

        let mapped = app.world().resource::<MidiInbox>().events[0].0;
        assert!(
            (mapped - elapsed).abs() < 1e-9,
            "near-now host timestamp mapped to {mapped}, expected fixed elapsed {elapsed}s"
        );
    }

    #[test]
    fn feed_midi_drains_every_event_into_the_inbox() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::MidiEvent {
            status: 0x90,
            data1: 60,
            data2: 100,
            host_time: 1,
        })
        .unwrap();
        tx.send(sway_midi::MidiEvent {
            status: 0x80,
            data1: 60,
            data2: 0,
            host_time: 2,
        })
        .unwrap();

        let mut app = App::new();
        app.insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(MidiRx(rx))
            .init_resource::<MidiTimeEpoch>()
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
        tx.send(sway_midi::MidiEvent {
            status: 0x90,
            data1: 60,
            data2: 100,
            host_time: 0,
        })
        .unwrap();

        let mut fixed = Time::<Fixed>::from_hz(120.0);
        fixed.advance_by(Duration::from_secs_f64(7.0));
        let mut app = App::new();
        app.insert_resource(fixed)
            .insert_resource(MidiRx(rx))
            .init_resource::<MidiTimeEpoch>()
            .init_resource::<MidiInbox>()
            .add_systems(PreUpdate, feed_midi);
        app.update();

        let mapped = app.world().resource::<MidiInbox>().events[0].0;
        assert!(
            (mapped - 7.0).abs() < 1e-9,
            "zero host_time must mean now; got {mapped}"
        );
    }
}
