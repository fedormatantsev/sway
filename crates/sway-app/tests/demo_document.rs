//! The one check that `assets/demo.sway.ron` is a document this build reads.
//!
//! The demo is the only real graph in the tree, so it is the end-to-end gate
//! on the format: a stale `version`, a node kind renamed out from under it, an
//! edge naming a path that no longer exists, or an annotation whose type
//! nothing registered would each leave the app running against an empty scene
//! rather than failing. Every one of those is a `LoadDiagnostics` item, and
//! this asserts there are none.
//!
//! Loads through `load_from_path` against a registry built from the same
//! plugins the app adds, so it exercises the real short-name resolution rather
//! than a hand-listed set of kinds.

use bevy::prelude::*;
use sway_graph::graph::registry::registered_node_kinds;

fn demo_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("demo.sway.ron")
}

/// An app carrying every node kind the demo names, and nothing that needs a
/// GPU: registration is all this test reads, and `MinimalPlugins` already
/// brings the glam types an annotation recovers itself from.
fn registry_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<sway_runtime::SpriteMaterialAsset>();
    app.add_plugins(sway_base_nodes::BaseNodesPlugin);
    sway_runtime::register_runtime_node_kinds(&mut app);
    // `MidiTime` comes from `MidiPlugin`, which wants a channel it will never
    // read here.
    let (_tx, rx) = crossbeam_channel::unbounded();
    app.add_plugins(sway_midi::MidiPlugin { rx });
    app
}

#[test]
fn the_demo_document_loads_with_no_diagnostics() {
    let app = registry_app();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let (graph, ids, diagnostics) =
        sway_document::load_from_path(&demo_path(), &registry).expect("the demo parses");

    assert!(
        diagnostics.is_clean(),
        "the demo must load clean; got {:#?}\n(registered kinds: {:?})",
        diagnostics.items,
        registered_node_kinds(&registry),
    );
    assert_eq!(graph.len(), 26, "every node in the file loaded");
    assert!(!graph.edges().is_empty());
    assert!(ids.node_of("cubeA").is_some(), "ids are keyed by the file's");
}

#[test]
fn every_node_in_the_demo_carries_its_canvas_placement() {
    // The annotation round trip, on the real file: `"pos"` is a `Vec2` the
    // graph does not interpret, and it has to survive a load or reopening the
    // project loses the canvas the author left.
    let app = registry_app();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let (graph, ids, _) =
        sway_document::load_from_path(&demo_path(), &registry).expect("the demo parses");

    for (id, node) in graph.iter() {
        let pos = node
            .metadata()
            .get("pos")
            .unwrap_or_else(|| panic!("{id} carries no placement"));
        assert!(
            pos.try_downcast_ref::<Vec2>().is_some(),
            "{id}'s placement must read back as the `Vec2` it was written as",
        );
    }

    let midi_time = ids.node_of("midiTime").expect("the demo has a MidiTime");
    assert_eq!(
        graph.get(midi_time).unwrap().metadata()["pos"].try_downcast_ref::<Vec2>(),
        Some(&Vec2::new(-700.0, 100.0)),
    );
}

#[test]
fn a_version_three_demo_would_be_refused_by_version() {
    // The format broke at 4 rather than growing a compatibility read for a
    // field that should never have been in it. What a holder of an older copy
    // sees is this message, not a serde error about a missing field.
    let text = std::fs::read_to_string(demo_path()).expect("readable");
    let downgraded = text.replace("    version: 4,", "    version: 3,");
    assert_ne!(downgraded, text, "the file really does declare version 4");

    let error = sway_document::parse(&downgraded).expect_err("must be refused");
    assert_eq!(
        error.to_string(),
        "graph document version 3 is not supported (this build reads 4)",
    );
}

#[test]
fn the_demo_survives_a_save_and_reload_with_its_placement_intact() {
    // What a save from the editor does to this file, minus the editor: load,
    // write, read back. A placement lost here is a canvas the author has to
    // rebuild every time they reopen the project.
    let app = registry_app();
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let registry = registry.read();

    let (graph, mut ids, diagnostics) =
        sway_document::load_from_path(&demo_path(), &registry).expect("the demo parses");
    assert!(diagnostics.is_clean(), "{:#?}", diagnostics.items);

    let dir = std::env::temp_dir().join("sway-demo-round-trip");
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let saved = dir.join(format!("{}.sway.ron", std::process::id()));
    sway_document::save_to_path(&graph, &registry, &mut ids, &saved).expect("saves");

    let (reopened, reopened_ids, reopened_diagnostics) =
        sway_document::load_from_path(&saved, &registry).expect("the saved file parses");
    assert!(
        reopened_diagnostics.is_clean(),
        "a file this build wrote must be one it reads: {:#?}",
        reopened_diagnostics.items,
    );
    assert_eq!(reopened.len(), graph.len());
    assert_eq!(reopened.edges().len(), graph.edges().len());

    for (id, node) in graph.iter() {
        let stable = ids.id_of(id).expect("every loaded node has a stable id");
        let same = reopened_ids
            .node_of(stable)
            .and_then(|other| reopened.get(other))
            .unwrap_or_else(|| panic!("{stable} did not survive the round trip"));
        assert_eq!(same.kind(), node.kind());
        assert_eq!(
            same.metadata()["pos"].try_downcast_ref::<Vec2>(),
            node.metadata()["pos"].try_downcast_ref::<Vec2>(),
            "{stable} moved across a save",
        );
    }

    // And a second save of an unchanged document is byte-identical, so a
    // committed asset's diff stays clean.
    let again = dir.join(format!("{}-again.sway.ron", std::process::id()));
    sway_document::save_to_path(&reopened, &registry, &mut ids.clone(), &again).expect("saves");
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap(),
        std::fs::read_to_string(&again).unwrap(),
    );

    let _ = std::fs::remove_file(&saved);
    let _ = std::fs::remove_file(&again);
}
