mod graph;
mod scene;

use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, WindowMode};
use graph::{graph_tick, GraphState, MidiRx, TICK_HZ};
use scene::{apply_level, setup_scene};

struct Args {
    monitor: usize,
    midi_filter: String,
    windowed: bool,
    list_only: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        monitor: 0,
        midi_filter: String::new(),
        windowed: false,
        list_only: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--monitor" => {
                args.monitor = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--monitor needs a number");
            }
            "--midi" => {
                args.midi_filter = it.next().expect("--midi needs a substring");
            }
            "--windowed" => args.windowed = true,
            "--list" => args.list_only = true,
            other => panic!("unknown argument: {other}"),
        }
    }
    args
}

/// Logs every monitor once, so choosing `--monitor N` does not require
/// guessing.
///
/// This must run in `Update`, not `Startup`. Bevy spawns `Monitor` entities
/// from `create_monitors`, which winit calls from its event-loop resume
/// handler — after `Startup` has already run, so a `Startup` query sees an
/// empty world. The `Local` latch makes it fire once, on the first frame where
/// monitors actually exist.
fn log_monitors(monitors: Query<&Monitor>, mut logged: Local<bool>) {
    if *logged || monitors.is_empty() {
        return;
    }
    *logged = true;
    for (i, m) in monitors.iter().enumerate() {
        info!(
            "monitor {i}: {} {}x{} @ {:?} mHz",
            m.name.as_deref().unwrap_or("<unnamed>"),
            m.physical_width,
            m.physical_height,
            m.refresh_rate_millihertz,
        );
    }
}

fn main() {
    let args = parse_args();

    let sources = sway_midi::list_sources();
    if sources.is_empty() {
        eprintln!("no CoreMIDI sources found");
    } else {
        eprintln!("CoreMIDI sources:");
        for (i, name) in &sources {
            eprintln!("  {i}: {name}");
        }
    }
    if args.list_only {
        return;
    }

    let (tx, rx) = crossbeam_channel::unbounded();
    // Held for the process lifetime: dropping it closes the port and frees the
    // sender the CoreMIDI callback points at.
    let _midi = match sway_midi::open_input(&args.midi_filter, tx) {
        Ok(conn) => Some(conn),
        Err(status) => {
            eprintln!("could not open MIDI input (OSStatus {status}); continuing without MIDI");
            None
        }
    };

    let mode = if args.windowed {
        WindowMode::Windowed
    } else {
        WindowMode::BorderlessFullscreen(MonitorSelection::Index(args.monitor))
    };

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                mode,
                title: "sway".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(MidiRx(rx))
        .init_resource::<GraphState>()
        .add_systems(Startup, setup_scene)
        .add_systems(FixedUpdate, graph_tick)
        .add_systems(Update, (apply_level, log_monitors))
        .run();
}
