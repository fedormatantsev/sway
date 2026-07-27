mod graph;
mod scene;
mod shell;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::{Monitor, MonitorSelection, WindowMode};
use graph::{graph_tick, GraphState, MidiRx, TICK_HZ};
use scene::{apply_level, setup_scene};

/// Which M1 render spike (if any) to run instead of the M0 cube. See
/// `main`'s demo-dispatch match for how each variant is wired up, and its
/// comment on the camera-collision hazard between these demos and
/// `scene::setup_scene`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Demo {
    PointCloud,
    Sprites,
    Scatter,
    All,
}

struct Args {
    monitor: usize,
    midi_filter: String,
    windowed: bool,
    list_only: bool,
    demo: Option<Demo>,
    editor: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        monitor: 0,
        midi_filter: String::new(),
        windowed: false,
        list_only: false,
        demo: None,
        editor: false,
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
            "--editor" => args.editor = true,
            "--demo" => {
                let value = it.next().expect("--demo needs a value");
                args.demo = Some(match value.as_str() {
                    "point-cloud" => Demo::PointCloud,
                    "sprites" => Demo::Sprites,
                    "scatter" => Demo::Scatter,
                    "all" => Demo::All,
                    other => panic!("unknown --demo value: {other}"),
                });
            }
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

/// Logs `FrameTimeDiagnosticsPlugin`'s smoothed FPS once per second. "At
/// frame rate" is an M1 exit criterion and needs a measured number, not an
/// impression — this is what produces that number in the run logs.
fn log_fps(
    diagnostics: Res<DiagnosticsStore>,
    time: Res<Time>,
    mut since_last_log: Local<f32>,
) {
    *since_last_log += time.delta_secs();
    if *since_last_log < 1.0 {
        return;
    }
    *since_last_log = 0.0;

    if let Some(fps) = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
    {
        info!("fps (smoothed): {fps:.1}");
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

    // `--editor` runs the M1b winit/vello shell instead of the Bevy app
    // below, and must branch before any of it is touched: `DefaultPlugins`
    // (added further down) creates its own winit event loop as soon as
    // `add_plugins` runs, not lazily at `app.run()`, and winit allows only
    // one event loop per process -- building the Bevy app first and
    // deciding whether to call `.run()` afterward panics with
    // `EventLoopError::RecreationAttempt` the moment the editor shell tries
    // to create its own. Task 3 unifies the two paths; for now they are
    // mutually exclusive and this is the earliest point that's true.
    if args.editor {
        shell::run();
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

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            mode,
            title: "sway".into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
    .insert_resource(MidiRx(rx))
    .init_resource::<GraphState>()
    .add_systems(FixedUpdate, graph_tick)
    .add_systems(Update, (apply_level, log_monitors, log_fps));

    // Camera-collision hazard: `scene::setup_scene` (M0) and each demo's own
    // setup helper each spawn a camera, and Bevy renders every camera with
    // the same (default) order to the same window — the last one drawn wins
    // and the rest are invisibly overdrawn. So exactly one of "M0 scene" or
    // "a demo" runs per process, never both, and `all` is wired to end up
    // with exactly one active camera too:
    //   - `point-cloud` spawns its own camera (required: it carries
    //     `NoIndirectDrawing`, which the point-cloud pipeline needs).
    //   - `sprites` spawns its own dedicated camera via
    //     `sprite_layer::spawn_demo_camera`.
    //   - `scatter` spawns no camera at all: it is compute + readback only,
    //     proven by a log line, not by anything on screen.
    //   - `all` reuses the point cloud's camera for the sprite layers too
    //     (skipping `spawn_demo_camera`) rather than spawning a second one.
    match args.demo {
        None => {
            app.add_systems(Startup, setup_scene);
        }
        Some(Demo::PointCloud) => {
            app.add_plugins(sway_runtime::PointCloudPlugin).add_systems(
                Startup,
                sway_runtime::point_cloud::spawn_demo_point_cloud,
            );
        }
        Some(Demo::Sprites) => {
            app.add_plugins(sway_runtime::SpriteLayerPlugin).add_systems(
                Startup,
                (
                    sway_runtime::sprite_layer::spawn_demo_sprite_layers,
                    sway_runtime::sprite_layer::spawn_demo_camera,
                ),
            );
        }
        Some(Demo::Scatter) => {
            app.add_plugins(sway_runtime::ScatterPlugin)
                .add_systems(Startup, sway_runtime::scatter::spawn_demo_scatter);
        }
        Some(Demo::All) => {
            app.add_plugins((
                sway_runtime::PointCloudPlugin,
                sway_runtime::SpriteLayerPlugin,
                sway_runtime::ScatterPlugin,
            ))
            .add_systems(
                Startup,
                (
                    sway_runtime::point_cloud::spawn_demo_point_cloud,
                    sway_runtime::sprite_layer::spawn_demo_sprite_layers,
                    sway_runtime::scatter::spawn_demo_scatter,
                ),
            );
        }
    }

    app.run();
}
