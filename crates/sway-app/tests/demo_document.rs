//! The demo document's only non-visual coverage.
//!
//! Parses and applies the real `assets/demo.sway.ron`, then asserts the world
//! against the document's own comment-drawn diagram. A renamed short name, a
//! malformed payload, or a dropped `register_authorable`/`register_wire` call
//! would otherwise leave the suite green and only surface when a human ran the
//! app.
//!
//!   lfoA ──amplitude──▶ lfoB
//!   lfoA ──vec3.y────▶ vec3A ──translation──▶ cubeA ─┐
//!   lfoB ──vec3.y────▶ vec3B ──translation──▶ cubeB ─┤─parent─▶ group
//!   mat  ──material──▶ cubeA, cubeB

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use sway_document::{DocId, to_document};
use sway_nodes::{AmplitudeFrom, MaterialFrom, MaterialOut, TranslationFrom, Vec3YFrom};

const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

fn demo_app() -> App {
    let mut app = App::new();
    app.add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin));
    app
}

fn entity_named(world: &mut World, id: &str) -> Entity {
    world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .find(|(_, doc_id)| doc_id.0 == id)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("demo document has no entity \"{id}\""))
}

#[test]
fn demo_document_parses() {
    sway_document::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
}

#[test]
fn demo_document_loads_and_reconciles_cleanly() {
    let document = sway_document::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();

    let diagnostics = sway_document::apply(app.world_mut(), &document);

    assert!(
        diagnostics.is_clean(),
        "the demo document should be clean against the current registry, got: {:?}",
        diagnostics.items
    );

    let world = app.world_mut();
    let mut ids: Vec<String> = world.query::<&DocId>().iter(world).map(|id| id.0.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "camera".to_string(),
            "cubeA".to_string(),
            "cubeB".to_string(),
            "group".to_string(),
            "lfoA".to_string(),
            "lfoB".to_string(),
            "mat".to_string(),
            "sun".to_string(),
            "vec3A".to_string(),
            "vec3B".to_string(),
        ],
        "exactly the demo's 10 entities should carry a DocId"
    );

    let lfo_a = entity_named(world, "lfoA");
    let lfo_b = entity_named(world, "lfoB");
    let vec3_a = entity_named(world, "vec3A");
    let vec3_b = entity_named(world, "vec3B");
    let cube_a = entity_named(world, "cubeA");
    let cube_b = entity_named(world, "cubeB");
    let group = entity_named(world, "group");
    let material = entity_named(world, "mat");
    let camera = entity_named(world, "camera");
    let sun = entity_named(world, "sun");

    assert_eq!(world.get::<AmplitudeFrom>(lfo_b).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_a).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_b).map(|w| w.0), Some(lfo_b));
    assert_eq!(world.get::<TranslationFrom>(cube_a).map(|w| w.0), Some(vec3_a));
    assert_eq!(world.get::<TranslationFrom>(cube_b).map(|w| w.0), Some(vec3_b));
    assert_eq!(world.get::<MaterialFrom>(cube_a).map(|w| w.0), Some(material));
    assert_eq!(world.get::<MaterialFrom>(cube_b).map(|w| w.0), Some(material));
    assert_eq!(world.get::<ChildOf>(cube_a).map(|c| c.parent()), Some(group));
    assert_eq!(world.get::<ChildOf>(cube_b).map(|c| c.parent()), Some(group));

    // D4: the document names one component per node and Bevy supplies the rest.
    // None of these appear in the file.
    assert!(world.get::<sway_nodes::FloatOut>(lfo_a).is_some(), "Lfo requires FloatOut");
    assert!(world.get::<Mesh3d>(cube_a).is_some(), "MeshAsset requires Mesh3d");
    assert!(world.get::<Visibility>(cube_a).is_some(), "MeshAsset requires Visibility");
    assert!(world.get::<Transform>(cube_a).is_some(), "Mesh3d requires Transform");
    assert!(world.get::<MaterialOut>(material).is_some(), "PbrMaterial requires MaterialOut");
    assert!(world.get::<Camera3d>(camera).is_some(), "SceneCamera requires Camera3d");
    assert!(world.get::<Transform>(sun).is_some(), "DirectionalLight requires Transform");
}

#[test]
fn demo_document_survives_a_reload() {
    // The hot-reload path, and the sharp case for Task 1's exemption: on the
    // second apply the required companions are already present and still
    // unnamed, which is exactly what the removal pass looks for.
    let document = sway_document::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();
    sway_document::apply(app.world_mut(), &document);
    let cube = entity_named(app.world_mut(), "cubeA");

    sway_document::apply(app.world_mut(), &document);

    assert!(app.world().get::<Mesh3d>(cube).is_some(), "Mesh3d survived the reload");
    assert!(app.world().get::<Transform>(cube).is_some(), "Transform survived the reload");
}

#[test]
fn demo_document_round_trips_through_the_world() {
    let document = sway_document::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();
    sway_document::apply(app.world_mut(), &document);
    let once = to_document(app.world_mut());

    let mut second = demo_app();
    let diagnostics = sway_document::apply(second.world_mut(), &once);
    assert!(diagnostics.is_clean(), "re-apply of emitted doc: {:?}", diagnostics.items);
    let twice = to_document(second.world_mut());

    assert_eq!(once, twice);
}
