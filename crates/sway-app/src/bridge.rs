// THROWAWAY. M2b's scene nodes delete this file. It exists so M2a has a live
// path: without it the engine is verified only by tests and never by an
// Octatrack plugged into a real machine (spec §10).

use bevy::prelude::*;
use crossbeam_channel::Receiver;
use sway_graph::{
    EdgeFrom, EdgeTo, GraphNode, NodeId, NodeType, NodeTypeRegistry, ParamEdge, PortArena,
    PortKind, compile,
};
use sway_midi::MidiEvent;
use sway_nodes::{
    Envelope, EnvelopeParams, EnvelopeState, MidiInbox, MidiNote, MidiNoteParams, MidiNoteState,
    RawMidi,
};

/// The receiving end of the CoreMIDI channel.
#[derive(Resource)]
pub struct MidiRx(pub Receiver<MidiEvent>);

/// Offset from mach-absolute seconds to the graph's fixed-clock epoch.
#[derive(Resource, Default)]
pub struct MidiTimeEpoch(Option<f64>);

/// Identifies the continuous arena slot that drives the M0 cube.
#[derive(Resource)]
pub struct CubeGraphOutput {
    pub entity: Entity,
    pub ordinal: u16,
}

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
        eprintln!(
            "midi in: status=0x{:02X} data1={} data2={} t={t:.4} elapsed={elapsed:.4}",
            event.status, event.data1, event.data2
        );
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

fn node_type_id<N: NodeType>(world: &World) -> sway_graph::NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("signal node type registered")
}

/// Builds and compiles the temporary MIDI-note-to-envelope graph.
pub fn setup_cube_graph(world: &mut World) {
    let midi_type = node_type_id::<MidiNote>(world);
    let envelope_type = node_type_id::<Envelope>(world);
    let midi = world
        .spawn((
            GraphNode {
                id: NodeId(0),
                node_type: midi_type,
            },
            MidiNoteParams {
                channel: 0,
                note_lo: 0,
                note_hi: 127,
            },
            MidiNoteState,
        ))
        .id();
    let envelope = world
        .spawn((
            GraphNode {
                id: NodeId(1),
                node_type: envelope_type,
            },
            EnvelopeParams {
                trigger: sway_graph::Event::default(),
                release_trigger: sway_graph::Event::default(),
                attack: 0.01,
                decay: 0.1,
                sustain: 0.7,
                release: 0.3,
            },
            EnvelopeState::default(),
        ))
        .id();
    world.spawn((
        ParamEdge {
            source_port: MidiNote::OUT_NOTE_ON,
            target_port: Envelope::TRIGGER,
            kind: PortKind::Event,
        },
        EdgeFrom(midi),
        EdgeTo(envelope),
    ));
    world.spawn((
        ParamEdge {
            source_port: MidiNote::OUT_NOTE_OFF,
            target_port: Envelope::RELEASE_TRIGGER,
            kind: PortKind::Event,
        },
        EdgeFrom(midi),
        EdgeTo(envelope),
    ));

    let compiled = compile(world).expect("temporary cube graph must compile");
    world
        .resource_mut::<PortArena>()
        .resize(compiled.continuous_len, compiled.events_len);
    world.insert_resource(compiled);
    world.insert_resource(CubeGraphOutput {
        entity: envelope,
        ordinal: Envelope::OUT_VALUE,
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use sway_graph::{CompiledGraph, EdgeFrom, EdgeTo, ParamEdge, PortKind};
    use sway_nodes::{Envelope, MidiInbox, MidiNote, SignalNodesPlugin};

    #[test]
    fn host_time_near_now_maps_to_fixed_elapsed_time() {
        let (tx, rx) = crossbeam_channel::unbounded();
        tx.send(sway_midi::MidiEvent {
            status: 0x90,
            data1: 60,
            data2: 100,
            host_time: sway_midi::host_time_now(),
        })
        .unwrap();

        let mut fixed = Time::<Fixed>::from_hz(120.0);
        fixed.advance_by(Duration::from_secs_f64(42.0));
        let mut app = App::new();
        app.insert_resource(fixed)
            .insert_resource(MidiRx(rx))
            .init_resource::<MidiTimeEpoch>()
            .init_resource::<MidiInbox>()
            .add_systems(PreUpdate, feed_midi);
        app.update();

        let mapped = app.world().resource::<MidiInbox>().events[0].0;
        assert!(
            (mapped - 42.0).abs() < 0.05,
            "near-now host timestamp mapped to {mapped}, expected near fixed elapsed 42s"
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

    #[test]
    fn cube_graph_compiles_note_on_and_note_off_edges() {
        let mut app = App::new();
        app.add_plugins((sway_graph::GraphPlugin, SignalNodesPlugin));

        setup_cube_graph(app.world_mut());

        let output = app.world().resource::<CubeGraphOutput>();
        let output_entity = output.entity;
        assert_eq!(output.ordinal, Envelope::OUT_VALUE);
        assert!(
            app.world()
                .get::<sway_graph::NodeRuntime>(output_entity)
                .is_some()
        );
        assert_eq!(app.world().resource::<CompiledGraph>().plans.len(), 2);

        let mut edges = app
            .world_mut()
            .query::<(&ParamEdge, &EdgeFrom, &EdgeTo)>()
            .iter(app.world())
            .map(|(edge, from, to)| (edge.source_port, edge.target_port, edge.kind, from.0, to.0))
            .collect::<Vec<_>>();
        edges.sort_by_key(|edge| edge.0);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].0, MidiNote::OUT_NOTE_ON);
        assert_eq!(edges[0].1, Envelope::TRIGGER);
        assert_eq!(edges[0].2, PortKind::Event);
        assert_eq!(edges[0].4, output_entity);
        assert_eq!(edges[1].0, MidiNote::OUT_NOTE_OFF);
        assert_eq!(edges[1].1, Envelope::RELEASE_TRIGGER);
        assert_eq!(edges[1].2, PortKind::Event);
        assert_eq!(edges[1].4, output_entity);
        assert_eq!(edges[0].3, edges[1].3);
    }
}
