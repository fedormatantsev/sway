//! `MidiNotes`: the tick's note-on and note-off messages, published as one
//! batch of occurrences.
//!
//! The first real producer on the occurrence mechanism (design D11), and an
//! ordinary node kind in every other respect: it reads [`TickMidi`] out of
//! `&World` during its own evaluation, exactly as [`MidiCc`](crate::MidiCc)
//! reads [`MidiControls`](crate::MidiControls). Nothing new is scheduled — the
//! drain already fills `TickMidi` with each message and its offset before
//! `GraphTickSet`, and this node publishes *during* the tick rather than
//! before it, so there is no ordering constraint against `EventClearSet` to
//! get wrong.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
use sway_events::{EventArena, EventHandle};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::{MidiMessage, TickMidi};

/// One note message that arrived during this tick.
///
/// A struct with an `on` flag rather than an enum: every field is meaningful
/// for both kinds — a note-off carries release velocity — so an enum would
/// duplicate the payload to distinguish one boolean.
///
/// This payload is the MIDI domain's own vocabulary and stays here (design
/// D11). The boundary to what other domains understand is crossed inside this
/// domain, by the converter nodes that fire generic events.
#[derive(Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Debug, PartialEq)]
pub struct NoteEvent {
    /// The MIDI channel in the protocol's own 0–15 numbering, not display
    /// 1–16, as `MidiMessage` carries it.
    pub channel: u8,
    /// The note number, 0–127.
    pub note: u8,
    /// The velocity. On a note-off this is the **release** velocity, which is
    /// why it is carried for both kinds rather than only for a note-on.
    pub velocity: u8,
    /// `true` for a note-on, `false` for a note-off.
    pub on: bool,
    /// Seconds from the start of this tick — the sub-tick offset the MIDI
    /// drain already records. Nothing in this change reads it; it is here so
    /// the converter nodes that need exact retrigger timing do not need a new
    /// payload.
    pub offset: f32,
}

/// [`MidiNotes`]'s outlets.
#[derive(Reflect, Default, Debug)]
#[reflect(Default, Debug)]
pub struct MidiNotesOut {
    /// A handle naming this tick's batch of note occurrences, or the empty
    /// handle on a tick in which no note message arrived.
    pub notes: EventHandle<NoteEvent>,
}

/// Publishes every note message of the tick as one batch.
///
/// **It selects nothing.** `MidiCc` filters because it publishes *one* held
/// value and has to pick which; a batch does not. Publishing everything and
/// letting a later node choose costs nothing, avoids an "omni" encoding in an
/// `f32` inlet, and means one `MidiNotes` in a scene can feed every consumer —
/// which is why this node has no inlets at all (design D11).
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct MidiNotes {
    /// Inlets — none: there is nothing to choose.
    pub inlets: (),
    /// State — none: notes, batches and handles are never kept between ticks.
    pub state: (),
    /// Outlets.
    pub outlets: MidiNotesOut,
}

impl NodeKind for MidiNotes {
    fn evaluate(&mut self, world: &World) {
        // No arena and no MIDI input are each the empty handle, not a failed
        // evaluation.
        let Some(arena) = world.get_non_send::<EventArena>() else {
            self.outlets.notes = EventHandle::EMPTY;
            return;
        };
        let Some(tick) = world.get_resource::<TickMidi>() else {
            self.outlets.notes = EventHandle::EMPTY;
            return;
        };
        // Arrival order, and no message of any other kind.
        self.outlets.notes = arena.publish(
            tick.events
                .iter()
                .filter_map(|&(offset, message)| note_event(offset, message)),
        );
    }
}

/// One MIDI message as an occurrence, or `None` if it is not a note message.
fn note_event(offset: f32, message: MidiMessage) -> Option<NoteEvent> {
    match message {
        // A zero-velocity note-on is a note-off. `MidiMessage::from_bytes`
        // already folds the two on the wire, so this is the second line of
        // defence — for a `TickMidi` filled by anything else.
        MidiMessage::NoteOn {
            channel,
            note,
            velocity: 0,
        } => Some(NoteEvent {
            channel,
            note,
            velocity: 0,
            on: false,
            offset,
        }),
        MidiMessage::NoteOn {
            channel,
            note,
            velocity,
        } => Some(NoteEvent {
            channel,
            note,
            velocity,
            on: true,
            offset,
        }),
        MidiMessage::NoteOff {
            channel,
            note,
            velocity,
        } => Some(NoteEvent {
            channel,
            note,
            velocity,
            on: false,
            offset,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::TypeRegistry;
    use sway_events::{EventBatch, register_event_handle};
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing::{read_field, tick_once as tick, trace_world};
    use sway_graph::graph::{Graph, Node, NodeId, Part};

    use super::*;

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MidiNotes>(&mut registry);
        registry.register::<MidiNotesOut>();
        registry.register::<NoteEvent>();
        register_event_handle::<NoteEvent>(&mut registry);
        registry
    }

    /// A trace world with an arena and a `TickMidi` seeded with `events`.
    fn world_with(events: Vec<(f32, MidiMessage)>) -> World {
        let mut world = trace_world(registry());
        world.insert_non_send(EventArena::default());
        world.insert_resource(TickMidi { events });
        world
    }

    fn notes_of(world: &World, graph: &Graph, node: NodeId) -> Option<EventBatch<NoteEvent>> {
        let handle = read_field::<EventHandle<NoteEvent>>(graph, node, Part::Outlets, "notes");
        world.get_non_send::<EventArena>()?.read(handle)
    }

    fn handle_of(graph: &Graph, node: NodeId) -> EventHandle<NoteEvent> {
        read_field::<EventHandle<NoteEvent>>(graph, node, Part::Outlets, "notes")
    }

    /// Ticks a fresh graph holding one `MidiNotes` against `world`.
    fn tick_notes(world: &World) -> (Graph, NodeId) {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(MidiNotes::default()));
        tick(&mut graph, world);
        (graph, node)
    }

    #[test]
    fn a_note_on_then_a_note_off_publish_two_occurrences_in_that_order() {
        let world = world_with(vec![
            (
                0.001,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            ),
            (
                0.004,
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 40,
                },
            ),
        ]);

        let (graph, node) = tick_notes(&world);

        let batch = notes_of(&world, &graph, node).expect("a batch");
        assert_eq!(
            &*batch,
            &[
                NoteEvent {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                    on: true,
                    offset: 0.001,
                },
                NoteEvent {
                    channel: 0,
                    note: 60,
                    velocity: 40,
                    on: false,
                    offset: 0.004,
                },
            ],
            "arrival order, each carrying its channel, note, velocity and offset"
        );
    }

    #[test]
    fn a_zero_velocity_note_on_publishes_as_a_note_off() {
        let world = world_with(vec![(
            0.0,
            MidiMessage::NoteOn {
                channel: 3,
                note: 64,
                velocity: 0,
            },
        )]);

        let (graph, node) = tick_notes(&world);

        let batch = notes_of(&world, &graph, node).expect("a batch");
        assert_eq!(batch.len(), 1);
        assert!(!batch[0].on, "published as a note-off");
        assert_eq!(batch[0].note, 64);
    }

    #[test]
    fn every_channel_is_published() {
        let world = world_with(vec![
            (
                0.0,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 10,
                },
            ),
            (
                0.002,
                MidiMessage::NoteOn {
                    channel: 9,
                    note: 36,
                    velocity: 127,
                },
            ),
        ]);

        let (graph, node) = tick_notes(&world);

        let batch = notes_of(&world, &graph, node).expect("a batch");
        assert_eq!(
            batch.iter().map(|note| note.channel).collect::<Vec<_>>(),
            vec![0, 9],
            "the node selects nothing: every note message, on every channel"
        );
    }

    #[test]
    fn a_tick_with_no_note_messages_leaves_the_empty_handle() {
        let world = world_with(Vec::new());

        let (graph, node) = tick_notes(&world);

        assert_eq!(handle_of(&graph, node), EventHandle::EMPTY);
        assert!(notes_of(&world, &graph, node).is_none());
    }

    #[test]
    fn a_tick_of_only_control_and_clock_messages_leaves_the_empty_handle() {
        let world = world_with(vec![
            (
                0.0,
                MidiMessage::Control {
                    channel: 0,
                    cc: 1,
                    value: 64,
                },
            ),
            (0.001, MidiMessage::Clock),
            (0.002, MidiMessage::Start),
        ]);

        let (graph, node) = tick_notes(&world);

        assert_eq!(
            handle_of(&graph, node),
            EventHandle::EMPTY,
            "an empty batch is the empty handle, so nothing downstream dirties"
        );
    }

    #[test]
    fn no_arena_leaves_the_empty_handle_rather_than_failing() {
        let mut world = trace_world(registry());
        world.insert_resource(TickMidi {
            events: vec![(
                0.0,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            )],
        });

        let (graph, node) = tick_notes(&world);

        assert_eq!(handle_of(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn no_tick_midi_leaves_the_empty_handle_rather_than_failing() {
        // No MIDI input present at all: the evaluation still succeeds.
        let mut world = trace_world(registry());
        world.insert_non_send(EventArena::default());

        let (graph, node) = tick_notes(&world);

        assert_eq!(handle_of(&graph, node), EventHandle::EMPTY);
    }
}
