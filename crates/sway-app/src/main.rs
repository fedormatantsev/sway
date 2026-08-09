use sway_app::demo_assets;

mod midi_feed;
mod presenter;
mod scene;
mod shell;

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::math::UVec2;
use bevy::prelude::*;
use bevy::window::Monitor;
use midi_feed::{MidiClockOffset, MidiRx, feed_midi};
use scene::setup_scene;

/// Provisional graph tick rate pending the measurements specified in spec §11.
const TICK_HZ: f64 = 120.0;

/// Which M1 render spike (if any) to run instead of the project document. See
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

#[allow(dead_code)] // `monitor` and `windowed`: see the DEVIATION note in main().
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
/// DEVIATION (Task 3): this can no longer fire. `Monitor` entities are
/// spawned by `bevy_winit`'s `create_monitors`, called from winit's
/// event-loop resume handler -- but `WinitPlugin` is disabled now (see
/// `sway_runtime::headless::build_app`), so nothing ever spawns a `Monitor`
/// entity and this query is permanently empty. Left in place rather than
/// deleted: it is harmless (never logs, never panics) and documents that
/// `--monitor` selection is a known, accepted regression until M6 (per the
/// task-3 brief's ruling on this).
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
fn log_fps(diagnostics: Res<DiagnosticsStore>, time: Res<Time>, mut since_last_log: Local<f32>) {
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

fn load_project(asset_server: Res<AssetServer>, mut handle: ResMut<sway_graph::ProjectHandle>) {
    handle.0 = Some(asset_server.load("demo.sway.ron"));
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
        // Briefly publish the virtual destination so `--list` shows what
        // Ableton will see while sway is running.
        let (tx, _rx) = crossbeam_channel::unbounded();
        let _midi = sway_midi::open_input("", tx).ok();
        let destinations = sway_midi::list_destinations();
        if destinations.is_empty() {
            eprintln!("no CoreMIDI destinations found");
        } else {
            eprintln!("CoreMIDI destinations:");
            for (i, name) in &destinations {
                eprintln!("  {i}: {name}");
            }
        }
        return;
    }

    let destinations = sway_midi::list_destinations();
    if destinations.is_empty() {
        eprintln!("no CoreMIDI destinations found");
    } else {
        eprintln!("CoreMIDI destinations:");
        for (i, name) in &destinations {
            eprintln!("  {i}: {name}");
        }
    }

    let (tx, rx) = crossbeam_channel::unbounded();
    // Held for the process lifetime: dropping it closes the port/destination
    // and frees the sender the CoreMIDI callback points at. `shell::run`
    // below blocks until the window closes, so this stays alive on `main`'s
    // stack for exactly as long as it needs to.
    let _midi = match sway_midi::open_input(&args.midi_filter, tx) {
        Ok(conn) => {
            eprintln!(
                "virtual MIDI destination '{}' published (Ableton: MIDI To → {})",
                sway_midi::VIRTUAL_DESTINATION_NAME,
                sway_midi::VIRTUAL_DESTINATION_NAME,
            );
            Some(conn)
        }
        Err(status) => {
            eprintln!("could not open MIDI input (OSStatus {status}); continuing without MIDI");
            None
        }
    };

    // DEVIATION (Task 3): `--monitor` and fullscreen selection are dropped.
    // Window creation now happens once, in `shell::run`, before any demo is
    // known, and is windowed-only until M6 (per the task-3 brief's ruling on
    // this) -- `args.monitor`/`args.windowed` are still parsed (so existing
    // invocations don't fail argument parsing) but no longer read here.
    let demo = args.demo;
    let editor = args.editor;

    // Everything demo-specific is built into the closure the shell calls
    // once the window, shared device, and viewport texture exist --
    // `sway_runtime::headless::build_app` builds the underlying `App`
    // (Bevy's `RenderPlugin` in manual mode, no window, no winit event loop
    // of its own); this closure only adds what's specific to this run.
    let build_app: shell::AppBuilder = Box::new(move |gpu, viewport, size: UVec2| {
        let mut app = sway_runtime::headless::build_app(gpu, viewport, size);

        if editor {
            app.insert_resource(sway_graph::Authoring);
        }

        app.add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            sway_graph::WiresPlugin,
            sway_graph::ProjectPlugin,
            sway_nodes::WireNodesPlugin,
            sway_nodes::MidiPlugin,
            demo_assets::DemoAssetsPlugin,
        ))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(MidiRx(rx))
        .init_resource::<MidiClockOffset>()
        .add_systems(Startup, load_project)
        .add_systems(PreUpdate, feed_midi)
        .add_systems(Update, (log_monitors, log_fps));

        // Camera-collision hazard: `scene::setup_scene` (camera + light for
        // the project-document demo) and each demo's own setup helper each spawn a
        // camera, and Bevy renders every camera with the same (default)
        // order to the same target -- the last one drawn wins and the rest
        // are invisibly overdrawn. So exactly one of "the demo graph" or "a
        // render spike demo" runs per process, never both, and `all` is
        // wired to end up with exactly one active camera too:
        //   - `point-cloud` spawns its own camera (required: it carries
        //     `NoIndirectDrawing`, which the point-cloud pipeline needs).
        //   - `sprites` spawns its own dedicated camera via
        //     `sprite_layer::spawn_demo_camera`.
        //   - `scatter` spawns no camera at all: it is compute + readback
        //     only, proven by a log line, not by anything on screen.
        //   - `all` reuses the point cloud's camera for the sprite layers
        //     too (skipping `spawn_demo_camera`) rather than spawning a
        //     second one.
        match demo {
            None => {
                app.add_systems(Startup, setup_scene);
            }
            Some(Demo::PointCloud) => {
                app.add_plugins(sway_runtime::PointCloudPlugin)
                    .add_systems(Startup, sway_runtime::point_cloud::spawn_demo_point_cloud);
            }
            Some(Demo::Sprites) => {
                app.add_plugins(sway_runtime::SpriteLayerPlugin)
                    .add_systems(
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

        app
    });

    shell::run(shell::ShellConfig {
        editor,
        build_app,
    });
}
