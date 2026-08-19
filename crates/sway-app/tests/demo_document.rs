//! The demo document's only non-visual coverage.
//!
//! Parses and loads the real `assets/demo.sway.ron` (format version 3), then
//! asserts the graph against the document's own comment-drawn diagram. A
//! renamed node kind, a malformed inlets payload, or a dropped
//! `register_node_kind` call would otherwise leave the suite green and only
//! surface when a human ran the app.
//!
//!   midiTime ──time──▶ lfoA, lfoB
//!   lfoA ──amplitude──▶ lfoB
//!   lfoA ──y──▶ vec3A ──translation──▶ cubeA ─┐
//!   lfoB ──y──▶ vec3B ──translation──▶ cubeB ─┤─children─▶ group
//!   mat  ──material──▶ cubeA, cubeB, cubeC              │
//!   cube ──mesh──────▶ cubeA, cubeB, cubeC ─────────────┘
//!
//! `cubeC` carries an authored pose and no translation edge — the one
//! mesh in the demo whose translate-drag holds (M7 Task 15's exit criterion
//! needs this; cubeA/cubeB spring back on the next tick by design).
//!
//! The three cubes share **one** `MeshAsset` node: geometry, material and
//! placement are separate nodes, so one mesh serves several placements and
//! the sharing is an edge fan-out rather than three copies of a path.

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::reflect::TypeRegistry;
use sway_document::v3;
use sway_graph::graph::{Graph, NodeId, Part, path};

const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

/// A registry with every node kind the demo names, and nothing that needs a
/// render device: `register_runtime_node_kinds` is the schema-only half of
/// `RuntimeNodesPlugin` (the other half adds the sprite material's render
/// pipeline, which needs plugins this test has no use for).
fn demo_registry_app() -> App {
    let mut app = App::new();
    let (_tx, _rx) = crossbeam_channel::unbounded::<()>();
    app.add_plugins((
        bevy::app::TaskPoolPlugin::default(),
        AssetPlugin::default(),
        sway_nodes::GraphNodesPlugin,
        sway_midi::MidiGraphNodesPlugin,
    ));
    // `MidiPlugin` only for the `Transport` resource the `MidiTime` node
    // reads; nothing here ticks the graph.
    let _ = _rx;
    sway_runtime::nodes::register_runtime_node_kinds(&mut app);
    app
}

fn loaded() -> (Graph, v3::StableIds, App) {
    let app = demo_registry_app();
    let doc = v3::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
    let type_registry = app.world().resource::<AppTypeRegistry>().clone();
    let (graph, ids, diagnostics) = {
        let registry = type_registry.read();
        v3::load(&doc, &registry)
    };
    assert!(
        diagnostics.is_clean(),
        "the demo document should load clean against the current registry, got: {:?}",
        diagnostics.items
    );
    (graph, ids, app)
}

fn node_of(ids: &v3::StableIds, id: &str) -> NodeId {
    ids.node_of(id)
        .unwrap_or_else(|| panic!("demo document has no node \"{id}\""))
}

/// Every edge `(from_id.from_path -> to_id.to_path, slot)`, as document ids,
/// so the assertions below read like the header diagram.
fn edges(graph: &Graph, ids: &v3::StableIds) -> Vec<(String, String, String, String, i32)> {
    graph
        .edges()
        .iter()
        .map(|edge| {
            (
                ids.id_of(edge.src.node).unwrap_or("?").to_string(),
                edge.src.path.clone(),
                ids.id_of(edge.dst.node).unwrap_or("?").to_string(),
                edge.dst.path.clone(),
                edge.slot,
            )
        })
        .collect()
}

fn has_edge(
    graph: &Graph,
    ids: &v3::StableIds,
    from: &str,
    src: &str,
    to: &str,
    dst: &str,
) -> bool {
    edges(graph, ids)
        .iter()
        .any(|(a, b, c, d, _)| a == from && b == src && c == to && d == dst)
}

fn kind_of(graph: &Graph, node: NodeId) -> &'static str {
    graph.get(node).expect("a live node").kind()
}

fn float_inlet(graph: &Graph, node: NodeId, field: &str) -> f32 {
    path::resolve(graph.get(node).expect("a live node"), Part::Inlets, field)
        .and_then(|value| value.try_downcast_ref::<f32>().copied())
        .unwrap_or_else(|| panic!("no f32 inlet \"{field}\""))
}

fn pose_of(graph: &Graph, node: NodeId) -> Transform {
    let value = graph.get(node).expect("a live node");
    let field = |name: &str| {
        path::resolve(value, Part::Inlets, name).unwrap_or_else(|| panic!("a scene node's {name}"))
    };
    Transform {
        translation: field("translation")
            .try_downcast_ref::<Vec3>()
            .copied()
            .expect("translation is Vec3"),
        rotation: field("rotation")
            .try_downcast_ref::<Quat>()
            .copied()
            .expect("rotation is Quat"),
        scale: field("scale")
            .try_downcast_ref::<Vec3>()
            .copied()
            .expect("scale is Vec3"),
    }
}

#[test]
fn the_demo_document_parses_as_version_3() {
    let doc = v3::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
    assert_eq!(doc.version, v3::FORMAT_VERSION);
}

#[test]
fn the_demo_document_loads_clean_and_holds_every_node() {
    let (graph, ids, _app) = loaded();

    let mut named: Vec<String> = graph
        .node_ids()
        .into_iter()
        .map(|node| {
            ids.id_of(node)
                .expect("every node has a stable id")
                .to_string()
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            "camera",
            "colorSeq",
            "cube",
            "cubeA",
            "cubeB",
            "cubeC",
            "depthSeq",
            "group",
            "lfoA",
            "lfoB",
            "mat",
            "midiTime",
            "spriteMat",
            "spriteMat2",
            "spriteOsc",
            "spriteOsc2",
            "spritePlane",
            "spritePlane2",
            "spritePlaneMesh",
            "spritePlaneMesh2",
            "spriteRemap",
            "spriteRemap2",
            "sun",
            "vec3A",
            "vec3B",
        ],
        "exactly the demo's 25 nodes",
    );
}

#[test]
fn geometry_material_and_placement_are_separate_nodes() {
    let (graph, ids, _app) = loaded();

    let cube = node_of(&ids, "cube");
    assert!(
        kind_of(&graph, cube).ends_with("::MeshAsset"),
        "the cube's geometry is a producer node",
    );
    for placement in ["cubeA", "cubeB", "cubeC"] {
        let node = node_of(&ids, placement);
        assert!(
            kind_of(&graph, node).ends_with("::MeshNode"),
            "{placement} is a placement, not geometry",
        );
        assert!(has_edge(&graph, &ids, "cube", "mesh", placement, "mesh"));
        assert!(has_edge(
            &graph, &ids, "mat", "material", placement, "material"
        ));
    }

    // The point of the split: one `cube.gltf` in the whole document.
    let mesh_nodes: Vec<NodeId> = graph
        .node_ids()
        .into_iter()
        .filter(|node| kind_of(&graph, *node).ends_with("::MeshAsset"))
        .collect();
    assert_eq!(mesh_nodes.len(), 1, "one mesh serves all three placements");
    // Comments mention the path too; only the authored inlet counts.
    assert_eq!(
        DEMO_DOCUMENT
            .matches("path: \"cube.gltf#Mesh0/Primitive0\"")
            .count(),
        1
    );
}

#[test]
fn a_marker_edge_carries_no_value_but_still_orders() {
    let (graph, ids, _app) = loaded();
    let cube = node_of(&ids, "cube");
    let cube_a = node_of(&ids, "cubeA");

    let mesh_edge = graph
        .edges()
        .iter()
        .find(|edge| edge.src.node == cube && edge.dst.node == cube_a)
        .expect("cube.mesh -> cubeA.mesh");
    assert!(mesh_edge.valueless, "a mesh connection carries no value");

    let order = &graph.order().order;
    let at = |node: NodeId| order.iter().position(|other| *other == node);
    assert!(
        at(cube) < at(cube_a),
        "the mesh producer is still ordered before the placement that reads it",
    );
}

#[test]
fn the_cube_chain_is_wired_as_the_header_draws_it() {
    let (graph, ids, _app) = loaded();

    assert!(has_edge(&graph, &ids, "midiTime", "out", "lfoA", "time"));
    assert!(has_edge(&graph, &ids, "midiTime", "out", "lfoB", "time"));
    assert!(has_edge(&graph, &ids, "lfoA", "out", "lfoB", "amplitude"));
    assert!(has_edge(&graph, &ids, "lfoA", "out", "vec3A", "y"));
    assert!(has_edge(&graph, &ids, "lfoB", "out", "vec3B", "y"));
    assert!(has_edge(
        &graph,
        &ids,
        "vec3A",
        "out",
        "cubeA",
        "translation"
    ));
    assert!(has_edge(
        &graph,
        &ids,
        "vec3B",
        "out",
        "cubeB",
        "translation"
    ));

    // cubeC is the one placement with no translation edge, and it keeps its
    // authored pose.
    let cube_c = node_of(&ids, "cubeC");
    assert!(
        graph
            .edges_into(cube_c)
            .all(|edge| edge.dst.path != "translation"),
        "cubeC must not be driven",
    );
    assert_eq!(
        pose_of(&graph, cube_c).translation,
        Vec3::new(0.0, 1.6, -0.8),
    );
}

#[test]
fn the_group_orders_its_children_by_slot() {
    let (graph, ids, _app) = loaded();
    let mut children: Vec<(i32, String)> = edges(&graph, &ids)
        .into_iter()
        .filter(|(_, _, to, dst, _)| to == "group" && dst == "children")
        .map(|(from, _, _, _, slot)| (slot, from))
        .collect();
    children.sort();
    assert_eq!(
        children,
        vec![
            (10, "cubeA".to_string()),
            (20, "cubeB".to_string()),
            (30, "cubeC".to_string()),
        ],
        "sparse slots, so a fourth cube fits between two without renumbering",
    );
}

#[test]
fn the_sprite_layers_share_one_colour_run_and_one_depth_run() {
    let (graph, ids, _app) = loaded();

    for (osc, remap, material) in [
        ("spriteOsc", "spriteRemap", "spriteMat"),
        ("spriteOsc2", "spriteRemap2", "spriteMat2"),
    ] {
        assert!(has_edge(&graph, &ids, "midiTime", "out", osc, "time"));
        assert!(has_edge(&graph, &ids, osc, "out", remap, "input"));
        assert!(has_edge(&graph, &ids, remap, "out", material, "frame"));
        assert!(has_edge(
            &graph, &ids, "colorSeq", "sequence", material, "color"
        ));
        assert!(has_edge(
            &graph, &ids, "depthSeq", "sequence", material, "depth"
        ));
    }

    // The arithmetic the header records, pinned so a stray edit is caught.
    assert_eq!(
        float_inlet(&graph, node_of(&ids, "spriteRemap"), "out_max"),
        30.0,
        "exactly the frame count: the read-side clamp is what bounds it",
    );
    assert_eq!(
        float_inlet(&graph, node_of(&ids, "spriteOsc2"), "phase"),
        0.5,
        "the two layers are never on the same frame",
    );
    assert!(
        float_inlet(&graph, node_of(&ids, "spriteMat"), "depth_range") < 0.0
            && float_inlet(&graph, node_of(&ids, "spriteMat2"), "depth_range") < 0.0,
        "depth_range is negative on both layers (see the header's depth-sign note)",
    );
}

#[test]
fn each_sprite_plane_has_its_own_mesh_and_material() {
    let (graph, ids, _app) = loaded();

    for (mesh, placement, material) in [
        ("spritePlaneMesh", "spritePlane", "spriteMat"),
        ("spritePlaneMesh2", "spritePlane2", "spriteMat2"),
    ] {
        assert!(kind_of(&graph, node_of(&ids, mesh)).ends_with("::PlaneMesh"));
        assert!(kind_of(&graph, node_of(&ids, placement)).ends_with("::MeshNode"));
        assert!(has_edge(&graph, &ids, mesh, "mesh", placement, "mesh"));
        assert!(has_edge(
            &graph, &ids, material, "material", placement, "material"
        ));
    }

    // spritePlane interpenetrates cubeC; spritePlane2 carries the 30° yaw.
    assert_eq!(
        pose_of(&graph, node_of(&ids, "spritePlane")).translation,
        Vec3::new(0.0, 1.6, -0.6),
    );
    let yawed = pose_of(&graph, node_of(&ids, "spritePlane2"));
    assert_eq!(yawed.translation, Vec3::new(0.3, 1.5, -0.5));
    assert!(
        (yawed.rotation.y - 0.258819).abs() < 1e-5,
        "a rotated second layer, so the two reliefs interleave",
    );
}

#[test]
fn the_document_round_trips_through_a_save() {
    let (graph, mut ids, app) = loaded();
    let type_registry = app.world().resource::<AppTypeRegistry>().clone();
    let registry: &TypeRegistry = &type_registry.read();

    let once = v3::to_document(&graph, registry, &mut ids).expect("saves");
    let text = v3::to_ron(&once).expect("serializes");

    let reparsed = v3::parse(&text).expect("the emitted document parses");
    let (graph2, mut ids2, diagnostics) = v3::load(&reparsed, registry);
    assert!(diagnostics.is_clean(), "{:?}", diagnostics.items);
    let twice = v3::to_document(&graph2, registry, &mut ids2).expect("saves again");

    assert_eq!(once, twice, "a save/load/save round trip is a fixed point");
    assert_eq!(graph2.len(), graph.len());
    assert_eq!(graph2.edges().len(), graph.edges().len());
}
