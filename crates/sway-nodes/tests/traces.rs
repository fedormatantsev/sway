use std::fs;
use std::path::PathBuf;

use bevy_app::App;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use serde::{Deserialize, Serialize};
use sway_graph::{Transport, TransportTime, WiresPlugin};
use sway_nodes::{
    BeatTriggerState, Division, EnvelopeParams, EnvelopeState, MathOp, MidiInbox, MidiPlugin,
    RawMidi, TickMidi, Waveform, beat_pulses, envelope_tick, lfo_value, math_value, note_message,
    remap_value,
};

#[derive(Debug, Deserialize)]
struct TraceInput {
    tick_hz: f64,
    ticks: u32,
    #[serde(default)]
    events: Vec<(f64, MidiEvent)>,
    #[serde(default)]
    clock: Option<ClockSpec>,
}

#[derive(Debug, Deserialize)]
struct ClockSpec {
    start: f64,
    segments: Vec<(f64, f64)>,
    #[serde(default)]
    jitter: f64,
    #[serde(default)]
    dropout: Option<(f64, f64)>,
}

struct Lcg(u64);

impl Lcg {
    fn next_signed(&mut self, magnitude: f64) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let unit = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        (unit * 2.0 - 1.0) * magnitude
    }
}

fn clock_events(spec: &ClockSpec) -> Vec<(f64, RawMidi)> {
    let mut random = Lcg(0xC10C_C10C);
    let mut events = Vec::new();
    let mut at = spec.start;
    for &(bpm, beats) in &spec.segments {
        let seconds_per_pulse = (60.0 / bpm) / 24.0;
        for _ in 0..((beats * 24.0).round() as usize) {
            let dropped = spec.dropout.is_some_and(|(from, to)| at >= from && at < to);
            if !dropped {
                events.push((
                    at + random.next_signed(spec.jitter),
                    RawMidi {
                        status: sway_midi::CLOCK,
                        data1: 0,
                        data2: 0,
                    },
                ));
            }
            at += seconds_per_pulse;
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    events
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
    Transport,
    Beat {
        state: BeatTriggerState,
    },
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
            "transport-lock" | "transport-tempo-change" | "transport-dropout" => (
                Self::Transport,
                vec![
                    "transport.bpm".into(),
                    "transport.beats".into(),
                    "transport.playing".into(),
                ],
            ),
            "beat-trigger" => (
                Self::Beat {
                    state: BeatTriggerState::default(),
                },
                vec![
                    "transport.bpm".into(),
                    "transport.beats".into(),
                    "transport.playing".into(),
                    "beat.pulse".into(),
                ],
            ),
            _ => panic!("unknown trace case `{name}`"),
        }
    }

    fn snapshot(&mut self, app: &App) -> Vec<Snapshot> {
        let fixed = app.world().resource::<Time<Fixed>>();
        let dt = fixed.delta_secs();
        let tick_start = fixed.elapsed_secs_f64() - dt as f64;
        match self {
            Self::Envelope {
                state,
                fan_in,
                trace_notes,
            } => {
                let messages = &app.world().resource::<TickMidi>().events;
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
                for &(_, message) in &app.world().resource::<TickMidi>().events {
                    if let Some(value) = sway_nodes::cc_value(message, 0, 74) {
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
            Self::Transport => transport_snapshots(app),
            Self::Beat { state } => {
                let time = app.world().resource::<Time<Transport>>();
                let pulses = beat_pulses(
                    state,
                    Division::Beat,
                    time.is_playing(),
                    time.transport().beats_per_bar,
                    time.beats(),
                    time.delta_secs_f64(),
                    time.transport().origin_beats,
                    dt,
                );
                let mut snapshots = transport_snapshots(app);
                snapshots.push(Snapshot::Events(
                    pulses
                        .into_iter()
                        .map(|pulse| {
                            (
                                pulse.offset,
                                format!(
                                    "beat({},{},{})",
                                    pulse.value.bar, pulse.value.beat, pulse.value.sixteenth
                                ),
                            )
                        })
                        .collect(),
                ));
                snapshots
            }
        }
    }
}

fn transport_snapshots(app: &App) -> Vec<Snapshot> {
    let time = app.world().resource::<Time<Transport>>();
    vec![
        Snapshot::Continuous(time.bpm() as f32),
        Snapshot::Continuous(time.beats() as f32),
        Snapshot::Continuous(if time.is_playing() { 1.0 } else { 0.0 }),
    ]
}

fn run_trace(name: &str) -> TraceOutput {
    let input = load_input(name);
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(input.tick_hz))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins((WiresPlugin, MidiPlugin));
    app.update();

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
    if let Some(clock) = &input.clock {
        for (time, message) in clock_events(clock) {
            app.world_mut()
                .resource_mut::<MidiInbox>()
                .push(time, message);
        }
    }

    let (mut runner, ports) = Runner::for_case(name);
    let ticks = (0..input.ticks)
        .map(|tick| {
            app.update();
            (tick, runner.snapshot(&app))
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
trace_test!(transport_lock, "transport-lock");
trace_test!(transport_tempo_change, "transport-tempo-change");
trace_test!(transport_dropout, "transport-dropout");
trace_test!(beat_trigger, "beat-trigger");

#[test]
fn traces_replay_bit_identically() {
    assert_eq!(
        run_trace("transport-dropout"),
        run_trace("transport-dropout")
    );
    assert_eq!(
        run_trace("envelope-retrigger"),
        run_trace("envelope-retrigger")
    );
}
