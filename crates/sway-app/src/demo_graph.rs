//! The M2b demo graph, built in Rust. Design §8.
//!
//! ```text
//! Grid.geo ──→ Displace.geo ──→ Mesh.geo,  StandardMaterial.material ──→ Mesh.material
//! Mesh.spatial ──→ Group("root").children[0]
//! MidiCC 74.value ──→ Displace.amount
//! MidiNote.note_on ──→ Envelope.triggers[0].value ──→ Rgb.r
//! LFO.value ──→ Group("root").rotation_y
//! ```

use bevy::prelude::*;
use sway_geo::{Displace, DisplaceInlets, DisplaceState, Grid, GridInlets, GridState};
use sway_graph::{
    Edge, EdgeFrom, EdgeTo, Endpoint, EditorPos, GraphNode, NodeId, NodeType, NodeTypeRegistry,
    PortArena, compile,
};
use sway_nodes::{
    Envelope, EnvelopeInlets, EnvelopeState, Group, GroupInlets, GroupState, LFO, LfoInlets,
    LfoState, MaterialState, MeshNode, MeshNodeInlets, MeshNodeState, MidiCC, MidiCCInlets,
    MidiCCState, MidiNote, MidiNoteInlets, MidiNoteState, Rgb, RgbInlets, RgbState,
    StandardMaterialNode, StandardMaterialInlets,
};

fn node_type_id<N: NodeType>(world: &World) -> sway_graph::NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

fn edge(world: &mut World, from: Entity, from_field: u16, to: Entity, to_field: u16, to_index: u16) {
    world.spawn((
        Edge {
            from: Endpoint::field(from_field),
            to: Endpoint { field: to_field, index: to_index },
        },
        EdgeFrom(from),
        EdgeTo(to),
    ));
}

pub fn setup_demo_graph(world: &mut World) {
    let mut next = 0u32;
    let mut id = || {
        next += 1;
        NodeId(next - 1)
    };

    let grid = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Grid>(world) },
            GridInlets { rows: 48, cols: 48, width: 4.0, height: 4.0 },
            GridState,
            EditorPos(Vec2::new(20.0, 380.0)),
        ))
        .id();
    let displace = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Displace>(world) },
            DisplaceInlets { amount: 0.2, frequency: 3.0, ..Default::default() },
            DisplaceState,
            EditorPos(Vec2::new(240.0, 380.0)),
        ))
        .id();
    let mesh = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MeshNode>(world) },
            MeshNodeInlets::default(),
            MeshNodeState::default(),
            EditorPos(Vec2::new(900.0, 200.0)),
        ))
        .id();
    let material = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<StandardMaterialNode>(world) },
            StandardMaterialInlets::default(),
            MaterialState::default(),
            EditorPos(Vec2::new(680.0, 20.0)),
        ))
        .id();
    let rgb = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Rgb>(world) },
            RgbInlets { r: 0.1, g: 0.2, b: 0.8 },
            RgbState,
            EditorPos(Vec2::new(460.0, 20.0)),
        ))
        .id();
    let root = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Group>(world) },
            GroupInlets { children: vec![Default::default(); 1], ..Default::default() },
            GroupState,
            EditorPos(Vec2::new(680.0, 260.0)),
        ))
        .id();
    let cc = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MidiCC>(world) },
            MidiCCInlets { channel: 0, cc: 74 },
            MidiCCState,
            EditorPos(Vec2::new(20.0, 140.0)),
        ))
        .id();
    let note = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MidiNote>(world) },
            MidiNoteInlets { channel: 0, note_lo: 0, note_hi: 127 },
            MidiNoteState,
            EditorPos(Vec2::new(20.0, 20.0)),
        ))
        .id();
    let envelope = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Envelope>(world) },
            EnvelopeInlets {
                triggers: vec![Default::default(); 1],
                release_triggers: vec![Default::default(); 1],
                attack: 0.01,
                decay: 0.1,
                sustain: 0.7,
                release: 0.3,
            },
            EnvelopeState::default(),
            EditorPos(Vec2::new(240.0, 20.0)),
        ))
        .id();
    let lfo = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<LFO>(world) },
            LfoInlets {
                hz: 0.1,
                shape: sway_nodes::Waveform::Saw,
                phase: 0.0,
                // A full turn (radians) of slow rotation on Group.rotation_y — clippy's
                // `approx_constant` rejects the brief's literal `3.14`, so this uses the
                // precise constant it approximates rather than suppressing the lint.
                amplitude: core::f32::consts::PI,
            },
            LfoState,
            EditorPos(Vec2::new(20.0, 260.0)),
        ))
        .id();

    // Structure: the Feeds chain, and where it enters the ChildOf tree.
    edge(world, grid, Grid::OUT_GEO, displace, Displace::IN_GEO, 0);
    edge(world, displace, Displace::OUT_GEO, mesh, MeshNode::IN_GEO, 0);
    edge(world, material, StandardMaterialNode::OUT_MATERIAL, mesh, MeshNode::IN_MATERIAL, 0);
    edge(world, mesh, MeshNode::OUT_SPATIAL, root, Group::CHILDREN, 0);
    edge(world, rgb, Rgb::OUT_COLOR, material, StandardMaterialNode::BASE_COLOR, 0);

    // Signals. CC drives displacement, so the cook gate is visible on stage
    // rather than only in tests (design §8).
    edge(world, cc, MidiCC::OUT_VALUE, displace, Displace::AMOUNT, 0);
    edge(world, note, MidiNote::OUT_NOTE_ON, envelope, Envelope::TRIGGERS, 0);
    edge(world, note, MidiNote::OUT_NOTE_OFF, envelope, Envelope::RELEASE_TRIGGERS, 0);
    edge(world, envelope, Envelope::OUT_VALUE, rgb, Rgb::R, 0);
    edge(world, lfo, LFO::OUT_VALUE, root, Group::ROTATION_Y, 0);

    let compiled = compile(world).expect("the demo graph must compile");
    world
        .resource_mut::<PortArena>()
        .resize(compiled.slots_len);
    world.insert_resource(compiled);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use sway_geo::{GeoNodesPlugin, Geometry};
    use sway_graph::{CompiledGraph, GraphPlugin};
    use sway_nodes::{GroupState, SceneNodesPlugin, SignalNodesPlugin};

    /// A headless `App` with every plugin the demo graph needs.
    ///
    /// DEVIATION from the brief's literal fixture: sets `Time::<Fixed>` and
    /// `TimeUpdateStrategy::FixedTimesteps(1)` here (the brief's version set
    /// neither), and burns one warm-up `app.update()` before returning.
    /// Without both, `the_demo_graph_compiles_and_cooks_a_mesh`'s single
    /// `app.update()` call can never run `FixedUpdate`/`graph_tick`, for any
    /// implementation of `setup_demo_graph`: Bevy's very first `Time::<Real>`
    /// update on a freshly-built `App` always reports a zero delta by design —
    /// `last_update` starts `None`, and both the `Automatic` and
    /// `FixedTimesteps(n)` branches of `time_system` route through
    /// `update_with_instant`, which on that first call only records
    /// `first_update`/`last_update` and returns *before* computing a delta
    /// (verified against vendored `bevy_time-0.19.0/src/real.rs:99-108`).  So
    /// the fixed-timestep accumulator can never reach its threshold on frame
    /// 0, no matter how much wall-clock time actually elapsed beforehand or
    /// what the implementation does. Confirmed empirically too: with the
    /// brief's literal fixture, `GraphTickCount` stayed `0` and
    /// `Time::<Real>::delta()` was `0ns` after the sole `app.update()`, and
    /// the test failed identically over five repeated runs (not flaky).
    /// `sway-graph/src/test_nodes.rs`'s `headless_app`/`structure_app` hit the
    /// identical hazard (see their doc comments) and use exactly this
    /// warm-up-tick recipe; Task 7's report documents applying the same fix
    /// to `structure_app` for the same reason. This changes test harness
    /// setup only — the property each test below asserts is unchanged.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins((GraphPlugin, SignalNodesPlugin, GeoNodesPlugin, SceneNodesPlugin));
        app.update();
        app
    }

    #[test]
    fn the_demo_graph_compiles_and_cooks_a_mesh() {
        let mut app = app();
        setup_demo_graph(app.world_mut());
        assert!(app.world().get_resource::<CompiledGraph>().is_some());

        app.update();

        let mut geometries = app.world_mut().query::<&Geometry>();
        assert!(
            geometries.iter(app.world()).count() >= 2,
            "Grid and Displace must both have cooked"
        );
        let mut meshes = app.world_mut().query::<&Mesh3d>();
        assert_eq!(meshes.iter(app.world()).count(), 1, "the Mesh node draws");
    }

    #[test]
    fn the_mesh_is_parented_under_the_root_group() {
        use bevy::ecs::hierarchy::ChildOf;
        use sway_nodes::MeshNodeState;

        let mut app = app();
        setup_demo_graph(app.world_mut());

        // Exactly one ChildOf in the demo graph, and it must run from the
        // Mesh node to the root Group — the one place a Feeds chain enters
        // the ChildOf tree (design §8).
        let mut children = app
            .world_mut()
            .query_filtered::<(Entity, &ChildOf), With<MeshNodeState>>();
        let found: Vec<(Entity, Entity)> = children
            .iter(app.world())
            .map(|(entity, parent)| (entity, parent.0))
            .collect();
        assert_eq!(found.len(), 1, "exactly one Mesh node, and it is parented");

        let (mesh_entity, parent_entity) = found[0];
        assert!(
            app.world().get::<GroupState>(parent_entity).is_some(),
            "the Mesh node's parent must be the root Group"
        );
        assert_ne!(mesh_entity, parent_entity);
    }
}
