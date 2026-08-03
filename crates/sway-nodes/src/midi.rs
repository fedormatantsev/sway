use std::collections::VecDeque;

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time};
use sway_graph::{Events, NodeType, PortView, TickCtx, graph_tick, register_events, register_node_type};

use crate::{Envelope, LFO, Math, Remap, Select, Switch};

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
    pub fn push(&mut self, t: f64, m: RawMidi) {
        self.events.push_back((t, m));
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
            let offset = (event_time - tick_start).clamp(0.0, dt as f64) as f32;
            tick_midi.events.push((offset, message));
            false
        } else {
            true
        }
    });
}

#[derive(Reflect, Component, Default)]
pub struct MidiNoteInlets {
    pub channel: u8,
    pub note_lo: u8,
    pub note_hi: u8,
}

#[derive(Reflect, Default)]
pub struct MidiNoteOutlets {
    pub note_on: Events<NoteMsg>,
    pub note_off: Events<NoteMsg>,
}

#[derive(Component, Default)]
pub struct MidiNoteState;

pub struct MidiNote;

impl MidiNote {
    pub const CHANNEL: u16 = 0;
    pub const NOTE_LO: u16 = 1;
    pub const NOTE_HI: u16 = 2;
    pub const OUT_NOTE_ON: u16 = 3;
    pub const OUT_NOTE_OFF: u16 = 4;
}

impl NodeType for MidiNote {
    type Inlets = MidiNoteInlets;
    type Outlets = MidiNoteOutlets;
    type State = MidiNoteState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("channel", Self::CHANNEL),
        ("note_lo", Self::NOTE_LO),
        ("note_hi", Self::NOTE_HI),
        ("note_on", Self::OUT_NOTE_ON),
        ("note_off", Self::OUT_NOTE_OFF),
    ];

    fn register(app: &mut App) {
        register_events::<NoteMsg>(app);
    }

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let channel: u8 = ports.read(Self::CHANNEL);
        let note_lo: u8 = ports.read(Self::NOTE_LO);
        let note_hi: u8 = ports.read(Self::NOTE_HI);

        for &(offset, message) in &world.resource::<TickMidi>().events {
            if message.status & 0x0f != channel
                || message.data1 < note_lo
                || message.data1 > note_hi
            {
                continue;
            }

            let payload = NoteMsg {
                note: message.data1,
                velocity: message.data2,
            };
            match (message.status & 0xf0, message.data2) {
                (0x90, velocity) if velocity > 0 => {
                    ports.emit(Self::OUT_NOTE_ON, offset, payload);
                }
                (0x80, _) | (0x90, 0) => {
                    ports.emit(Self::OUT_NOTE_OFF, offset, payload);
                }
                _ => {}
            }
        }
    }
}

#[derive(Reflect, Component, Default)]
pub struct MidiCCInlets {
    pub channel: u8,
    pub cc: u8,
}

#[derive(Reflect, Default)]
pub struct MidiCCOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct MidiCCState;

pub struct MidiCC;

impl MidiCC {
    pub const CHANNEL: u16 = 0;
    pub const CC: u16 = 1;
    pub const OUT_VALUE: u16 = 2;
}

impl NodeType for MidiCC {
    type Inlets = MidiCCInlets;
    type Outlets = MidiCCOutlets;
    type State = MidiCCState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("channel", Self::CHANNEL),
        ("cc", Self::CC),
        ("value", Self::OUT_VALUE),
    ];

    fn register(_app: &mut App) {}

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let channel: u8 = ports.read(Self::CHANNEL);
        let cc: u8 = ports.read(Self::CC);
        let value = world
            .resource::<TickMidi>()
            .events
            .iter()
            .rev()
            .find(|(_, message)| {
                message.status & 0xf0 == 0xb0
                    && message.status & 0x0f == channel
                    && message.data1 == cc
            })
            .map(|(_, message)| message.data2 as f32 / 127.0);

        if let Some(value) = value {
            ports.write(Self::OUT_VALUE, value);
        }
    }
}

pub struct SignalNodesPlugin;

impl Plugin for SignalNodesPlugin {
    fn build(&self, app: &mut App) {
        register_node_type::<MidiNote>(app);
        register_node_type::<MidiCC>(app);
        register_node_type::<LFO>(app);
        register_node_type::<Envelope>(app);
        register_node_type::<Math>(app);
        register_node_type::<Remap>(app);
        register_node_type::<Switch>(app);
        register_node_type::<Select>(app);
        app.init_resource::<MidiInbox>()
            .init_resource::<TickMidi>()
            .add_systems(FixedUpdate, drain_inbox.before(graph_tick));
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::entity::Entity;
    use bevy_ecs::resource::Resource;
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use sway_graph::{
        compile, CompiledGraph, Edge, EdgeFrom, EdgeTo, Endpoint, GraphNode, GraphPlugin, NodeId,
        NodeType, NodeTypeRegistry, Occurrence, PortArena,
    };

    use super::*;
    use crate::{RemapInlets, RemapState};

    const TICK_HZ: f64 = 120.0;

    #[derive(Resource)]
    struct TestNode(Entity);

    fn note_on(note: u8, velocity: u8) -> RawMidi {
        RawMidi {
            status: 0x90,
            data1: note,
            data2: velocity,
        }
    }

    fn midi_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, SignalNodesPlugin));
        app.update();
        app
    }

    fn node_type_id<N: NodeType>(app: &App) -> sway_graph::NodeTypeId {
        app.world()
            .resource::<NodeTypeRegistry>()
            .id_of(core::any::type_name::<N>())
            .expect("node type registered by SignalNodesPlugin")
    }

    fn connect(app: &mut App, from: Entity, from_field: u16, to: Entity, to_field: u16) -> Entity {
        app.world_mut()
            .spawn((
                Edge {
                    from: Endpoint::field(from_field),
                    to: Endpoint::field(to_field),
                },
                EdgeFrom(from),
                EdgeTo(to),
            ))
            .id()
    }

    fn compile_graph(app: &mut App) {
        let compiled = compile(app.world_mut()).expect("compiles");
        let slots_len = compiled.slots_len;
        app.world_mut().resource_mut::<PortArena>().resize(slots_len);
        app.world_mut().insert_resource(compiled);
    }

    fn midi_app_with_node() -> App {
        let mut app = midi_app();
        let node_type = node_type_id::<MidiNote>(&app);
        let node = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(0),
                    node_type,
                },
                MidiNoteInlets {
                    channel: 0,
                    note_lo: 60,
                    note_hi: 72,
                },
                MidiNoteState,
            ))
            .id();
        app.world_mut().insert_resource(TestNode(node));
        compile_graph(&mut app);
        app
    }

    fn midi_app_with_cc() -> App {
        let mut app = midi_app();
        let node_type = node_type_id::<MidiCC>(&app);
        let node = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(0),
                    node_type,
                },
                MidiCCInlets { channel: 0, cc: 74 },
                MidiCCState,
            ))
            .id();
        app.world_mut().insert_resource(TestNode(node));
        compile_graph(&mut app);
        app
    }

    fn event_slot(app: &App, ordinal: u16) -> Vec<Occurrence<NoteMsg>> {
        let node = app.world().resource::<TestNode>().0;
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("node is compiled");
        let slot = plan.base + plan.field_offsets[ordinal as usize];
        app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<Events<NoteMsg>>()
            .expect("slot holds Events<NoteMsg>")
            .occurrences
            .clone()
    }

    fn note_on_count(app: &App) -> usize {
        event_slot(app, MidiNote::OUT_NOTE_ON).len()
    }

    fn note_off_count(app: &App) -> usize {
        event_slot(app, MidiNote::OUT_NOTE_OFF).len()
    }

    fn first_note_on(app: &App) -> NoteMsg {
        event_slot(app, MidiNote::OUT_NOTE_ON)[0].value.clone()
    }

    fn cc_value(app: &App) -> f32 {
        let node = app.world().resource::<TestNode>().0;
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("node is compiled");
        let slot = plan.base + plan.field_offsets[MidiCC::OUT_VALUE as usize];
        *app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<f32>()
            .expect("CC output is f32")
    }

    #[test]
    fn an_event_inside_the_window_gets_its_offset_and_one_past_it_waits() {
        let mut app = midi_app(); // 120Hz, one fixed tick per update
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.002, note_on(60, 100));
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.020, note_on(64, 100));

        app.update(); // window [0.0, 0.00833)

        let drained = &app.world().resource::<TickMidi>().events;
        assert_eq!(drained.len(), 1, "the 0.020 event belongs to a later tick");
        assert!((drained[0].0 - 0.002).abs() < 1e-6);
    }

    #[test]
    fn an_eligible_event_is_not_blocked_by_an_earlier_future_event() {
        let mut app = midi_app();
        let future = note_on(64, 100);
        let eligible = note_on(60, 100);
        let inbox = &mut app.world_mut().resource_mut::<MidiInbox>();
        inbox.push(0.020, future);
        inbox.push(0.002, eligible);

        app.update();

        let drained = &app.world().resource::<TickMidi>().events;
        assert_eq!(drained.len(), 1);
        assert!((drained[0].0 - 0.002).abs() < 1e-6);
        assert_eq!(drained[0].1, eligible);
        let buffered = &app.world().resource::<MidiInbox>().events;
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered.front(), Some(&(0.020, future)));
    }

    #[test]
    fn a_late_arrival_clamps_to_zero_rather_than_going_negative() {
        let mut app = midi_app();
        for _ in 0..3 {
            app.update();
        }
        // stamped before the current window began
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0, note_on(60, 100));
        app.update();
        let drained = &app.world().resource::<TickMidi>().events;
        assert_eq!(drained[0].0, 0.0, "clamped, not dropped and not negative");
    }

    #[test]
    fn note_on_with_zero_velocity_is_a_note_off() {
        // Many devices spell note-off that way — sway-app/src/graph.rs:44
        // already handled this and the behaviour must survive the move.
        let mut app = midi_app_with_node();
        app.world_mut().resource_mut::<MidiInbox>().push(
            0.001,
            RawMidi {
                status: 0x90,
                data1: 60,
                data2: 0,
            },
        );
        app.update();
        assert_eq!(note_on_count(&app), 0);
        assert_eq!(note_off_count(&app), 1);
    }

    #[test]
    fn the_channel_and_note_range_filters_reject_non_matching_events() {
        // MidiNote spawned with channel 0, note_lo 60, note_hi 72.
        let mut app = midi_app_with_node();
        let inbox = &mut app.world_mut().resource_mut::<MidiInbox>();
        inbox.push(
            0.001,
            RawMidi {
                status: 0x91,
                data1: 64,
                data2: 100,
            },
        ); // channel 1
        inbox.push(
            0.002,
            RawMidi {
                status: 0x90,
                data1: 48,
                data2: 100,
            },
        ); // below range
        inbox.push(
            0.003,
            RawMidi {
                status: 0x90,
                data1: 80,
                data2: 100,
            },
        ); // above range
        inbox.push(
            0.004,
            RawMidi {
                status: 0x90,
                data1: 64,
                data2: 100,
            },
        ); // matches

        app.update();

        assert_eq!(
            note_on_count(&app),
            1,
            "only the in-range, in-channel note passes"
        );
        assert_eq!(first_note_on(&app).note, 64);
    }

    #[test]
    fn midi_cc_holds_its_value_between_messages() {
        // The continuous/event distinction made observable: a CC with no new
        // message this tick still reads its last value, where an event port
        // would read empty (spec §4).
        let mut app = midi_app_with_cc();
        app.world_mut().resource_mut::<MidiInbox>().push(
            0.001,
            RawMidi {
                status: 0xB0,
                data1: 74,
                data2: 127,
            },
        );
        app.update();
        assert_eq!(cc_value(&app), 1.0);
        app.update();
        assert_eq!(cc_value(&app), 1.0, "held, not reset");
    }

    #[test]
    fn midi_cc_output_is_typed_before_the_first_matching_message() {
        let mut app = midi_app();
        let cc_type = node_type_id::<MidiCC>(&app);
        let remap_type = node_type_id::<Remap>(&app);
        let cc = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(0),
                    node_type: cc_type,
                },
                MidiCCInlets { channel: 0, cc: 74 },
                MidiCCState,
            ))
            .id();
        let remap = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(1),
                    node_type: remap_type,
                },
                RemapInlets {
                    in_max: 1.0,
                    out_max: 1.0,
                    ..Default::default()
                },
                RemapState,
            ))
            .id();
        connect(&mut app, cc, MidiCC::OUT_VALUE, remap, Remap::VALUE);
        compile_graph(&mut app);

        app.update();

        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == remap).expect("remap is compiled");
        let slot = plan.base + plan.field_offsets[Remap::VALUE as usize];
        assert_eq!(
            app.world().resource::<PortArena>().values[slot].try_downcast_ref::<f32>(),
            Some(&0.0),
        );
    }
}
