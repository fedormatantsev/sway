use std::fs;
use std::path::PathBuf;

use bevy_app::App;
use bevy_ecs::entity::Entity;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use serde::{Deserialize, Serialize};
use sway_graph::{
    EdgeFrom, EdgeTo, Event, GraphNode, GraphPlugin, NodeId, NodeRuntime, NodeType,
    NodeTypeRegistry, ParamEdge, PortArena, PortKind, compile,
};
use sway_nodes::{
    Envelope, EnvelopeParams, EnvelopeState, LFO, LfoParams, LfoState, Math, MathOp, MathParams,
    MathState, MidiCC, MidiCCParams, MidiCCState, MidiInbox, MidiNote, MidiNoteParams,
    MidiNoteState, NoteMsg, RawMidi, Remap, RemapParams, RemapState, SignalNodesPlugin, Waveform,
};

#[derive(Debug, Deserialize)]
struct TraceInput {
    tick_hz: f64,
    ticks: u32,
    events: Vec<(f64, MidiEvent)>,
}

#[derive(Debug, Deserialize)]
struct MidiEvent {
    status: u8,
    data1: u8,
    data2: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TraceOutput {
    ports: Vec<String>,
    ticks: Vec<(u32, Vec<Snapshot>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum Snapshot {
    Continuous(f32),
    Events(Vec<(f32, String)>),
}

#[derive(Clone, Copy)]
enum PortKindSpec {
    Continuous(u16),
    NoteEvents(u16, &'static str),
}

struct TracedPort {
    label: &'static str,
    node: Entity,
    kind: PortKindSpec,
}

fn trace_path(name: &str, suffix: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("traces")
        .join(format!("{name}.{suffix}.ron"))
}

fn load_input(name: &str) -> TraceInput {
    let path = trace_path(name, "in");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    ron::from_str(&source).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn node_type_id<N: NodeType>(app: &App) -> sway_graph::NodeTypeId {
    app.world()
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered by SignalNodesPlugin")
}

fn connect(
    app: &mut App,
    source: Entity,
    source_port: u16,
    target: Entity,
    target_port: u16,
    kind: PortKind,
) {
    app.world_mut().spawn((
        ParamEdge {
            source_port,
            target_port,
            kind,
        },
        EdgeFrom(source),
        EdgeTo(target),
    ));
}

fn compile_graph(app: &mut App) {
    let graph = compile(app.world_mut()).expect("trace graph compiles");
    let (continuous_len, events_len) = (graph.continuous_len, graph.events_len);
    app.world_mut()
        .resource_mut::<PortArena>()
        .resize(continuous_len, events_len);
    app.world_mut().insert_resource(graph);
}

fn spawn_midi_note(app: &mut App, id: u32, channel: u8, note_lo: u8, note_hi: u8) -> Entity {
    let midi_type = node_type_id::<MidiNote>(app);
    app.world_mut()
        .spawn((
            GraphNode {
                id: NodeId(id),
                node_type: midi_type,
            },
            MidiNoteParams {
                channel,
                note_lo,
                note_hi,
            },
            MidiNoteState,
        ))
        .id()
}

fn spawn_envelope(app: &mut App, id: u32) -> Entity {
    let envelope_type = node_type_id::<Envelope>(app);
    app.world_mut()
        .spawn((
            GraphNode {
                id: NodeId(id),
                node_type: envelope_type,
            },
            EnvelopeParams {
                trigger: Event::default(),
                release_trigger: Event::default(),
                attack: 0.05,
                decay: 0.08,
                sustain: 0.4,
                release: 0.1,
            },
            EnvelopeState::default(),
        ))
        .id()
}

fn build_midi_envelope(app: &mut App, trace_note_events: bool) -> Vec<TracedPort> {
    let midi = spawn_midi_note(app, 0, 0, 0, 127);
    let envelope = spawn_envelope(app, 1);
    connect(
        app,
        midi,
        MidiNote::OUT_NOTE_ON,
        envelope,
        Envelope::TRIGGER,
        PortKind::Event,
    );
    connect(
        app,
        midi,
        MidiNote::OUT_NOTE_OFF,
        envelope,
        Envelope::RELEASE_TRIGGER,
        PortKind::Event,
    );
    compile_graph(app);
    let mut ports = vec![TracedPort {
        label: "envelope.value",
        node: envelope,
        kind: PortKindSpec::Continuous(Envelope::OUT_VALUE),
    }];
    if trace_note_events {
        ports.push(TracedPort {
            label: "midinote.note_on",
            node: midi,
            kind: PortKindSpec::NoteEvents(MidiNote::OUT_NOTE_ON, "note_on"),
        });
    }
    ports
}

fn build_envelope_retrigger(app: &mut App) -> Vec<TracedPort> {
    build_midi_envelope(app, true)
}

fn build_lfo_one_cycle(app: &mut App) -> Vec<TracedPort> {
    let lfo_type = node_type_id::<LFO>(app);
    let lfo = app
        .world_mut()
        .spawn((
            GraphNode {
                id: NodeId(0),
                node_type: lfo_type,
            },
            LfoParams {
                hz: 2.0,
                shape: Waveform::Sine,
                phase: 0.0,
                amplitude: 1.0,
            },
            LfoState,
        ))
        .id();
    compile_graph(app);
    vec![TracedPort {
        label: "lfo.value",
        node: lfo,
        kind: PortKindSpec::Continuous(LFO::OUT_VALUE),
    }]
}

fn build_cc_hold(app: &mut App) -> Vec<TracedPort> {
    let cc_type = node_type_id::<MidiCC>(app);
    let cc = app
        .world_mut()
        .spawn((
            GraphNode {
                id: NodeId(0),
                node_type: cc_type,
            },
            MidiCCParams { channel: 0, cc: 74 },
            MidiCCState,
        ))
        .id();
    compile_graph(app);
    vec![TracedPort {
        label: "midicc.value",
        node: cc,
        kind: PortKindSpec::Continuous(MidiCC::OUT_VALUE),
    }]
}

fn build_chain_math_remap(app: &mut App) -> Vec<TracedPort> {
    let lfo_type = node_type_id::<LFO>(app);
    let math_type = node_type_id::<Math>(app);
    let remap_type = node_type_id::<Remap>(app);
    let lfo = app
        .world_mut()
        .spawn((
            GraphNode {
                id: NodeId(0),
                node_type: lfo_type,
            },
            LfoParams {
                hz: 1.0,
                shape: Waveform::Sine,
                phase: 0.0,
                amplitude: 1.0,
            },
            LfoState,
        ))
        .id();
    let math = app
        .world_mut()
        .spawn((
            GraphNode {
                id: NodeId(1),
                node_type: math_type,
            },
            MathParams {
                op: MathOp::Add,
                a: 0.0,
                b: 1.0,
            },
            MathState,
        ))
        .id();
    let remap = app
        .world_mut()
        .spawn((
            GraphNode {
                id: NodeId(2),
                node_type: remap_type,
            },
            RemapParams {
                value: 0.0,
                in_min: 0.0,
                in_max: 2.0,
                out_min: -1.0,
                out_max: 1.0,
                clamp: true,
            },
            RemapState,
        ))
        .id();
    connect(
        app,
        lfo,
        LFO::OUT_VALUE,
        math,
        Math::A,
        PortKind::Continuous,
    );
    connect(
        app,
        math,
        Math::OUT_VALUE,
        remap,
        Remap::VALUE,
        PortKind::Continuous,
    );
    compile_graph(app);
    vec![
        TracedPort {
            label: "lfo.value",
            node: lfo,
            kind: PortKindSpec::Continuous(LFO::OUT_VALUE),
        },
        TracedPort {
            label: "math.value",
            node: math,
            kind: PortKindSpec::Continuous(Math::OUT_VALUE),
        },
        TracedPort {
            label: "remap.value",
            node: remap,
            kind: PortKindSpec::Continuous(Remap::OUT_VALUE),
        },
    ]
}

fn build_two_notes_one_tick(app: &mut App) -> Vec<TracedPort> {
    build_midi_envelope(app, true)
}

fn build_event_fan_in(app: &mut App) -> Vec<TracedPort> {
    let midi_a = spawn_midi_note(app, 0, 0, 0, 127);
    let midi_b = spawn_midi_note(app, 1, 1, 0, 127);
    let envelope = spawn_envelope(app, 2);
    for midi in [midi_a, midi_b] {
        connect(
            app,
            midi,
            MidiNote::OUT_NOTE_ON,
            envelope,
            Envelope::TRIGGER,
            PortKind::Event,
        );
    }
    compile_graph(app);
    vec![
        TracedPort {
            label: "envelope.value",
            node: envelope,
            kind: PortKindSpec::Continuous(Envelope::OUT_VALUE),
        },
        TracedPort {
            label: "envelope.trigger",
            node: envelope,
            kind: PortKindSpec::NoteEvents(Envelope::TRIGGER, "note_on"),
        },
    ]
}

fn snapshot_port(app: &App, port: &TracedPort) -> Snapshot {
    let runtime = app
        .world()
        .get::<NodeRuntime>(port.node)
        .expect("traced node is compiled");
    let arena = app.world().resource::<PortArena>();
    match port.kind {
        PortKindSpec::Continuous(ordinal) => {
            let value = arena.continuous[runtime.continuous_base + ordinal as usize]
                .try_downcast_ref::<f32>()
                .copied()
                .expect("traced continuous port is f32");
            Snapshot::Continuous(value)
        }
        PortKindSpec::NoteEvents(ordinal, event_name) => {
            let events = arena.events[runtime.event_base + ordinal as usize]
                .iter()
                .map(|occurrence| {
                    let message = occurrence
                        .value
                        .try_downcast_ref::<NoteMsg>()
                        .expect("traced event payload is NoteMsg");
                    (
                        occurrence.offset,
                        format!("{event_name}({},{})", message.note, message.velocity),
                    )
                })
                .collect();
            Snapshot::Events(events)
        }
    }
}

fn run_trace(name: &str) -> TraceOutput {
    let input = load_input(name);
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(input.tick_hz))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins((GraphPlugin, SignalNodesPlugin));
    app.update();

    let ports = match name {
        "envelope-retrigger" => build_envelope_retrigger(&mut app),
        "lfo-one-cycle" => build_lfo_one_cycle(&mut app),
        "cc-hold" => build_cc_hold(&mut app),
        "chain-math-remap" => build_chain_math_remap(&mut app),
        "two-notes-one-tick" => build_two_notes_one_tick(&mut app),
        "event-fan-in" => build_event_fan_in(&mut app),
        _ => panic!("unknown trace case `{name}`"),
    };
    for (time, message) in input.events {
        app.world_mut().resource_mut::<MidiInbox>().push(
            time,
            RawMidi {
                status: message.status,
                data1: message.data1,
                data2: message.data2,
            },
        );
    }

    let ticks = (0..input.ticks)
        .map(|tick| {
            app.update();
            let values = ports.iter().map(|port| snapshot_port(&app, port)).collect();
            (tick, values)
        })
        .collect();
    TraceOutput {
        ports: ports.iter().map(|port| port.label.to_owned()).collect(),
        ticks,
    }
}

fn assert_or_bless(name: &str, actual: &TraceOutput) {
    let path = trace_path(name, "out");
    if std::env::var("SWAY_BLESS").as_deref() == Ok("1") {
        let serialized = ron::ser::to_string_pretty(actual, ron::ser::PrettyConfig::default())
            .expect("serialize golden trace");
        fs::write(&path, format!("{serialized}\n"))
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
        println!("~ rewrote {}", path.display());
        return;
    }

    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let expected: TraceOutput =
        ron::from_str(&source).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    if expected == *actual {
        return;
    }

    let tick_count = expected.ticks.len().max(actual.ticks.len());
    for tick_position in 0..tick_count {
        let expected_tick = expected.ticks.get(tick_position);
        let actual_tick = actual.ticks.get(tick_position);
        let tick = actual_tick
            .map(|(tick, _)| *tick)
            .or_else(|| expected_tick.map(|(tick, _)| *tick))
            .unwrap_or(tick_position as u32);
        let port_count = expected_tick
            .map_or(0, |(_, values)| values.len())
            .max(actual_tick.map_or(0, |(_, values)| values.len()));
        for port_index in 0..port_count {
            let expected_value = expected_tick.and_then(|(_, values)| values.get(port_index));
            let actual_value = actual_tick.and_then(|(_, values)| values.get(port_index));
            if expected_value != actual_value {
                let port = actual
                    .ports
                    .get(port_index)
                    .or_else(|| expected.ports.get(port_index))
                    .map(String::as_str)
                    .unwrap_or("<missing>");
                panic!(
                    "golden trace mismatch at tick {tick}, port `{port}`: expected \
                     {expected_value:?}, actual {actual_value:?}"
                );
            }
        }
    }
    panic!("golden trace mismatch at tick 0, port `<metadata>` for `{name}`");
}

#[test]
fn envelope_retrigger() {
    let actual = run_trace("envelope-retrigger");
    assert_or_bless("envelope-retrigger", &actual);
}

#[test]
fn lfo_one_cycle() {
    let actual = run_trace("lfo-one-cycle");
    assert_or_bless("lfo-one-cycle", &actual);
}

#[test]
fn cc_hold() {
    let actual = run_trace("cc-hold");
    assert_or_bless("cc-hold", &actual);
}

#[test]
fn chain_math_remap() {
    let actual = run_trace("chain-math-remap");
    assert_or_bless("chain-math-remap", &actual);
}

#[test]
fn two_notes_one_tick() {
    let actual = run_trace("two-notes-one-tick");
    assert_or_bless("two-notes-one-tick", &actual);
}

#[test]
fn event_fan_in() {
    let actual = run_trace("event-fan-in");
    assert_or_bless("event-fan-in", &actual);
}

#[test]
fn the_same_trace_twice_is_bit_identical() {
    let a = run_trace("envelope-retrigger");
    let b = run_trace("envelope-retrigger");
    assert_eq!(a, b);
}
