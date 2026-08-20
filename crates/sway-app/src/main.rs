mod presenter;
mod shell;

use std::path::PathBuf;

use bevy::asset::LoadState;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::math::UVec2;
use bevy::prelude::*;
use bevy::window::Monitor;
use sway_document::v4::{GraphInitialized, LiveGraphPlugin, ProjectDirectory};
use sway_graph::graph::Graph;
use sway_runtime::nodes::{FrameSequence, MeshAsset};
use sway_runtime::{ProducerSet, ProjectionSet, RuntimePlugin};

/// Provisional graph tick rate pending the measurements specified in spec §11.
const TICK_HZ: f64 = 120.0;

struct Args {
    midi_filter: String,
    list_only: bool,
    editor: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        midi_filter: String::new(),
        list_only: false,
        editor: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--midi" => {
                args.midi_filter = it.next().expect("--midi needs a substring");
            }
            "--list" => args.list_only = true,
            "--editor" => args.editor = true,
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
/// monitor selection is a known gap until M6.
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

/// Logs `FrameTimeDiagnosticsPlugin`'s smoothed FPS once per second.
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

/// True once the graph has loaded and every named mesh / frame-sequence asset
/// is either ready or has failed. Evaluation and scene projection wait on
/// this; producer loads and the MIDI drain do not
/// (`architecture`: Evaluation waits for assets; input capture does not).
fn assets_ready(
    initialized: Option<Res<GraphInitialized>>,
    graph: Option<Res<Graph>>,
    server: Option<Res<AssetServer>>,
) -> bool {
    let Some(initialized) = initialized else {
        return false;
    };
    if !initialized.0 {
        return false;
    }
    let Some(graph) = graph else {
        return false;
    };
    let Some(server) = server else {
        return true;
    };
    for (_id, node) in graph.iter() {
        if let Some(mesh) = node.value().downcast_ref::<MeshAsset>() {
            if mesh.inlets.path.is_empty() {
                continue;
            }
            if mesh.state.handle == Handle::<Mesh>::default() {
                return false;
            }
            match server.load_state(&mesh.state.handle) {
                LoadState::Loaded | LoadState::Failed(_) => {}
                _ => return false,
            }
        }
        if let Some(sequence) = node.value().downcast_ref::<FrameSequence>() {
            if sequence.inlets.folder.is_empty() {
                continue;
            }
            if sequence.state.folder == Handle::<bevy::asset::LoadedFolder>::default() {
                return false;
            }
            match server.load_state(&sequence.state.folder) {
                LoadState::Loaded | LoadState::Failed(_) => {}
                _ => return false,
            }
        }
    }
    true
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

    let editor = args.editor;

    let (graph_tx, graph_rx) = crossbeam_channel::unbounded();
    let (viewport_tx, viewport_rx) = crossbeam_channel::unbounded();

    let project = shell::ProjectSpec {
        directory: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        graph_file: "demo.sway.ron".into(),
    };

    // Rebuilt on Open: `Receiver` is `Clone`, so this is `Fn` rather than
    // `FnOnce`. The window and the wgpu device survive; only the `App` does
    // not (`architecture`: A project is a directory).
    let build_app: shell::AppBuilder = Box::new(move |gpu, viewport, size: UVec2, project| {
        let mut app = sway_runtime::headless::build_app(gpu, viewport, size, &project.directory);

        if editor {
            app.insert_resource(sway_editor_viewport::ViewportInputRx(viewport_rx.clone()))
                .add_plugins((
                    sway_editor::edit::GraphEditPlugin::new(graph_rx.clone()),
                    sway_editor_viewport::EditorViewportPlugin,
                ));
        }

        app.insert_resource(ProjectDirectory(project.directory.clone()))
            .add_plugins((
                FrameTimeDiagnosticsPlugin::default(),
                sway_midi::MidiPlugin { rx: rx.clone() },
            ))
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .add_systems(Update, (log_monitors, log_fps));

        app.add_plugins((
            sway_graph::GraphPlugin,
            LiveGraphPlugin {
                graph_file: project.graph_file.clone(),
            },
            sway_base_nodes::BaseNodesPlugin,
            RuntimePlugin,
        ))
        .configure_sets(FixedUpdate, sway_graph::GraphTickSet.run_if(assets_ready))
        .configure_sets(
            Update,
            ProjectionSet.run_if(assets_ready).after(ProducerSet),
        );

        app
    });

    shell::run(shell::ShellConfig {
        editor,
        build_app,
        commands: graph_tx,
        viewport_input: viewport_tx,
        project,
    });
}
