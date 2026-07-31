//! Envelope — ADSR as a pure function of absolute gate times (spec §6, §8).

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::{
    ContinuousIdx, Event, EventIdx, NodeType, PortView, TickCtx, register_event_port,
};

use crate::NoteMsg;

#[derive(Reflect, Component, Default)]
pub struct EnvelopeParams {
    pub trigger: Event<NoteMsg>,
    pub release_trigger: Event<NoteMsg>,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

#[derive(Reflect, Default)]
pub struct EnvelopeOutputs {
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
    pub const ATTACK: u16 = 0;
    pub const DECAY: u16 = 1;
    pub const SUSTAIN: u16 = 2;
    pub const RELEASE: u16 = 3;
    pub const OUT_VALUE: u16 = 4;
    pub const TRIGGER: u16 = 0;
    pub const RELEASE_TRIGGER: u16 = 1;
}

impl NodeType for Envelope {
    type Params = EnvelopeParams;
    type Outputs = EnvelopeOutputs;
    type State = EnvelopeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("attack", Self::ATTACK),
        ("decay", Self::DECAY),
        ("sustain", Self::SUSTAIN),
        ("release", Self::RELEASE),
        ("value", Self::OUT_VALUE),
        ("trigger", Self::TRIGGER),
        ("release_trigger", Self::RELEASE_TRIGGER),
    ];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, ctx: &TickCtx) {
        let attack: f32 = ports.read(ContinuousIdx(Self::ATTACK as u32));
        let decay: f32 = ports.read(ContinuousIdx(Self::DECAY as u32));
        let sustain: f32 = ports.read(ContinuousIdx(Self::SUSTAIN as u32));
        let release: f32 = ports.read(ContinuousIdx(Self::RELEASE as u32));

        let mut gate_events: Vec<(f32, bool, NoteMsg)> = ports
            .events::<NoteMsg>(EventIdx(Self::TRIGGER as u32))
            .map(|ev| (ev.offset, true, ev.value.clone()))
            .collect();
        gate_events.extend(
            ports
                .events::<NoteMsg>(EventIdx(Self::RELEASE_TRIGGER as u32))
                .map(|ev| (ev.offset, false, ev.value.clone())),
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
        ports.write(ContinuousIdx(Self::OUT_VALUE as u32), value);
    }
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
        EdgeFrom, EdgeTo, GraphNode, GraphPlugin, NodeId, NodeRuntime, NodeType, NodeTypeRegistry,
        ParamEdge, PortArena, PortKind, compile,
    };

    use crate::{MidiInbox, MidiNote, MidiNoteParams, MidiNoteState, RawMidi, SignalNodesPlugin};

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
                MidiNoteParams {
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
                EnvelopeParams {
                    trigger: Event::default(),
                    release_trigger: Event::default(),
                    attack: 0.05,
                    decay: 0.01,
                    sustain: 0.4,
                    release: 0.05,
                },
                EnvelopeState::default(),
            ))
            .id();
        app.world_mut().spawn((
            ParamEdge {
                source_port: MidiNote::OUT_NOTE_ON,
                target_port: Envelope::TRIGGER,
                kind: PortKind::Event,
            },
            EdgeFrom(midi),
            EdgeTo(envelope),
        ));
        app.world_mut().spawn((
            ParamEdge {
                source_port: MidiNote::OUT_NOTE_OFF,
                target_port: Envelope::RELEASE_TRIGGER,
                kind: PortKind::Event,
            },
            EdgeFrom(midi),
            EdgeTo(envelope),
        ));
        app.world_mut().insert_resource(EnvelopeNode(envelope));

        let compiled = compile(app.world_mut()).expect("envelope graph compiles");
        let (continuous_len, events_len) = (compiled.continuous_len, compiled.events_len);
        app.world_mut()
            .resource_mut::<PortArena>()
            .resize(continuous_len, events_len);
        app.world_mut().insert_resource(compiled);
        app
    }

    fn envelope_value(app: &App) -> f32 {
        let node = app.world().resource::<EnvelopeNode>().0;
        let base = app
            .world()
            .get::<NodeRuntime>(node)
            .expect("compiled")
            .continuous_base;
        *app.world().resource::<PortArena>().continuous[base + Envelope::OUT_VALUE as usize]
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
