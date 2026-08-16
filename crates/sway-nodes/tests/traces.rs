use std::fs;
use std::path::PathBuf;

use bevy_app::App;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use serde::{Deserialize, Serialize};
use sway_nodes::{
    EnvelopeParams, EnvelopeState, MathOp, NoteMsg, Waveform, envelope_tick, lfo_value, math_value,
    remap_value,
};

#[derive(Debug, Deserialize)]
struct TraceInput {
    tick_hz: f64,
    ticks: u32,
    #[serde(default)]
    events: Vec<(f64, TraceMidi)>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
struct TraceMidi {
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

fn note_message(
    message: TraceMidi,
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

fn cc_value(message: TraceMidi, channel: u8, cc: u8) -> Option<f32> {
    (message.status & 0xf0 == 0xb0 && message.status & 0x0f == channel && message.data1 == cc)
        .then(|| message.data2 as f32 / 127.0)
}

fn due_this_tick(
    events: &[(f64, TraceMidi)],
    tick_start: f64,
    tick_end: f64,
    dt: f32,
) -> Vec<(f32, TraceMidi)> {
    events
        .iter()
        .filter(|(time, _)| *time > tick_start && *time <= tick_end)
        .map(|(time, message)| (((time - tick_start).clamp(0.0, dt as f64)) as f32, *message))
        .collect()
}

enum Runner {
    Envelope {
        state: EnvelopeState,
        fan_in: bool,
        trace_notes: bool,
    },
    Lfo,
    Cc {
        held: f32,
    },
    Chain,
}

impl Runner {
    fn for_case(name: &str) -> (Self, Vec<String>) {
        match name {
            "envelope-retrigger" | "two-notes-one-tick" => (
                Self::Envelope {
                    state: EnvelopeState::default(),
                    fan_in: false,
                    trace_notes: true,
                },
                vec!["envelope.value".into(), "midinote.note_on".into()],
            ),
            "event-fan-in" => (
                Self::Envelope {
                    state: EnvelopeState::default(),
                    fan_in: true,
                    trace_notes: true,
                },
                vec!["envelope.value".into(), "envelope.trigger".into()],
            ),
            "lfo-one-cycle" => (Self::Lfo, vec!["lfo.value".into()]),
            "cc-hold" => (Self::Cc { held: 0.0 }, vec!["midicc.value".into()]),
            "chain-math-remap" => (
                Self::Chain,
                vec![
                    "lfo.value".into(),
                    "math.value".into(),
                    "remap.value".into(),
                ],
            ),
            _ => panic!("unknown trace case `{name}`"),
        }
    }

    fn snapshot(
        &mut self,
        tick_start: f64,
        dt: f32,
        messages: &[(f32, TraceMidi)],
    ) -> Vec<Snapshot> {
        match self {
            Self::Envelope {
                state,
                fan_in,
                trace_notes,
            } => {
                let channels: &[u8] = if *fan_in { &[0, 1] } else { &[0] };
                let mut envelope_events = Vec::new();
                let mut note_events = Vec::new();
                for &channel in channels {
                    for &(offset, raw) in messages {
                        if let Some((gate_on, note)) = note_message(raw, channel, 0, 127) {
                            if gate_on {
                                note_events.push((offset, note.clone()));
                            }
                            envelope_events.push((offset, gate_on, note));
                        }
                    }
                }
                envelope_events.sort_by(|a, b| a.0.total_cmp(&b.0));
                note_events.sort_by(|a, b| a.0.total_cmp(&b.0));
                let value = envelope_tick(
                    state,
                    &envelope_events,
                    tick_start,
                    dt,
                    EnvelopeParams {
                        attack: 0.05,
                        decay: 0.08,
                        sustain: 0.4,
                        release: 0.1,
                    },
                );
                let mut snapshots = vec![Snapshot::Continuous(value)];
                if *trace_notes {
                    snapshots.push(Snapshot::Events(
                        note_events
                            .into_iter()
                            .map(|(offset, note)| {
                                (offset, format!("note_on({},{})", note.note, note.velocity))
                            })
                            .collect(),
                    ));
                }
                snapshots
            }
            Self::Lfo => vec![Snapshot::Continuous(lfo_value(
                2.0,
                Waveform::Sine,
                0.0,
                1.0,
                tick_start,
            ))],
            Self::Cc { held } => {
                for &(_, message) in messages {
                    if let Some(value) = cc_value(message, 0, 74) {
                        *held = value;
                    }
                }
                vec![Snapshot::Continuous(*held)]
            }
            Self::Chain => {
                let lfo = lfo_value(1.0, Waveform::Sine, 0.0, 1.0, tick_start);
                let math = math_value(MathOp::Add, lfo, 1.0);
                let remap = remap_value(math, 0.0, 2.0, -1.0, 1.0, true);
                vec![
                    Snapshot::Continuous(lfo),
                    Snapshot::Continuous(math),
                    Snapshot::Continuous(remap),
                ]
            }
        }
    }
}

fn run_trace(name: &str) -> TraceOutput {
    let input = load_input(name);
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(input.tick_hz))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1));
    app.update();

    let (mut runner, ports) = Runner::for_case(name);
    let ticks = (0..input.ticks)
        .map(|tick| {
            app.update();
            let fixed = app.world().resource::<Time<Fixed>>();
            let dt = fixed.delta_secs();
            let tick_start = fixed.elapsed_secs_f64() - dt as f64;
            let tick_end = tick_start + dt as f64;
            let messages = due_this_tick(&input.events, tick_start, tick_end, dt);
            (tick, runner.snapshot(tick_start, dt, &messages))
        })
        .collect();
    TraceOutput { ports, ticks }
}

fn assert_or_bless(name: &str, actual: &TraceOutput) {
    let path = trace_path(name, "out");
    if std::env::var("SWAY_BLESS").as_deref() == Ok("1") {
        let serialized = ron::ser::to_string_pretty(actual, ron::ser::PrettyConfig::default())
            .expect("serialize golden trace");
        fs::write(&path, format!("{serialized}\n"))
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
        return;
    }
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let expected: TraceOutput =
        ron::from_str(&source).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
    if expected == *actual {
        return;
    }
    for (expected_tick, actual_tick) in expected.ticks.iter().zip(&actual.ticks) {
        for (port_index, (expected_value, actual_value)) in
            expected_tick.1.iter().zip(&actual_tick.1).enumerate()
        {
            if expected_value != actual_value {
                panic!(
                    "golden trace mismatch at tick {}, port `{}`: expected {:?}, actual {:?}",
                    actual_tick.0,
                    actual
                        .ports
                        .get(port_index)
                        .map(String::as_str)
                        .unwrap_or("<missing>"),
                    expected_value,
                    actual_value
                );
            }
        }
    }
    panic!("golden trace metadata mismatch for `{name}`");
}

macro_rules! trace_test {
    ($name:ident, $case:literal) => {
        #[test]
        fn $name() {
            let actual = run_trace($case);
            assert_or_bless($case, &actual);
        }
    };
}

trace_test!(envelope_retrigger, "envelope-retrigger");
trace_test!(lfo_one_cycle, "lfo-one-cycle");
trace_test!(cc_hold, "cc-hold");
trace_test!(chain_math_remap, "chain-math-remap");
trace_test!(two_notes_one_tick, "two-notes-one-tick");
trace_test!(event_fan_in, "event-fan-in");

#[test]
fn traces_replay_bit_identically() {
    assert_eq!(
        run_trace("envelope-retrigger"),
        run_trace("envelope-retrigger")
    );
}
