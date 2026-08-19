//! Projection, end to end (task 5.8).
//!
//! Every test drives a real [`Graph`] through the real [`ProjectionPlugin`]
//! chain in a device-free `App`: `AssetPlugin` plus the asset types the
//! projectors touch, no renderer, following `frame_sequence.rs`'s and
//! `sprite_material.rs`'s existing pattern.

use bevy::asset::{AssetPlugin, LoadedFolder, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sway_graph::graph::{ConnectError, Graph, Node, NodeId, Port};

use crate::nodes::frame_sequence::{FrameSequence, FrameSequenceIn, FrameSequenceState};
use crate::nodes::mesh::{MeshAsset, MeshAssetIn, PlaneMesh, PlaneMeshIn};
use crate::nodes::pbr_material::{PbrMaterial, PbrMaterialIn};
use crate::nodes::protocol;
use crate::nodes::scene::{Camera, DirectionalLight, Group, MeshNode, PointLight};
use crate::nodes::sprite_material::{SpriteMaterial, SpriteMaterialIn};
use crate::project::{MaterialAttachment, NodeEntities, ProjectionPlugin, dirty_in_graph_order};
use crate::sprite_material::SpriteMaterialAsset;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// `AssetPlugin` plus the four asset types the projectors touch and the
/// transform propagation a parented scene node needs. No device, no renderer.
///
/// The default `Image` and `StandardMaterial` handles are seeded because
/// `ImagePlugin` and `PbrPlugin` seed real fallbacks there in every real app.
/// Without them a projector that only ever calls `get_mut` would look correct
/// here and would overwrite the engine's shared default in the real app.
fn projection_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::app::TaskPoolPlugin::default(),
        AssetPlugin::default(),
        bevy::transform::TransformPlugin,
    ));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<SpriteMaterialAsset>();
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(&Handle::default(), Image::default())
        .expect("seeding the default handle succeeds");
    app.world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .insert(&Handle::default(), StandardMaterial::default())
        .expect("seeding the default handle succeeds");
    crate::nodes::register_runtime_node_kinds(&mut app);
    app.add_plugins(ProjectionPlugin);
    app
}

fn graph(app: &mut App) -> Mut<'_, Graph> {
    app.world_mut().resource_mut::<Graph>()
}

fn insert<T: Reflect + TypePath>(app: &mut App, value: T) -> NodeId {
    graph(app).insert(Node::of(Vec2::ZERO, value))
}

fn connect(app: &mut App, src: (NodeId, &str), dst: (NodeId, &str)) {
    graph(app)
        .connect(Port::new(src.0, src.1), Port::new(dst.0, dst.1), 0)
        .expect("a legal connection");
}

fn entity_of(app: &App, node: NodeId) -> Option<Entity> {
    app.world().resource::<NodeEntities>().entity(node)
}

/// A solid frame, so a layer's contents are identifiable by one byte.
fn frame(size: u32, fill: u8) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![fill; (size * size * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// A frame sequence whose folder is already enumerated, so the test needs no
/// assets directory and no async arrival. Everything after enumeration —
/// readiness, ordering, assembly, handle discipline — is the real code path.
fn insert_sequence(app: &mut App, layers: u32, size: u32) -> NodeId {
    let mut images: Vec<Handle<Image>> = Vec::new();
    for layer in 0..layers {
        images.push(
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(frame(size, layer as u8 + 1)),
        );
    }
    let folder = app
        .world_mut()
        .resource_mut::<Assets<LoadedFolder>>()
        .add(LoadedFolder {
            handles: images.into_iter().map(Handle::untyped).collect(),
        });
    insert(
        app,
        FrameSequence {
            inlets: FrameSequenceIn {
                folder: "frames".into(),
                ..default()
            },
            state: FrameSequenceState {
                folder_path: "frames".into(),
                folder,
                pending: true,
                ..default()
            },
            ..default()
        },
    )
}

// ---------------------------------------------------------------------------
// 5.1 — the projector layer
// ---------------------------------------------------------------------------

#[test]
fn a_scene_node_gets_an_entity_and_a_producer_gets_none() {
    // `architecture`: "A node MAY produce an entity, an asset, a component on
    // another node's entity, or nothing at all" — graph shape and world shape
    // are not assumed to match, and the map is the only place they meet.
    let mut app = projection_app();
    let mesh = insert(
        &mut app,
        MeshAsset {
            inlets: MeshAssetIn {
                path: "cube.gltf#Mesh0/Primitive0".into(),
            },
            ..default()
        },
    );
    let placement = insert(&mut app, MeshNode::default());

    app.update();

    let entity = entity_of(&app, placement).expect("a scene node owns an entity");
    assert_eq!(
        entity_of(&app, mesh),
        None,
        "a producer owns an asset and no entity"
    );
    assert_eq!(
        app.world().resource::<NodeEntities>().node(entity),
        Some(placement),
        "and picking resolves back to it, identity only"
    );
}

#[test]
fn dirty_nodes_are_visited_in_graph_order() {
    // Task 5.1's "projector ordering by graph order". A marker edge
    // propagates nothing but stays in the sort, which is the whole mechanism:
    // without it the sequence and the material would be visited in id order,
    // and a material created before its sequence would read an empty texture
    // for a frame.
    let mut app = projection_app();
    let material = insert(&mut app, SpriteMaterial::default());
    let sequence = insert_sequence(&mut app, 2, 2);
    connect(
        &mut app,
        (sequence, protocol::SEQUENCE),
        (material, protocol::COLOR),
    );

    let mut graph = graph(&mut app);
    graph.rebuild_order_if_dirty();
    let order = dirty_in_graph_order(&graph);

    let material_at = order.iter().position(|id| *id == material);
    let sequence_at = order.iter().position(|id| *id == sequence);
    assert!(
        sequence_at < material_at,
        "the producer must be visited first, despite its higher id: {order:?}"
    );
}

#[test]
fn a_removed_node_takes_its_entity_and_its_asset_with_it() {
    // Task 5.8's named case, and `architecture`: "A removed node takes its
    // projection with it — no orphaned entity, asset or component remains."
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (plane, protocol::MESH),
        (placement, protocol::MESH),
    );
    app.update();

    let entity = entity_of(&app, placement).expect("projected");
    // The *id*, never a strong handle: holding one here would keep the asset
    // alive on its own and the release assertion below would pass for the
    // wrong reason — or rather, could never fail.
    let mesh_id = app
        .world()
        .get::<Mesh3d>(entity)
        .expect("the mesh reached the placement")
        .0
        .id();
    assert!(
        app.world().resource::<Assets<Mesh>>().contains(mesh_id),
        "the mesh asset exists while the node does"
    );

    graph(&mut app).remove(placement);
    graph(&mut app).remove(plane);
    app.update();
    // One more update so `Assets::track_assets` sees the dropped handle.
    app.update();

    assert_eq!(entity_of(&app, placement), None);
    assert!(
        app.world().get_entity(entity).is_err(),
        "the entity is despawned"
    );
    assert!(
        !app.world().resource::<Assets<Mesh>>().contains(mesh_id),
        "and the asset the deleted node owned is released"
    );
}

#[test]
fn the_world_is_not_an_authoring_surface() {
    // `architecture`: "WHEN a component on a projected entity is changed
    // directly in the world THEN the next projection restores it from the
    // graph AND the graph is unchanged."
    let mut app = projection_app();
    let placement = insert(
        &mut app,
        MeshNode {
            inlets: crate::nodes::scene::MeshNodeIn {
                transform: Transform::from_xyz(1.0, 2.0, 3.0),
                ..default()
            },
            ..default()
        },
    );
    app.update();
    let entity = entity_of(&app, placement).expect("projected");

    *app.world_mut()
        .get_mut::<Transform>(entity)
        .expect("projected") = Transform::from_xyz(9.0, 9.0, 9.0);
    app.update();

    assert_eq!(
        app.world().get::<Transform>(entity).map(|t| t.translation),
        Some(Vec3::new(1.0, 2.0, 3.0)),
        "the next projection restores it"
    );
    let graph = app.world().resource::<Graph>();
    let node = graph.get(placement).expect("still there");
    let inlets = node
        .value()
        .downcast_ref::<MeshNode>()
        .expect("a mesh node")
        .inlets
        .transform;
    assert_eq!(
        inlets.translation,
        Vec3::new(1.0, 2.0, 3.0),
        "and nothing flowed back into the graph"
    );
}

#[test]
fn a_settled_projection_stops_writing() {
    // The never-write-an-equal-value rule, applied to the world: a projector
    // that inserted unconditionally would mark every projected component
    // changed every frame, re-extracting and re-uploading a scene that did
    // not move.
    let mut app = projection_app();
    let material = insert(&mut app, PbrMaterial::default());
    let plane = insert(&mut app, PlaneMesh::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (plane, protocol::MESH),
        (placement, protocol::MESH),
    );
    connect(
        &mut app,
        (material, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();
    app.update();
    let entity = entity_of(&app, placement).expect("projected");

    // A tick with nothing happening: nothing may be rewritten.
    app.update();

    let world = app.world();
    let entity_ref = world.entity(entity);
    assert!(
        !entity_ref
            .get_ref::<Transform>()
            .expect("present")
            .is_changed()
    );
    assert!(
        !entity_ref
            .get_ref::<Mesh3d>()
            .expect("present")
            .is_changed()
    );
    assert!(
        !entity_ref
            .get_ref::<MeshMaterial3d<StandardMaterial>>()
            .expect("present")
            .is_changed(),
        "an idle material must not be re-attached"
    );
}

// ---------------------------------------------------------------------------
// 5.2 / 5.4 — protocols and producers
// ---------------------------------------------------------------------------

#[test]
fn an_asset_connection_carries_no_value_and_the_consumer_still_reaches_the_asset() {
    // `nodes`: "A node that owns an asset does not pass it along a
    // connection." The edge is valueless — the graph decided that from
    // `size_of_val` at connect time — and the consumer reaches the asset
    // through the connection's existence instead.
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (plane, protocol::MESH),
        (placement, protocol::MESH),
    );

    let edge_is_valueless = app
        .world()
        .resource::<Graph>()
        .edges()
        .iter()
        .all(|edge| edge.valueless);
    assert!(edge_is_valueless, "a marker edge propagates nothing");

    app.update();

    let entity = entity_of(&app, placement).expect("projected");
    let published = app
        .world()
        .resource::<Graph>()
        .get(plane)
        .and_then(|node| node.value().downcast_ref::<PlaneMesh>())
        .expect("a plane mesh")
        .state
        .handle
        .clone();
    assert_ne!(published, Handle::default(), "the producer allocated");
    assert_eq!(
        app.world().get::<Mesh3d>(entity).map(|mesh| mesh.0.clone()),
        Some(published),
        "and the consumer renders with it"
    );
}

#[test]
fn one_mesh_node_serves_three_placements_and_is_built_once() {
    // `nodes`: "One mesh serves several placements — all three render that
    // mesh AND the mesh is loaded or built once." Sharing is visible as
    // connections rather than implied by two nodes naming one path.
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());
    let placements: Vec<NodeId> = (0..3)
        .map(|_| {
            let node = insert(&mut app, MeshNode::default());
            connect(&mut app, (plane, protocol::MESH), (node, protocol::MESH));
            node
        })
        .collect();

    app.update();

    let handles: Vec<Handle<Mesh>> = placements
        .iter()
        .map(|node| {
            let entity = entity_of(&app, *node).expect("projected");
            app.world().get::<Mesh3d>(entity).expect("meshed").0.clone()
        })
        .collect();
    assert_eq!(handles[0], handles[1]);
    assert_eq!(handles[1], handles[2]);
    assert_eq!(
        app.world().resource::<Assets<Mesh>>().len(),
        1,
        "built once, not once per placement"
    );
}

#[test]
fn a_geometry_node_connected_to_nothing_has_no_transform_and_draws_nothing() {
    // `nodes`: "A geometry node has no placement." The node owns an asset, so
    // there is nothing in the world to give a transform to.
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());

    app.update();

    assert_eq!(entity_of(&app, plane), None);
    assert_eq!(
        app.world().resource::<NodeEntities>().len(),
        0,
        "nothing is drawn for it"
    );
}

#[test]
fn a_plane_mesh_rebuild_keeps_its_handle() {
    // Handle discipline: a scene node already holding the handle must see the
    // new tessellation without its `Mesh3d` — and therefore its draw — moving.
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());
    app.update();
    let before = published_plane_handle(&app, plane);

    {
        let mut graph = graph(&mut app);
        let node = graph.get_mut(plane).expect("present");
        node.value_mut()
            .downcast_mut::<PlaneMesh>()
            .expect("a plane")
            .inlets = PlaneMeshIn {
            size: Vec2::splat(2.0),
            horizontal: 1,
            vertical: 1,
        };
        graph.mark_dirty(plane);
    }
    app.update();

    assert_eq!(
        published_plane_handle(&app, plane),
        before,
        "the handle must not change under an edit"
    );
    let mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(&before)
        .expect("resolves");
    assert_eq!(
        mesh.count_vertices(),
        9,
        "1×1 subdivisions rebuilt in place"
    );
}

fn published_plane_handle(app: &App, node: NodeId) -> Handle<Mesh> {
    app.world()
        .resource::<Graph>()
        .get(node)
        .and_then(|node| node.value().downcast_ref::<PlaneMesh>())
        .expect("a plane mesh")
        .state
        .handle
        .clone()
}

#[test]
fn a_frame_sequence_publishes_one_array_texture_with_a_layer_per_frame() {
    let mut app = projection_app();
    let sequence = insert_sequence(&mut app, 3, 2);

    app.update();

    let published = app
        .world()
        .resource::<Graph>()
        .get(sequence)
        .and_then(|node| node.value().downcast_ref::<FrameSequence>())
        .expect("a sequence")
        .state
        .clone();
    assert_eq!(published.layers, 3);
    assert_ne!(published.texture, Handle::default());
    let texture = app
        .world()
        .resource::<Assets<Image>>()
        .get(&published.texture)
        .expect("resolves");
    assert_eq!(texture.texture_descriptor.size.depth_or_array_layers, 3);
}

// ---------------------------------------------------------------------------
// 5.5 — material nodes
// ---------------------------------------------------------------------------

#[test]
fn connecting_a_material_node_makes_the_scene_node_render_with_it() {
    // `nodes`: "Connecting applies the material." Nothing outside the
    // material node knows which kind it is — the node inserted
    // `MeshMaterial3d<StandardMaterial>` itself.
    let mut app = projection_app();
    let material = insert(
        &mut app,
        PbrMaterial {
            inlets: PbrMaterialIn {
                metallic: 0.25,
                ..default()
            },
            ..default()
        },
    );
    let placement = insert(&mut app, MeshNode::default());
    app.update();
    let entity = entity_of(&app, placement).expect("projected");
    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_none(),
        "a newly created scene node carries no material"
    );

    connect(
        &mut app,
        (material, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();

    let handle = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .expect("the material node attached itself")
        .0
        .clone();
    assert_eq!(
        app.world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .map(|material| material.metallic),
        Some(0.25)
    );
}

#[test]
fn disconnecting_a_material_node_stops_the_scene_node_rendering_with_it() {
    // `nodes`: "Disconnecting removes the material — nothing is drawn for it."
    let mut app = projection_app();
    let material = insert(&mut app, PbrMaterial::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (material, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();
    let entity = entity_of(&app, placement).expect("projected");
    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_some()
    );

    let edge = app.world().resource::<Graph>().edges()[0].id;
    graph(&mut app).disconnect(edge);
    app.update();

    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_none(),
        "the attachment left with the connection"
    );
    assert!(app.world().get::<MaterialAttachment>(entity).is_none());
}

#[test]
fn deleting_a_material_node_stops_the_scene_node_rendering_with_it() {
    // The case a purely dirty-driven attachment pass cannot see:
    // `Graph::remove` drops the edge without dirtying the scene node at the
    // other end, and the deleted node is gone before anything could ask it to
    // detach — which is why the attachment records how to undo itself.
    let mut app = projection_app();
    let material = insert(&mut app, PbrMaterial::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (material, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();
    let entity = entity_of(&app, placement).expect("projected");
    // The id, not a strong handle — see the note in the removal test above.
    let material_id = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(entity)
        .expect("attached")
        .0
        .id();

    graph(&mut app).remove(material);
    app.update();
    app.update();

    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_none(),
        "nothing is drawn for it any more"
    );
    assert!(
        !app.world()
            .resource::<Assets<StandardMaterial>>()
            .contains(material_id),
        "and the asset the deleted node owned is released"
    );
}

#[test]
fn editing_a_material_node_mutates_its_asset_in_place() {
    // One material node connected to two scene nodes is one asset, so an edit
    // has to reach both without the handle moving.
    let mut app = projection_app();
    let material = insert(&mut app, PbrMaterial::default());
    let first = insert(&mut app, MeshNode::default());
    let second = insert(&mut app, MeshNode::default());
    for placement in [first, second] {
        connect(
            &mut app,
            (material, protocol::MATERIAL),
            (placement, protocol::MATERIAL),
        );
    }
    app.update();
    let entities: Vec<Entity> = [first, second]
        .iter()
        .map(|node| entity_of(&app, *node).expect("projected"))
        .collect();
    let before = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(entities[0])
        .expect("attached")
        .0
        .clone();
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entities[1])
            .map(|material| material.0.clone()),
        Some(before.clone()),
        "both scene nodes share one asset"
    );

    {
        let mut graph = graph(&mut app);
        let node = graph.get_mut(material).expect("present");
        node.value_mut()
            .downcast_mut::<PbrMaterial>()
            .expect("a material")
            .inlets
            .metallic = 1.0;
        graph.mark_dirty(material);
    }
    app.update();

    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entities[0])
            .map(|material| material.0.clone()),
        Some(before.clone()),
        "the handle must not change under an edit"
    );
    assert_eq!(
        app.world()
            .resource::<Assets<StandardMaterial>>()
            .get(&before)
            .map(|material| material.metallic),
        Some(1.0)
    );
}

#[test]
fn a_scene_node_never_carries_two_material_kinds_at_once() {
    // `nodes`: connected to one kind, then to another, it "renders with
    // exactly one material AND is drawn once". Two `MeshMaterial3d`
    // components would be extracted by two `MaterialPlugin`s.
    let mut app = projection_app();
    let pbr = insert(&mut app, PbrMaterial::default());
    let sprite = insert(&mut app, SpriteMaterial::default());
    let color = insert_sequence(&mut app, 4, 2);
    let depth = insert_sequence(&mut app, 4, 2);
    connect(
        &mut app,
        (color, protocol::SEQUENCE),
        (sprite, protocol::COLOR),
    );
    connect(
        &mut app,
        (depth, protocol::SEQUENCE),
        (sprite, protocol::DEPTH),
    );
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (pbr, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();
    let entity = entity_of(&app, placement).expect("projected");
    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_some()
    );

    // Connecting a second material to a non-variadic inlet evicts the first
    // rather than doubling it — that is the graph invariant this leans on.
    connect(
        &mut app,
        (sprite, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();

    assert!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .is_none(),
        "the first kind left with its connection"
    );
    assert!(
        app.world()
            .get::<MeshMaterial3d<SpriteMaterialAsset>>(entity)
            .is_some(),
        "and the second kind attached its own"
    );
}

#[test]
fn an_incomplete_sprite_material_publishes_nothing_and_completes_when_both_runs_arrive() {
    // `runtime`: "A material whose runs are not both connected MUST render
    // nothing rather than render incorrectly." `ImagePlugin` seeds a real 1×1
    // white image at `Handle::default()`, so an asset published with a
    // missing run would draw a plain white quad.
    let mut app = projection_app();
    let sprite = insert(&mut app, SpriteMaterial::default());
    let color = insert_sequence(&mut app, 4, 2);
    connect(
        &mut app,
        (color, protocol::SEQUENCE),
        (sprite, protocol::COLOR),
    );
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (sprite, protocol::MATERIAL),
        (placement, protocol::MATERIAL),
    );
    app.update();
    let entity = entity_of(&app, placement).expect("projected");

    assert!(
        app.world()
            .get::<MeshMaterial3d<SpriteMaterialAsset>>(entity)
            .is_none(),
        "nothing is drawn while a run is missing"
    );
    assert!(
        app.world()
            .resource::<Assets<SpriteMaterialAsset>>()
            .is_empty(),
        "and no asset was allocated and then abandoned"
    );

    let depth = insert_sequence(&mut app, 4, 2);
    connect(
        &mut app,
        (depth, protocol::SEQUENCE),
        (sprite, protocol::DEPTH),
    );
    app.update();

    assert!(
        app.world()
            .get::<MeshMaterial3d<SpriteMaterialAsset>>(entity)
            .is_some(),
        "both runs connected, so the material renders"
    );
}

#[test]
fn disagreeing_run_lengths_bound_the_frame_by_the_shorter() {
    // `runtime`: "WHEN a material's colour run has 30 layers and its depth
    // run has 24 THEN a diagnostic naming the material is reported AND the
    // frame number is bounded by 24."
    let mut app = projection_app();
    let sprite = insert(
        &mut app,
        SpriteMaterial {
            inlets: SpriteMaterialIn {
                frame: 37.5,
                ..default()
            },
            ..default()
        },
    );
    let color = insert_sequence(&mut app, 30, 1);
    let depth = insert_sequence(&mut app, 24, 1);
    connect(
        &mut app,
        (color, protocol::SEQUENCE),
        (sprite, protocol::COLOR),
    );
    connect(
        &mut app,
        (depth, protocol::SEQUENCE),
        (sprite, protocol::DEPTH),
    );

    app.update();

    let state = app
        .world()
        .resource::<Graph>()
        .get(sprite)
        .and_then(|node| node.value().downcast_ref::<SpriteMaterial>())
        .expect("a sprite material")
        .state
        .clone();
    let asset = app
        .world()
        .resource::<Assets<SpriteMaterialAsset>>()
        .get(&state.handle)
        .expect("published");
    assert_eq!(
        asset.uniform.layer, 23,
        "bounded by the shorter run, not the colour run"
    );
    let reported = state.reported.expect("the mismatch is reported");
    assert!(reported.contains("30") && reported.contains("24"));
}

// ---------------------------------------------------------------------------
// 5.6 — the closed scene node set
// ---------------------------------------------------------------------------

#[test]
fn a_group_refuses_geometry() {
    // `nodes`: "WHEN a mesh node is connected to a group THEN the connection
    // is refused." Enforced by the schema: a `Group` declares no `mesh` port,
    // so there is no destination path to resolve.
    let mut app = projection_app();
    let plane = insert(&mut app, PlaneMesh::default());
    let group = insert(&mut app, Group::default());

    let refusal = graph(&mut app).connect(
        Port::new(plane, protocol::MESH),
        Port::new(group, protocol::MESH),
        0,
    );

    assert_eq!(refusal, Err(ConnectError::MissingDestinationPath));
}

#[test]
fn a_group_places_its_children_without_drawing() {
    // `nodes`: "WHEN three mesh placements are connected as children of a
    // group and the group is moved THEN all three move with it AND nothing is
    // drawn for the group itself."
    let mut app = projection_app();
    let group = insert(
        &mut app,
        Group {
            inlets: crate::nodes::scene::GroupIn {
                transform: Transform::from_xyz(10.0, 0.0, 0.0),
                ..default()
            },
            ..default()
        },
    );
    let children: Vec<NodeId> = (0..3)
        .map(|index| {
            let child = insert(
                &mut app,
                MeshNode {
                    inlets: crate::nodes::scene::MeshNodeIn {
                        transform: Transform::from_xyz(0.0, index as f32, 0.0),
                        ..default()
                    },
                    ..default()
                },
            );
            connect(
                &mut app,
                (child, protocol::CHILD),
                (group, protocol::CHILDREN),
            );
            child
        })
        .collect();

    app.update();

    let group_entity = entity_of(&app, group).expect("projected");
    assert!(
        app.world().get::<Mesh3d>(group_entity).is_none(),
        "nothing is drawn for the group itself"
    );
    for (index, child) in children.iter().enumerate() {
        let entity = entity_of(&app, *child).expect("projected");
        assert_eq!(
            app.world().get::<ChildOf>(entity).map(ChildOf::parent),
            Some(group_entity)
        );
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(entity)
                .map(|global| global.translation()),
            Some(Vec3::new(10.0, index as f32, 0.0)),
            "the group's placement reaches its children through Bevy's own propagation"
        );
    }
}

#[test]
fn every_scene_node_kind_is_projected() {
    // The set is closed and complete: a kind left out of the projector would
    // sit in the graph producing nothing, with no error anywhere.
    let mut app = projection_app();
    let mesh = insert(&mut app, MeshNode::default());
    let group = insert(&mut app, Group::default());
    let camera = insert(&mut app, Camera::default());
    let sun = insert(&mut app, DirectionalLight::default());
    let lamp = insert(&mut app, PointLight::default());

    app.update();

    let camera_entity = entity_of(&app, camera).expect("projected");
    assert!(app.world().get::<Camera3d>(camera_entity).is_some());
    assert!(
        app.world()
            .get::<bevy::prelude::DirectionalLight>(entity_of(&app, sun).expect("projected"))
            .is_some()
    );
    assert!(
        app.world()
            .get::<bevy::prelude::PointLight>(entity_of(&app, lamp).expect("projected"))
            .is_some()
    );
    for node in [mesh, group] {
        assert!(entity_of(&app, node).is_some());
    }
    assert_eq!(app.world().resource::<NodeEntities>().len(), 5);
}

#[test]
fn editing_a_light_reaches_its_component() {
    let mut app = projection_app();
    let sun = insert(&mut app, DirectionalLight::default());
    app.update();
    let entity = entity_of(&app, sun).expect("projected");

    {
        let mut graph = graph(&mut app);
        let node = graph.get_mut(sun).expect("present");
        node.value_mut()
            .downcast_mut::<DirectionalLight>()
            .expect("a light")
            .inlets
            .illuminance = 6000.0;
        graph.mark_dirty(sun);
    }
    app.update();

    assert_eq!(
        app.world()
            .get::<bevy::prelude::DirectionalLight>(entity)
            .map(|light| light.illuminance),
        Some(6000.0)
    );
}

// ---------------------------------------------------------------------------
// 5.7 — children edges into parenting
// ---------------------------------------------------------------------------

#[test]
fn a_scene_node_with_no_child_connection_is_never_given_a_parent() {
    // `nodes`: "An unparented node has no parent — its placement is not
    // relative to any other node." Inserting a parent unconditionally would
    // make every node's transform relative to something.
    let mut app = projection_app();
    let lonely = insert(
        &mut app,
        MeshNode {
            inlets: crate::nodes::scene::MeshNodeIn {
                transform: Transform::from_xyz(1.0, 0.0, 0.0),
                ..default()
            },
            ..default()
        },
    );
    let _group = insert(&mut app, Group::default());

    app.update();

    let entity = entity_of(&app, lonely).expect("projected");
    assert!(app.world().get::<ChildOf>(entity).is_none());
    assert_eq!(
        app.world()
            .get::<GlobalTransform>(entity)
            .map(|global| global.translation()),
        Some(Vec3::new(1.0, 0.0, 0.0))
    );
}

#[test]
fn disconnecting_a_child_edge_unparents_the_child() {
    let mut app = projection_app();
    let group = insert(&mut app, Group::default());
    let child = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (child, protocol::CHILD),
        (group, protocol::CHILDREN),
    );
    app.update();
    let entity = entity_of(&app, child).expect("projected");
    assert!(app.world().get::<ChildOf>(entity).is_some());

    let edge = app.world().resource::<Graph>().edges()[0].id;
    graph(&mut app).disconnect(edge);
    app.update();

    assert!(
        app.world().get::<ChildOf>(entity).is_none(),
        "the child edge is the only thing that ever inserts a parent"
    );
}

#[test]
fn deleting_a_group_does_not_despawn_the_nodes_that_were_under_it() {
    // `EntityWorldMut::despawn` takes descendants with it, so a group's
    // deletion would otherwise silently delete live scene nodes whose own
    // graph nodes are untouched.
    let mut app = projection_app();
    let group = insert(&mut app, Group::default());
    let child = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (child, protocol::CHILD),
        (group, protocol::CHILDREN),
    );
    app.update();
    let child_entity = entity_of(&app, child).expect("projected");

    graph(&mut app).remove(group);
    app.update();

    assert_eq!(entity_of(&app, child), Some(child_entity));
    assert!(
        app.world().get_entity(child_entity).is_ok(),
        "the child survives its parent"
    );
    assert!(app.world().get::<ChildOf>(child_entity).is_none());
}

#[test]
fn a_cycle_of_child_connections_leaves_the_nodes_unparented() {
    // `Graph::connect` refuses a self-connection but not a longer cycle, and
    // a cycle in `ChildOf` is an infinite loop inside Bevy's own hierarchy
    // rather than a diagnostic.
    let mut app = projection_app();
    let first = insert(&mut app, Group::default());
    let second = insert(&mut app, Group::default());
    connect(
        &mut app,
        (first, protocol::CHILD),
        (second, protocol::CHILDREN),
    );
    connect(
        &mut app,
        (second, protocol::CHILD),
        (first, protocol::CHILDREN),
    );

    app.update();

    for node in [first, second] {
        let entity = entity_of(&app, node).expect("projected");
        assert!(app.world().get::<ChildOf>(entity).is_none());
    }
}

#[test]
fn a_mesh_asset_path_becomes_a_handle_the_placement_renders() {
    // `AssetServer::load` hands back its handle synchronously, so the handle
    // exists from the first pass and only its content is ever pending — the
    // whole of design D7's "a connection is never waiting on a handle that
    // does not exist yet". The path never resolves to a real file here, and
    // that is exactly the point.
    let mut app = projection_app();
    let mesh = insert(
        &mut app,
        MeshAsset {
            inlets: MeshAssetIn {
                path: "cube.gltf#Mesh0/Primitive0".into(),
            },
            ..default()
        },
    );
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (mesh, protocol::MESH),
        (placement, protocol::MESH),
    );

    app.update();

    let published = app
        .world()
        .resource::<Graph>()
        .get(mesh)
        .and_then(|node| node.value().downcast_ref::<MeshAsset>())
        .expect("a mesh asset")
        .state
        .handle
        .clone();
    assert_ne!(published, Handle::default());
    let entity = entity_of(&app, placement).expect("projected");
    assert_eq!(
        app.world().get::<Mesh3d>(entity).map(|mesh| mesh.0.clone()),
        Some(published)
    );
}

#[test]
fn an_empty_mesh_path_publishes_nothing() {
    // What a palette click produces before anyone types a path. It must not
    // ask the asset server to load "", which logs an error every frame.
    let mut app = projection_app();
    let mesh = insert(&mut app, MeshAsset::default());
    let placement = insert(&mut app, MeshNode::default());
    connect(
        &mut app,
        (mesh, protocol::MESH),
        (placement, protocol::MESH),
    );

    app.update();

    let entity = entity_of(&app, placement).expect("projected");
    assert!(app.world().get::<Mesh3d>(entity).is_none());
}

#[test]
fn a_sequence_that_finishes_loading_reaches_a_material_that_did_not_change() {
    // The change nothing in the graph can see: a marker edge propagates no
    // value, so a sequence assembling several frames after the material was
    // authored dirties nothing by itself. A producer that publishes therefore
    // marks its consumers dirty — without that the material would render
    // nothing forever.
    let mut app = projection_app();
    let sprite = insert(&mut app, SpriteMaterial::default());
    let color = insert_sequence(&mut app, 4, 2);
    let depth = insert_sequence(&mut app, 4, 2);
    connect(
        &mut app,
        (color, protocol::SEQUENCE),
        (sprite, protocol::COLOR),
    );
    connect(
        &mut app,
        (depth, protocol::SEQUENCE),
        (sprite, protocol::DEPTH),
    );
    // Settle everything, then take the colour run away and put it back — a
    // sequence arriving late, with nothing about the material changing.
    app.update();
    {
        let mut graph = graph(&mut app);
        let node = graph.get_mut(color).expect("present");
        let sequence = node
            .value_mut()
            .downcast_mut::<FrameSequence>()
            .expect("a sequence");
        sequence.state.pending = true;
        sequence.state.texture = Handle::default();
        sequence.state.layers = 0;
    }
    app.update();

    let published = app
        .world()
        .resource::<Graph>()
        .get(sprite)
        .and_then(|node| node.value().downcast_ref::<SpriteMaterial>())
        .expect("a sprite material")
        .state
        .handle
        .clone();
    let asset = app
        .world()
        .resource::<Assets<SpriteMaterialAsset>>()
        .get(&published)
        .expect("published");
    let sequence_texture = app
        .world()
        .resource::<Graph>()
        .get(color)
        .and_then(|node| node.value().downcast_ref::<FrameSequence>())
        .expect("a sequence")
        .state
        .texture
        .clone();
    assert_eq!(
        asset.color_texture, sequence_texture,
        "the re-published run reached a material nothing else touched"
    );
}
