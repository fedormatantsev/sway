//! The M2b demo graph, built in Rust. Design §8.
//!
//! ```text
//! Grid ──feeds(geo)──→ Displace ──feeds(geo)──→ Mesh ←──feeds(material)── StandardMaterial ← Rgb
//!                                                └──parent──→ Group(root)
//! MidiCC 74 ────────param→ Displace.amount
//! MidiNote → Envelope ─param→ Rgb.r
//! LFO ──────────────param→ Group.rotation.y
//! ```

use bevy::prelude::*;
use sway_geo::{Displace, DisplaceParams, DisplaceState, Grid, GridParams, GridState};
use sway_graph::{
    EdgeFrom, EdgeTo, FeedsEdge, GraphNode, NodeId, NodeType, NodeTypeRegistry, ParamEdge,
    ParentEdge, PortArena, PortKind, compile,
};
use sway_nodes::{
    Envelope, EnvelopeParams, EnvelopeState, Group, GroupParams, GroupState, LFO, LfoParams,
    LfoState, MaterialState, MeshNode, MeshNodeParams, MeshNodeState, MidiCC, MidiCCParams,
    MidiCCState, MidiNote, MidiNoteParams, MidiNoteState, Rgb, RgbParams, RgbState,
    StandardMaterialNode, StandardMaterialParams,
};

fn node_type_id<N: NodeType>(world: &World) -> sway_graph::NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

fn param(world: &mut World, from: Entity, sp: u16, to: Entity, tp: u16, kind: PortKind) {
    world.spawn((
        ParamEdge { source_port: sp, target_port: tp, kind },
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
            GridParams { rows: 48, cols: 48, width: 4.0, height: 4.0 },
            GridState,
        ))
        .id();
    let displace = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Displace>(world) },
            DisplaceParams { amount: 0.2, frequency: 3.0 },
            DisplaceState,
        ))
        .id();
    let mesh = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MeshNode>(world) },
            MeshNodeParams::default(),
            MeshNodeState::default(),
        ))
        .id();
    let material = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<StandardMaterialNode>(world) },
            StandardMaterialParams::default(),
            MaterialState::default(),
        ))
        .id();
    let rgb = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Rgb>(world) },
            RgbParams { r: 0.1, g: 0.2, b: 0.8 },
            RgbState,
        ))
        .id();
    let root = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Group>(world) },
            GroupParams::default(),
            GroupState,
        ))
        .id();
    let cc = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MidiCC>(world) },
            MidiCCParams { channel: 0, cc: 74 },
            MidiCCState,
        ))
        .id();
    let note = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<MidiNote>(world) },
            MidiNoteParams { channel: 0, note_lo: 0, note_hi: 127 },
            MidiNoteState,
        ))
        .id();
    let envelope = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Envelope>(world) },
            EnvelopeParams {
                trigger: sway_graph::Event::default(),
                release_trigger: sway_graph::Event::default(),
                attack: 0.01,
                decay: 0.1,
                sustain: 0.7,
                release: 0.3,
            },
            EnvelopeState::default(),
        ))
        .id();
    let lfo = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<LFO>(world) },
            LfoParams {
                hz: 0.1,
                shape: sway_nodes::Waveform::Saw,
                phase: 0.0,
                // A full turn (radians) of slow rotation on Group.rotation_y — clippy's
                // `approx_constant` rejects the brief's literal `3.14`, so this uses the
                // precise constant it approximates rather than suppressing the lint.
                amplitude: core::f32::consts::PI,
            },
            LfoState,
        ))
        .id();

    // Structure: the Feeds chain, and where it enters the ChildOf tree.
    world.spawn((FeedsEdge { slot: Displace::IN_GEO }, EdgeFrom(grid), EdgeTo(displace)));
    world.spawn((FeedsEdge { slot: MeshNode::IN_GEO }, EdgeFrom(displace), EdgeTo(mesh)));
    world.spawn((
        FeedsEdge { slot: MeshNode::IN_MATERIAL },
        EdgeFrom(material),
        EdgeTo(mesh),
    ));
    world.spawn((ParentEdge, EdgeFrom(mesh), EdgeTo(root)));

    // Signals. CC drives displacement, so the cook gate is visible on stage
    // rather than only in tests (design §8).
    param(world, cc, MidiCC::OUT_VALUE, displace, Displace::AMOUNT, PortKind::Continuous);
    param(world, note, MidiNote::OUT_NOTE_ON, envelope, Envelope::TRIGGER, PortKind::Event);
    param(
        world,
        note,
        MidiNote::OUT_NOTE_OFF,
        envelope,
        Envelope::RELEASE_TRIGGER,
        PortKind::Event,
    );
    param(world, envelope, Envelope::OUT_VALUE, rgb, Rgb::R, PortKind::Continuous);
    param(world, rgb, Rgb::OUT_COLOR, material, StandardMaterialNode::BASE_COLOR, PortKind::Continuous);
    param(world, lfo, LFO::OUT_VALUE, root, Group::ROTATION_Y, PortKind::Continuous);

    let compiled = compile(world).expect("the demo graph must compile");
    world
        .resource_mut::<PortArena>()
        .resize(compiled.continuous_len, compiled.events_len);
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
