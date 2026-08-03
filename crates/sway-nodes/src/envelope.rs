//! Envelope — ADSR as a pure function of absolute gate times (spec §6, §8).

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::{Events, NodeType, PortView, TickCtx, register_events};

use crate::NoteMsg;

#[derive(Reflect, Component, Default)]
pub struct EnvelopeInlets {
    pub triggers: Vec<Events<NoteMsg>>,
    pub release_triggers: Vec<Events<NoteMsg>>,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

#[derive(Reflect, Default)]
pub struct EnvelopeOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct EnvelopeState {
    pub gate_on: Option<f64>,
    pub gate_off: Option<f64>,
    pub velocity: f32,
}

pub struct Envelope;

impl Envelope {
    pub const TRIGGERS: u16 = 0;
    pub const RELEASE_TRIGGERS: u16 = 1;
    pub const ATTACK: u16 = 2;
    pub const DECAY: u16 = 3;
    pub const SUSTAIN: u16 = 4;
    pub const RELEASE: u16 = 5;
    pub const OUT_VALUE: u16 = 6;
}

impl NodeType for Envelope {
    type Inlets = EnvelopeInlets;
    type Outlets = EnvelopeOutlets;
    type State = EnvelopeState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("triggers", Self::TRIGGERS),
        ("release_triggers", Self::RELEASE_TRIGGERS),
        ("attack", Self::ATTACK),
        ("decay", Self::DECAY),
        ("sustain", Self::SUSTAIN),
        ("release", Self::RELEASE),
        ("value", Self::OUT_VALUE),
    ];

    fn register(app: &mut App) {
        register_events::<NoteMsg>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, ctx: &TickCtx) {
        let attack: f32 = ports.read(Self::ATTACK);
        let decay: f32 = ports.read(Self::DECAY);
        let sustain: f32 = ports.read(Self::SUSTAIN);
        let release: f32 = ports.read(Self::RELEASE);

        let mut gate_events: Vec<(f32, bool, NoteMsg)> = merged(ports, Self::TRIGGERS)
            .into_iter()
            .map(|(offset, msg)| (offset, true, msg))
            .collect();
        gate_events.extend(
            merged(ports, Self::RELEASE_TRIGGERS)
                .into_iter()
                .map(|(offset, msg)| (offset, false, msg)),
        );
        gate_events.sort_by(|a, b| a.0.total_cmp(&b.0));

        {
            let mut state = world
                .get_mut::<EnvelopeState>(node)
                .expect("EnvelopeState on envelope node");
            for (offset, gate_on, msg) in gate_events {
                let t = ctx.tick_start + offset as f64;
                if gate_on {
                    state.gate_on = Some(t);
                    state.gate_off = None;
                    state.velocity = msg.velocity as f32 / 127.0;
                } else {
                    state.gate_off = Some(t);
                }
            }
        }

        let state = world
            .get::<EnvelopeState>(node)
            .expect("EnvelopeState on envelope node");
        let now = ctx.tick_start + ctx.dt as f64;
        let value = match state.gate_on {
            None => 0.0,
            Some(gate_on) => {
                adsr_unscaled(
                    gate_on,
                    state.gate_off,
                    now,
                    attack,
                    decay,
                    sustain,
                    release,
                ) * state.velocity
            }
        };
        ports.write(Self::OUT_VALUE, value);
    }
}

/// Merges this node's trigger elements into one offset-ordered stream.
///
/// The engine used to do this, ordering sources by compiled rank and stable
/// sorting by offset. Element order now plays the part compiled rank did, and
/// the sort is still stable, so equal offsets resolve by element index —
/// which is what keeps the `event-fan-in` golden trace bit-identical.
fn merged(ports: &PortView, field: u16) -> Vec<(f32, NoteMsg)> {
    let mut merged: Vec<(f32, NoteMsg)> = Vec::new();
    for index in 0..ports.len(field) {
        for occurrence in ports.events_at::<NoteMsg>(field, index as u16) {
            merged.push((occurrence.offset, occurrence.value.clone()));
        }
    }
    merged.sort_by(|a, b| a.0.total_cmp(&b.0));
    merged
}

fn adsr_unscaled(
    gate_on: f64,
    gate_off: Option<f64>,
    now: f64,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
) -> f32 {
    let attack = attack.max(0.0);
    let decay = decay.max(0.0);
    let release = release.max(0.0);
    let level_while_gated = |t: f64| -> f32 {
        let elapsed = (t - gate_on) as f32;
        if elapsed < 0.0 {
            return 0.0;
        }
        if attack == 0.0 {
            if decay == 0.0 {
                return sustain;
            }
            if elapsed < decay {
                return 1.0 - (1.0 - sustain) * (elapsed / decay);
            }
            return sustain;
        }
        if elapsed < attack {
            return elapsed / attack;
        }
        let after_attack = elapsed - attack;
        if decay == 0.0 {
            return sustain;
        }
        if after_attack < decay {
            return 1.0 - (1.0 - sustain) * (after_attack / decay);
        }
        sustain
    };

    match gate_off {
        None => level_while_gated(now),
        Some(off) if now <= off => level_while_gated(now),
        Some(off) => {
            let start = level_while_gated(off);
            let elapsed = (now - off) as f32;
            if release == 0.0 || elapsed >= release {
                return 0.0;
            }
            start * (1.0 - elapsed / release)
        }
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
        NodeType, NodeTypeRegistry, PortArena,
    };

    use crate::{MidiInbox, MidiNote, MidiNoteInlets, MidiNoteState, RawMidi, SignalNodesPlugin};

    use super::*;

    const TICK_HZ: f64 = 120.0;

    #[derive(Resource)]
    struct EnvelopeNode(Entity);

    fn note_on(note: u8, velocity: u8) -> RawMidi {
        RawMidi {
            status: 0x90,
            data1: note,
            data2: velocity,
        }
    }

    fn node_type_id<N: NodeType>(app: &App) -> sway_graph::NodeTypeId {
        app.world()
            .resource::<NodeTypeRegistry>()
            .id_of(core::any::type_name::<N>())
            .expect("node type registered")
    }

    fn connect(app: &mut App, from: Entity, from_field: u16, to: Entity, to_field: u16, to_index: u16) {
        app.world_mut().spawn((
            Edge {
                from: Endpoint::field(from_field),
                to: Endpoint { field: to_field, index: to_index },
            },
            EdgeFrom(from),
            EdgeTo(to),
        ));
    }

    fn envelope_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, SignalNodesPlugin));
        app.update();

        let midi_type = node_type_id::<MidiNote>(&app);
        let envelope_type = node_type_id::<Envelope>(&app);
        let midi = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(0),
                    node_type: midi_type,
                },
                MidiNoteInlets {
                    channel: 0,
                    note_lo: 0,
                    note_hi: 127,
                },
                MidiNoteState,
            ))
            .id();
        let envelope = app
            .world_mut()
            .spawn((
                GraphNode {
                    id: NodeId(1),
                    node_type: envelope_type,
                },
                EnvelopeInlets {
                    triggers: vec![Events::default()],
                    release_triggers: vec![Events::default()],
                    attack: 0.05,
                    decay: 0.01,
                    sustain: 0.4,
                    release: 0.05,
                },
                EnvelopeState::default(),
            ))
            .id();
        connect(&mut app, midi, MidiNote::OUT_NOTE_ON, envelope, Envelope::TRIGGERS, 0);
        connect(
            &mut app,
            midi,
            MidiNote::OUT_NOTE_OFF,
            envelope,
            Envelope::RELEASE_TRIGGERS,
            0,
        );
        app.world_mut().insert_resource(EnvelopeNode(envelope));

        let compiled = compile(app.world_mut()).expect("envelope graph compiles");
        let slots_len = compiled.slots_len;
        app.world_mut().resource_mut::<PortArena>().resize(slots_len);
        app.world_mut().insert_resource(compiled);
        app
    }

    fn envelope_value(app: &App) -> f32 {
        let node = app.world().resource::<EnvelopeNode>().0;
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("compiled");
        let slot = plan.base + plan.field_offsets[Envelope::OUT_VALUE as usize];
        *app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<f32>()
            .expect("envelope output is f32")
    }

    #[test]
    fn two_notes_in_one_tick_at_different_offsets_give_different_envelope_values() {
        let mut app = envelope_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0001, note_on(60, 127));
        app.update();
        let early = envelope_value(&app);

        let mut app = envelope_app();
        app.world_mut()
            .resource_mut::<MidiInbox>()
            .push(0.0080, note_on(60, 127));
        app.update();
        let late = envelope_value(&app);

        assert!(
            early > late,
            "earlier note is further into its attack: {early} vs {late}"
        );
        assert!(
            (early - late).abs() > 1e-4,
            "difference must be real, not float noise"
        );
    }

    #[test]
    fn envelope_reaches_sustain_and_releases_from_its_gate_off_level() {
        let mut app = envelope_app();
        let node = app.world().resource::<EnvelopeNode>().0;
        {
            let mut state = app.world_mut().get_mut::<EnvelopeState>(node).unwrap();
            state.gate_on = Some(-1.0);
            state.velocity = 1.0;
        }
        app.update();
        assert!((envelope_value(&app) - 0.4).abs() < 1e-5);

        let gate_off = app.world().resource::<Time<Fixed>>().elapsed_secs_f64();
        app.world_mut()
            .get_mut::<EnvelopeState>(node)
            .unwrap()
            .gate_off = Some(gate_off);
        app.update();
        let releasing = envelope_value(&app);
        assert!(
            releasing > 0.0 && releasing < 0.4,
            "release must descend from sustain: {releasing}"
        );

        for _ in 0..8 {
            app.update();
        }
        assert_eq!(envelope_value(&app), 0.0);
    }
}
