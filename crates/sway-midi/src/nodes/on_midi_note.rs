//! `OnMidiNote`: MIDI note occurrences into generic pressed / released Triggers.
//!
//! This is the converter that crosses the MIDI-vocabulary boundary (design
//! D3). It reads [`NoteEvent`] and speaks only [`Trigger`]. Channel filtering
//! is [`MidiNotes`]'s job; this node matches a scientific-pitch name.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
use sway_base_nodes::Trigger;
use sway_events::{EventArena, EventHandle};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::NoteEvent;

/// Scientific pitch to a MIDI note number, or `None` if the name does not
/// parse or names a number outside 0–127.
///
/// Letter A–G (case-insensitive), optional `#` or `b`, integer octave
/// (negative allowed). Surrounding whitespace is trimmed. MIDI 60 is `C4`;
/// `D#1` and `Eb1` are the same number; `C-1` is 0.
pub fn parse_note_name(name: &str) -> Option<u8> {
    let trimmed = name.trim();
    let mut chars = trimmed.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let semitone = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (accidental, octave_str) = if let Some(stripped) = rest.strip_prefix('#') {
        (1, stripped)
    } else if let Some(stripped) = rest.strip_prefix('b') {
        (-1, stripped)
    } else {
        (0, rest.as_str())
    };
    let octave: i32 = octave_str.parse().ok()?;
    let midi = (octave + 1) * 12 + semitone + accidental;
    u8::try_from(midi).ok().filter(|&n| n <= 127)
}

/// [`OnMidiNote`]'s inlets. No channel: which channel is heard is the
/// producing [`MidiNotes`](crate::MidiNotes) node's concern.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct OnMidiNoteIn {
    pub notes: EventHandle<NoteEvent>,
    /// Scientific pitch; default `C4`.
    pub note: String,
}

impl Default for OnMidiNoteIn {
    fn default() -> Self {
        Self {
            notes: EventHandle::EMPTY,
            note: "C4".to_string(),
        }
    }
}

/// [`OnMidiNote`]'s outlets — independent handles, each empty when that side
/// had no matching occurrence this tick.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct OnMidiNoteOut {
    pub pressed: EventHandle<Trigger>,
    pub released: EventHandle<Trigger>,
}

/// Converts matching note-ons and note-offs into generic Triggers.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct OnMidiNote {
    pub inlets: OnMidiNoteIn,
    pub state: (),
    pub outlets: OnMidiNoteOut,
}

impl NodeKind for OnMidiNote {
    fn evaluate(&mut self, world: &World) {
        let Some(arena) = world.get_non_send::<EventArena>() else {
            self.outlets.pressed = EventHandle::EMPTY;
            self.outlets.released = EventHandle::EMPTY;
            return;
        };
        let Some(midi) = parse_note_name(&self.inlets.note) else {
            self.outlets.pressed = EventHandle::EMPTY;
            self.outlets.released = EventHandle::EMPTY;
            return;
        };
        let Some(batch) = arena.read(self.inlets.notes) else {
            self.outlets.pressed = EventHandle::EMPTY;
            self.outlets.released = EventHandle::EMPTY;
            return;
        };

        let pressed = batch
            .iter()
            .filter(|event| event.on && event.note == midi)
            .map(|_| Trigger);
        let released = batch
            .iter()
            .filter(|event| !event.on && event.note == midi)
            .map(|_| Trigger);

        self.outlets.pressed = arena.publish(pressed);
        self.outlets.released = arena.publish(released);
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::TypeRegistry;
    use sway_base_nodes::{Timer, TimerIn};
    use sway_events::{EventArena, register_event_handle};
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing::{read_field, set_field, tick_once as tick, trace_world};
    use sway_graph::graph::{Graph, Node, NodeId, Part, Port};

    use super::*;
    use crate::nodes::midi_notes::{MidiNotes, MidiNotesIn, MidiNotesOut};
    use crate::{MidiMessage, TickMidi};

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<OnMidiNote>(&mut registry);
        register_node_kind::<MidiNotes>(&mut registry);
        register_node_kind::<Timer>(&mut registry);
        registry.register::<OnMidiNoteIn>();
        registry.register::<OnMidiNoteOut>();
        registry.register::<MidiNotesIn>();
        registry.register::<MidiNotesOut>();
        registry.register::<NoteEvent>();
        registry.register::<Trigger>();
        register_event_handle::<NoteEvent>(&mut registry);
        register_event_handle::<Trigger>(&mut registry);
        registry
    }

    fn world_with_arena() -> bevy_ecs::world::World {
        let mut world = trace_world(registry());
        world.insert_non_send(EventArena::default());
        world
    }

    fn note_on(note: u8) -> NoteEvent {
        NoteEvent {
            channel: 0,
            note,
            velocity: 100,
            on: true,
            offset: 0.0,
        }
    }

    fn note_off(note: u8) -> NoteEvent {
        NoteEvent {
            channel: 0,
            note,
            velocity: 40,
            on: false,
            offset: 0.0,
        }
    }

    fn tick_converter(
        world: &bevy_ecs::world::World,
        note: &str,
        events: impl IntoIterator<Item = NoteEvent>,
    ) -> (Graph, NodeId) {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(OnMidiNote {
            inlets: OnMidiNoteIn {
                note: note.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }));
        let handle = world
            .get_non_send::<EventArena>()
            .expect("arena")
            .publish(events);
        set_field(&mut graph, node, "notes", &handle);
        tick(&mut graph, world);
        (graph, node)
    }

    fn pressed_len(world: &bevy_ecs::world::World, graph: &Graph, node: NodeId) -> usize {
        let handle = read_field::<EventHandle<Trigger>>(graph, node, Part::Outlets, "pressed");
        world
            .get_non_send::<EventArena>()
            .and_then(|arena| arena.read(handle))
            .map(|batch| batch.len())
            .unwrap_or(0)
    }

    fn released_len(world: &bevy_ecs::world::World, graph: &Graph, node: NodeId) -> usize {
        let handle = read_field::<EventHandle<Trigger>>(graph, node, Part::Outlets, "released");
        world
            .get_non_send::<EventArena>()
            .and_then(|arena| arena.read(handle))
            .map(|batch| batch.len())
            .unwrap_or(0)
    }

    fn pressed_handle(graph: &Graph, node: NodeId) -> EventHandle<Trigger> {
        read_field::<EventHandle<Trigger>>(graph, node, Part::Outlets, "pressed")
    }

    fn released_handle(graph: &Graph, node: NodeId) -> EventHandle<Trigger> {
        read_field::<EventHandle<Trigger>>(graph, node, Part::Outlets, "released")
    }

    #[test]
    fn parse_note_name_covers_the_specified_cases() {
        assert_eq!(parse_note_name("C4"), Some(60));
        assert_eq!(parse_note_name("c4"), Some(60));
        assert_eq!(parse_note_name("  C4  "), Some(60));
        assert_eq!(parse_note_name("D#1"), parse_note_name("Eb1"));
        assert_eq!(parse_note_name("D#1"), Some(27));
        assert_eq!(parse_note_name("C-1"), Some(0));
        assert_eq!(parse_note_name("not-a-note"), None);
        assert_eq!(parse_note_name("G#9"), None, "MIDI 128 is out of range");
    }

    #[test]
    fn a_matching_note_on_fires_pressed() {
        let world = world_with_arena();
        let (graph, node) = tick_converter(&world, "C4", [note_on(60)]);

        assert_eq!(pressed_len(&world, &graph, node), 1);
        assert_eq!(released_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn a_matching_note_off_fires_released() {
        let world = world_with_arena();
        let (graph, node) = tick_converter(&world, "C4", [note_off(60)]);

        assert_eq!(released_len(&world, &graph, node), 1);
        assert_eq!(pressed_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn unmatched_notes_are_ignored() {
        let world = world_with_arena();
        let (graph, node) = tick_converter(&world, "C4", [note_on(64)]);

        assert_eq!(pressed_handle(&graph, node), EventHandle::EMPTY);
        assert_eq!(released_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn a_sharp_name_matches_that_pitch() {
        let world = world_with_arena();
        let midi = parse_note_name("D#1").expect("D#1");
        let (graph, node) = tick_converter(&world, "D#1", [note_on(midi)]);

        assert_eq!(pressed_len(&world, &graph, node), 1);
    }

    #[test]
    fn two_matching_note_ons_both_fire() {
        let world = world_with_arena();
        let (graph, node) = tick_converter(&world, "C4", [note_on(60), note_on(60)]);

        assert_eq!(pressed_len(&world, &graph, node), 2);
    }

    #[test]
    fn an_unparseable_note_name_is_silent() {
        let world = world_with_arena();
        let (graph, node) = tick_converter(&world, "not-a-note", [note_on(60)]);

        assert_eq!(pressed_handle(&graph, node), EventHandle::EMPTY);
        assert_eq!(released_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn an_unconnected_inlet_is_silent() {
        let world = world_with_arena();
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(OnMidiNote::default()));
        tick(&mut graph, &world);

        assert_eq!(pressed_handle(&graph, node), EventHandle::EMPTY);
        assert_eq!(released_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn no_arena_is_silent_rather_than_failing() {
        let world = trace_world(registry());
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(OnMidiNote::default()));
        tick(&mut graph, &world);

        assert_eq!(pressed_handle(&graph, node), EventHandle::EMPTY);
        assert_eq!(released_handle(&graph, node), EventHandle::EMPTY);
    }

    #[test]
    fn midi_notes_on_midi_note_timer_chain_resets_in_the_same_tick() {
        let mut world = trace_world(registry());
        world.insert_non_send(EventArena::default());
        world.insert_resource(TickMidi { events: Vec::new() });

        let mut graph = Graph::default();
        let notes = graph.insert(Node::of(MidiNotes {
            inlets: MidiNotesIn { channel: 0.0 },
            ..Default::default()
        }));
        let converter = graph.insert(Node::of(OnMidiNote {
            inlets: OnMidiNoteIn {
                note: "C4".to_string(),
                ..Default::default()
            },
            ..Default::default()
        }));
        let timer = graph.insert(Node::of(Timer {
            inlets: TimerIn {
                time: 0.0,
                trigger: Vec::new(),
            },
            ..Default::default()
        }));
        graph
            .connect(Port::new(notes, "notes"), Port::new(converter, "notes"), 0)
            .expect("legal");
        graph
            .connect(
                Port::new(converter, "pressed"),
                Port::new(timer, "trigger"),
                0,
            )
            .expect("legal");

        tick(&mut graph, &world);
        set_field(&mut graph, timer, "time", &3.0_f32);
        tick(&mut graph, &world);
        assert_eq!(read_field::<f32>(&graph, timer, Part::Outlets, "out"), 3.0);

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
        tick(&mut graph, &world);
        assert_eq!(
            read_field::<f32>(&graph, timer, Part::Outlets, "out"),
            0.0,
            "a matching note-on zeros the Timer in the same tick"
        );
    }
}
