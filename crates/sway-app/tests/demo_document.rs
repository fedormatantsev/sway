//! The demo document's only automated coverage. Design spec §7 test 6 calls
//! for verifying the demo document loads into the expected world shape; up
//! to now nothing parsed or applied `assets/demo.sway.ron` at all, so a
//! renamed authorable short name, a malformed payload, or a dropped
//! `register_authorable`/`register_wire` call in `sway-nodes`'s
//! `WireNodesPlugin` would leave the whole suite green and only surface when
//! a human ran the app.
//!
//! This mirrors what `sway-app/src/main.rs` actually wires up for the demo
//! (`sway_graph::WiresPlugin`, `sway_nodes::WireNodesPlugin`,
//! `demo_assets::DemoAssetsPlugin`), then parses and applies the real asset
//! file, and asserts the resulting world against the document's own
//! comment-drawn diagram:
//!
//!   Lfo A ──amplitude──▶ Lfo B ──vec3.y──▶ vec3B ──translation──▶ cube B
//!         └──vec3.y──▶ vec3A ──translation──▶ cube A
//!   group ──parent──▶ cube A, cube B

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::{App, Entity};
use sway_app::demo_assets::DemoAssetsPlugin;
use sway_graph::project::{DocId, to_document};
use sway_nodes::{AmplitudeFrom, TranslationFrom, Vec3YFrom};

const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

fn demo_app() -> App {
    let mut app = App::new();
    app.add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin, DemoAssetsPlugin));
    app
}

fn entity_named(world: &mut bevy::ecs::world::World, id: &str) -> Entity {
    world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .find(|(_, doc_id)| doc_id.0 == id)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("demo document has no entity \"{id}\""))
}

#[test]
fn demo_document_parses() {
    sway_graph::project::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
}

#[test]
fn demo_document_loads_and_reconciles_cleanly() {
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
    let mut app = demo_app();

    let diagnostics = sway_graph::project::apply(app.world_mut(), &document);

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
            "cubeA".to_string(),
            "cubeB".to_string(),
            "group".to_string(),
            "lfoA".to_string(),
            "lfoB".to_string(),
            "vec3A".to_string(),
            "vec3B".to_string(),
        ],
        "exactly the demo's 7 entities should carry a DocId"
    );

    let lfo_a = entity_named(world, "lfoA");
    let lfo_b = entity_named(world, "lfoB");
    let group = entity_named(world, "group");
    let cube_a = entity_named(world, "cubeA");
    let cube_b = entity_named(world, "cubeB");

    // Six wires: amplitude, two vec3.y, two translation, and two parent
    // wires — matching the document.
    assert_eq!(world.get::<AmplitudeFrom>(lfo_b).map(|w| w.0), Some(lfo_a));
    let vec3_a = entity_named(world, "vec3A");
    let vec3_b = entity_named(world, "vec3B");
    assert_eq!(world.get::<Vec3YFrom>(vec3_a).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_b).map(|w| w.0), Some(lfo_b));
    assert_eq!(world.get::<TranslationFrom>(cube_a).map(|w| w.0), Some(vec3_a));
    assert_eq!(world.get::<TranslationFrom>(cube_b).map(|w| w.0), Some(vec3_b));
    assert_eq!(world.get::<ChildOf>(cube_a).map(|c| c.parent()), Some(group));
    assert_eq!(world.get::<ChildOf>(cube_b).map(|c| c.parent()), Some(group));
}

#[test]
fn demo_document_round_trips_through_the_world() {
    // Completeness against the demo's real component/wire set — the claim
    // the fixture-only `document_to_world_to_document_is_stable` in
    // `sway-graph` cannot make on its own.
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();
    sway_graph::project::apply(app.world_mut(), &document);
    let once = to_document(app.world_mut());

    let mut second = demo_app();
    let diagnostics = sway_graph::project::apply(second.world_mut(), &once);
    assert!(diagnostics.is_clean(), "re-apply of emitted doc: {:?}", diagnostics.items);
    let twice = to_document(second.world_mut());

    assert_eq!(once, twice);
}
