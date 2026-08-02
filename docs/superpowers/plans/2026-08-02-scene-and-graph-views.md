# Scene and Graph Views Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the editor's six hardcoded placeholder boxes with two read-only views onto real state — a world hierarchy pane and a graph pane showing the actual topology with live activity on continuous edges.

**Architecture:** One pure function, `sway_editor::snapshot::capture(&World) -> WorldSnapshot`, is the entire graph→UI read path; it is masonry-free and carries the bulk of the tests. `EditorPresenter::present` calls it once per frame and pushes the result into two tagged widgets (`SceneTree`, `GraphCanvas`) through `RenderRoot::edit_widget_with_tag`. The root widget becomes a nested `Split` so the Bevy viewport is a sibling of the graph canvas rather than a child of it.

**Tech Stack:** Rust 2024, `bevy_ecs`/`bevy_reflect`/`bevy_transform`/`bevy_math` 0.19.0, `sway-graph`, masonry (git rev `c5950bcb03d4f3d187a20d1159f6aa276fd056bf`), `imaging`, `kurbo`, `peniko`.

**Spec:** `docs/superpowers/specs/2026-08-02-scene-and-graph-views-design.md`

## Global Constraints

- `sway-editor` may depend on `sway-graph`, `bevy_ecs`, `bevy_reflect`, `bevy_transform`, `bevy_math`. It must **not** depend on `bevy` (the full facade), `bevy_render`, `wgpu`, `vello`, or `imaging_vello`.
- `sway-graph` may depend only on `bevy_app`, `bevy_ecs`, `bevy_reflect`, `bevy_time`, `bevy_transform`, and (this plan) `bevy_math`. Never `bevy_render`.
- All dependency versions come from `[workspace.dependencies]` in the root `Cargo.toml` via `.workspace = true`. Never write a version number in a crate manifest.
- Everything in this plan is **read-only** with respect to the graph. No task adds a write path from the editor into the world, and no task adds an editor-only write path into `graph_tick`.
- No pixel-diff tests (main design §4). Rendering is verified by eye.
- Every task ends with `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` passing before the commit.
- Commit messages end with:
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`

## File Structure

**Created:**
- `crates/sway-editor/src/snapshot.rs` — the `WorldSnapshot` types and `capture(&World)`. The only module in the crate that knows `bevy_ecs` exists. ~350 lines including tests.
- `crates/sway-editor/src/scene_tree.rs` — the `SceneTree` widget. Owns row labels as `Label` children, lays them out, paints group headers and selection.
- `crates/sway-editor/src/test_graph.rs` — `#[cfg(test)]` node types and world fixtures for `snapshot.rs`'s tests. Kept separate because it is ~150 lines of `NodeType` boilerplate that would otherwise bury the tests it serves.

**Modified:**
- `crates/sway-graph/src/edges.rs` — adds `EditorPos`.
- `crates/sway-graph/src/tick.rs` — registers `EditorPos`'s reflect type in `GraphPlugin::build`.
- `crates/sway-graph/src/lib.rs` — re-exports `EditorPos`.
- `crates/sway-graph/Cargo.toml` — adds `bevy_math`.
- `crates/sway-app/src/demo_graph.rs` — authors an `EditorPos` on each of the ten nodes.
- `crates/sway-editor/Cargo.toml` — adds the ECS dependencies.
- `crates/sway-editor/src/lib.rs` — new root widget (nested `Split`), `EditorUi::apply_snapshot`, deletes `VIEWPORT_WIDTH`/`VIEWPORT_HEIGHT`.
- `crates/sway-editor/src/node_box.rs` — gains a `Label` child, loses the drag-to-connect gesture.
- `crates/sway-editor/src/canvas.rs` — snapshot-driven, `NodeId`-keyed, loses `with_node`/`with_edge`/`with_viewport`.
- `crates/sway-app/src/presenter.rs` — calls `capture` and `apply_snapshot`.

---

### Task 1: `EditorPos`, authored on the demo graph

Node positions live on the node entity, not in the builder — the durable home for something M4 serializes and M7 writes back.

**Files:**
- Modify: `crates/sway-graph/Cargo.toml`
- Modify: `crates/sway-graph/src/edges.rs` (append after `ParentEdge`)
- Modify: `crates/sway-graph/src/tick.rs:258-265`
- Modify: `crates/sway-graph/src/lib.rs:17-20`
- Modify: `crates/sway-app/src/demo_graph.rs:46-130`
- Test: `crates/sway-graph/src/edges.rs` (new `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces: `sway_graph::EditorPos(pub Vec2)` — a `Component + Reflect`, `Vec2` from `bevy_math`. Task 2 reads it; Task 5 seeds node box positions from it.

- [ ] **Step 1: Add `bevy_math` to `sway-graph`**

In `crates/sway-graph/Cargo.toml`, under `[dependencies]`, after `bevy_ecs.workspace = true`:

```toml
bevy_math.workspace = true
```

Then update the manifest's header comment, which currently enumerates the allowed crates:

```toml
# Spec §2: bevy_app/ecs/math/reflect/time/transform only. NOT the `bevy`
# facade, NOT bevy_render. bevy_asset remains the only deferred dependency; it
# joins at M4. This manifest is the only place that constraint is enforced.
```

- [ ] **Step 2: Write the failing test**

Append to `crates/sway-graph/src/edges.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::EditorPos;
    use bevy_ecs::world::World;
    use bevy_math::Vec2;

    #[test]
    fn editor_pos_is_a_component_readable_off_a_node_entity() {
        let mut world = World::new();
        let entity = world.spawn(EditorPos(Vec2::new(20.0, 140.0))).id();
        assert_eq!(world.get::<EditorPos>(entity).map(|p| p.0), Some(Vec2::new(20.0, 140.0)));
    }
}
```

- [ ] **Step 3: Run it to make sure it fails**

Run: `cargo test -p sway-graph edges::tests::editor_pos_is_a_component -- --exact`
Expected: FAIL — `cannot find type EditorPos in this scope`.

- [ ] **Step 4: Implement `EditorPos`**

Append to `crates/sway-graph/src/edges.rs`, after `ParentEdge`. Add `use bevy_math::Vec2;` and `use bevy_reflect::Reflect;` to the file's imports.

```rust
/// Where the editor draws this node, in graph-canvas space.
///
/// Authored in the graph builder today, serialized by M4's project format,
/// and written back by drag at M7 — which is why it is a component on the
/// node entity rather than a field in whatever built the graph.
///
/// The editor treats this as a *seed*, read once when a node box first
/// appears and never again, so that a node dragged in-session is not snapped
/// back by the next frame's snapshot. Absent means "no authored position":
/// the editor falls back to a deterministic grid slot.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
pub struct EditorPos(pub Vec2);
```

- [ ] **Step 5: Register the reflect type and re-export**

In `crates/sway-graph/src/tick.rs`, in `GraphPlugin::build`, chain a `register_type` call:

```rust
impl Plugin for GraphPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PortArena::new(0, 0))
            .init_resource::<NodeTypeRegistry>()
            .init_resource::<GraphTickCount>()
            .register_type::<crate::edges::EditorPos>()
            .add_systems(FixedUpdate, graph_tick);
    }
}
```

In `crates/sway-graph/src/lib.rs`, add `EditorPos` to the `edges` re-export list (keep it alphabetical):

```rust
pub use edges::{
    EdgeFrom, EdgeTo, EditorPos, FeedsEdge, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges,
    ParamEdge, ParentEdge, PortKind,
};
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p sway-graph edges::tests::editor_pos_is_a_component -- --exact`
Expected: PASS.

- [ ] **Step 7: Author positions on the demo graph**

In `crates/sway-app/src/demo_graph.rs`, add `EditorPos` to the `sway_graph` import list and `use bevy::math::Vec2;` is already available via `bevy::prelude::*`.

Add an `EditorPos` to each of the ten spawns. The layout follows the module's own dataflow diagram, left to right, at a 220 × 120 pitch (the node box is 160 × 72):

| Node | `EditorPos` |
|---|---|
| `note` (MidiNote) | `(20.0, 20.0)` |
| `cc` (MidiCC) | `(20.0, 140.0)` |
| `lfo` (LFO) | `(20.0, 260.0)` |
| `grid` (Grid) | `(20.0, 380.0)` |
| `envelope` (Envelope) | `(240.0, 20.0)` |
| `displace` (Displace) | `(240.0, 380.0)` |
| `rgb` (Rgb) | `(460.0, 20.0)` |
| `material` (StandardMaterial) | `(680.0, 20.0)` |
| `root` (Group) | `(680.0, 260.0)` |
| `mesh` (Mesh) | `(900.0, 200.0)` |

For example, `grid` becomes:

```rust
    let grid = world
        .spawn((
            GraphNode { id: id(), node_type: node_type_id::<Grid>(world) },
            GridParams { rows: 48, cols: 48, width: 4.0, height: 4.0 },
            GridState,
            EditorPos(Vec2::new(20.0, 380.0)),
        ))
        .id();
```

Apply the same shape to the other nine.

- [ ] **Step 8: Verify the whole workspace still builds and passes**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS. The existing `demo_graph` tests must be unaffected — `EditorPos` participates in no compiler pass.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-graph/Cargo.toml crates/sway-graph/src/edges.rs \
        crates/sway-graph/src/tick.rs crates/sway-graph/src/lib.rs \
        crates/sway-app/src/demo_graph.rs
git commit -m "$(cat <<'EOF'
feat(graph): EditorPos, authored on the demo graph

A node's editor position is a component on the node entity, not a field in
whatever built the graph: M4 serializes it and M7 writes it back, and both
are cheaper against a component.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `snapshot` — nodes and edges

The read path's first half. Pure, masonry-free, and where most of this feature's tests live.

**Files:**
- Modify: `crates/sway-editor/Cargo.toml`
- Create: `crates/sway-editor/src/test_graph.rs`
- Create: `crates/sway-editor/src/snapshot.rs`
- Modify: `crates/sway-editor/src/lib.rs:7-9` (module list)

**Interfaces:**
- Consumes: `sway_graph::{EditorPos, GraphNode, NodeId, ParamEdge, FeedsEdge, ParentEdge, EdgeFrom, EdgeTo, PortKind, PortArena, CompiledGraph, NodeTypeRegistry}`.
- Produces:
  - `sway_editor::snapshot::WorldSnapshot { tree: Vec<TreeRow>, nodes: Vec<NodeView>, edges: Vec<EdgeView> }` (`tree` stays empty until Task 3)
  - `NodeView { entity: Entity, id: NodeId, name: String, pos: Option<kurbo::Point> }`
  - `EdgeView { from: usize, to: usize, kind: EdgeKind, activity: Option<f32> }`
  - `EdgeKind::{Param, Feeds}`
  - `pub fn capture(world: &World) -> WorldSnapshot`
  - `pub fn short_type_name(path: &str) -> String`

- [ ] **Step 1: Add the ECS dependencies**

In `crates/sway-editor/Cargo.toml`:

```toml
[dependencies]
masonry.workspace = true
masonry_core.workspace = true
imaging.workspace = true
kurbo.workspace = true
peniko.workspace = true
ui-events-winit.workspace = true
winit.workspace = true
sway-graph.workspace = true
bevy_ecs.workspace = true
bevy_math.workspace = true
bevy_reflect.workspace = true
bevy_transform.workspace = true

[dev-dependencies]
masonry_testing.workspace = true
bevy_app.workspace = true
bevy_time.workspace = true
```

- [ ] **Step 2: Rewrite the stale crate-doc invariant**

In `crates/sway-editor/src/lib.rs`, replace the first paragraph of the module doc:

```rust
//! The masonry half of the editor: a widget tree and the events that reach it.
//!
//! Depends on `bevy_ecs`, `bevy_reflect`, `bevy_transform` and `sway-graph`,
//! because the editor reads the live world directly (main design §2.8, §3:
//! "The editor links `sway-graph` regardless"). It deliberately depends on
//! none of `bevy` (the full facade), `bevy_render`, `wgpu`, `vello`, or
//! `imaging_vello` -- nothing here creates a device or touches a pipeline,
//! which is the M1b invariant that actually matters. `winit` appears only
//! because `ui-events-winit` takes `&winit::event::WindowEvent`; nothing here
//! draws with it.
```

Then add the new modules to the module list:

```rust
pub mod canvas;
pub mod external;
pub mod node_box;
pub mod scene_tree;
pub mod snapshot;

#[cfg(test)]
mod test_graph;
```

`scene_tree` does not exist until Task 6 — create `crates/sway-editor/src/scene_tree.rs` now as an empty file with a single doc line so the crate compiles:

```rust
//! `SceneTree` -- the world hierarchy pane. Implemented in Task 6.
```

- [ ] **Step 3: Write the test fixture**

Create `crates/sway-editor/src/test_graph.rs`:

```rust
//! Node types and world fixtures for `snapshot`'s tests.
//!
//! Deliberately local rather than reusing `sway-nodes`: those pull the `bevy`
//! facade and `bevy_render` through `sway-geo`, which this crate must not
//! link (see the crate doc). Two node types and a headless `App` is all the
//! read path needs to be tested against.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use sway_graph::{
    EdgeFrom, EdgeTo, EditorPos, GraphNode, GraphPlugin, NoOutputs, NoSlots, NodeId, NodeType,
    NodeTypeId, NodeTypeRegistry, ParamEdge, PortArena, PortKind, PortView, TickCtx, compile,
    register_node_type,
};

/// Graph tick rate for the fixture app. Matches `sway-graph`'s own test
/// harness; nothing here depends on the value.
const TICK_HZ: f64 = 120.0;

// --- Emit: no inputs, one continuous f32 output. ------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct EmitParams;

#[derive(Reflect, Default)]
pub(crate) struct EmitOut {
    pub value: f32,
}

#[derive(Component, Default)]
pub(crate) struct EmitState;

pub(crate) struct Emit;

impl Emit {
    pub const OUT_VALUE: u16 = 0;
}

impl NodeType for Emit {
    type Params = EmitParams;
    type Outputs = EmitOut;
    type Slots = NoSlots;
    type Produces = ();
    type State = EmitState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("value", Emit::OUT_VALUE)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        ports.write(sway_graph::ContinuousIdx(Emit::OUT_VALUE as u32), 0.75_f32);
    }
}

// --- Recv: one continuous f32 input, no outputs. ------------------------

#[derive(Reflect, Component, Default)]
pub(crate) struct RecvParams {
    pub amount: f32,
}

#[derive(Component, Default)]
pub(crate) struct RecvState;

pub(crate) struct Recv;

impl Recv {
    pub const AMOUNT: u16 = 0;
}

impl NodeType for Recv {
    type Params = RecvParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = RecvState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("amount", Recv::AMOUNT)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _ctx: &TickCtx) {}
}

// --- Fixtures -----------------------------------------------------------

/// Headless `App` with `Emit` and `Recv` registered, warmed up one frame.
///
/// The warm-up matters: Bevy's very first `Time::<Real>` update always
/// reports a zero delta, so without it the fixed-timestep accumulator can
/// never reach its threshold on frame 0 and `graph_tick` never runs. Same
/// recipe as `sway-graph`'s own `headless_app`.
pub(crate) fn app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins(GraphPlugin);
    register_node_type::<Emit>(&mut app);
    register_node_type::<Recv>(&mut app);
    app.update();
    app
}

fn type_id<N: NodeType>(world: &World) -> NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

pub(crate) fn spawn_emit(world: &mut World, id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Emit>(world) },
        EmitParams,
        EmitState::default(),
    ));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn spawn_recv(world: &mut World, id: u32, pos: Option<Vec2>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Recv>(world) },
        RecvParams::default(),
        RecvState::default(),
    ));
    if let Some(pos) = pos {
        entity.insert(EditorPos(pos));
    }
    entity.id()
}

pub(crate) fn connect(world: &mut World, from: Entity, sp: u16, to: Entity, tp: u16) {
    world.spawn((
        ParamEdge { source_port: sp, target_port: tp, kind: PortKind::Continuous },
        EdgeFrom(from),
        EdgeTo(to),
    ));
}

/// Compiles the world's graph and resizes the arena to match. Call after
/// every structural change, exactly as `sway-app` does.
pub(crate) fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("the fixture graph must compile");
    app.world_mut()
        .resource_mut::<PortArena>()
        .resize(compiled.continuous_len, compiled.events_len);
    app.world_mut().insert_resource(compiled);
}
```

- [ ] **Step 4: Write the failing tests**

Create `crates/sway-editor/src/snapshot.rs` with only the test module for now, so it fails to compile against a missing `capture`:

```rust
//! The graph -> UI read path. One pure function of `&World` per frame.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_graph::{Emit, Recv, app, connect, recompile, spawn_emit, spawn_recv};
    use bevy_math::Vec2;
    use kurbo::Point;

    #[test]
    fn short_type_name_strips_module_paths() {
        assert_eq!(short_type_name("sway_nodes::lfo::LFO"), "LFO");
        assert_eq!(
            short_type_name("sway_nodes::material::MaterialNode<bevy_pbr::StandardMaterial>"),
            "MaterialNode<StandardMaterial>"
        );
        assert_eq!(short_type_name("f32"), "f32");
    }

    #[test]
    fn nodes_carry_their_id_short_name_and_authored_position() {
        let mut app = app();
        spawn_emit(app.world_mut(), 7, Some(Vec2::new(20.0, 140.0)));
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.nodes.len(), 1);
        assert_eq!(snap.nodes[0].id.0, 7);
        assert_eq!(snap.nodes[0].name, "Emit");
        assert_eq!(snap.nodes[0].pos, Some(Point::new(20.0, 140.0)));
    }

    #[test]
    fn a_node_without_editor_pos_has_no_position() {
        let mut app = app();
        spawn_emit(app.world_mut(), 0, None);
        recompile(&mut app);

        assert_eq!(capture(app.world()).nodes[0].pos, None);
    }

    #[test]
    fn a_param_edge_indexes_into_the_node_list() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.edges.len(), 1);
        let from = &snap.nodes[snap.edges[0].from];
        let to = &snap.nodes[snap.edges[0].to];
        assert_eq!(from.entity, emit);
        assert_eq!(to.entity, recv);
        assert_eq!(snap.edges[0].kind, EdgeKind::Param);
    }

    #[test]
    fn a_continuous_f32_edge_reports_the_source_ports_live_value() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        // One tick, so `Emit::tick` has actually written its output port.
        app.update();

        assert_eq!(capture(app.world()).edges[0].activity, Some(0.75));
    }

    #[test]
    fn capture_before_compilation_yields_nodes_but_no_activity() {
        // A graph that has not been compiled has no `CompiledGraph` and an
        // empty arena. The editor must still draw it rather than panic.
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);

        let snap = capture(app.world());

        assert_eq!(snap.nodes.len(), 2);
        assert_eq!(snap.edges.len(), 1);
        assert_eq!(snap.edges[0].activity, None);
    }

    #[test]
    fn nodes_follow_compiled_topological_order() {
        // `recv` is spawned first but depends on `emit`, so the compiled order
        // puts `emit` first. The snapshot must follow that order, because the
        // fallback grid position is indexed by it (design §5).
        let mut app = app();
        let recv = spawn_recv(app.world_mut(), 1, None);
        let emit = spawn_emit(app.world_mut(), 0, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());

        assert_eq!(snap.nodes[0].entity, emit);
        assert_eq!(snap.nodes[1].entity, recv);
    }
}
```

- [ ] **Step 5: Run them to make sure they fail**

Run: `cargo test -p sway-editor snapshot::`
Expected: FAIL to compile — `cannot find function capture in this scope`, `cannot find type EdgeKind`.

- [ ] **Step 6: Implement the snapshot types and `capture`**

Prepend to `crates/sway-editor/src/snapshot.rs`, above the test module:

```rust
//! The graph -> UI read path. One pure function of `&World` per frame.
//!
//! Nothing is pushed here. Main design §2.11: "The editor likewise reads
//! rather than receives: live port values come from the arena and live node
//! values from components, with nothing pushed to it."
//!
//! Everything in this module is masonry-free by design -- `capture` is
//! testable against a headless `App` with no widget tree at all, which is
//! where the bulk of this feature's tests live.

use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use kurbo::Point;
use sway_graph::{
    CompiledGraph, EdgeFrom, EdgeTo, EditorPos, FeedsEdge, GraphNode, NodeId, NodePlan,
    NodeTypeRegistry, ParamEdge, PortArena, PortKind,
};

/// Which kind of edge this is. `ParentEdge` is deliberately absent: the tree
/// pane shows parenting already, and drawing it twice makes the canvas harder
/// to read for no gain (design §9).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Param,
    Feeds,
}

/// One graph node, as the canvas needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub entity: Entity,
    pub id: NodeId,
    /// The registered type name, shortened by [`short_type_name`].
    pub name: String,
    /// The authored [`EditorPos`], if any. The canvas treats this as a seed:
    /// read when a node box first appears and never again (design §5).
    pub pos: Option<Point>,
}

/// One edge, with both endpoints resolved to indices into
/// [`WorldSnapshot::nodes`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeView {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
    /// The source port's current value, when it is a continuous port holding
    /// an `f32`.
    ///
    /// `None` for every event edge, every `Feeds` edge, and every continuous
    /// edge carrying something other than an `f32` (a colour, a vector).
    /// Event edges are `None` **by design**, not by omission: an event
    /// occupies exactly one tick, so a frame-rate sampler observes roughly
    /// half of them and a MIDI note would pulse at random -- a worse signal
    /// than no signal. The alternative, an accumulator written by
    /// `graph_tick`, would put an editor-only write path in the hot tick,
    /// against §2.11. Design §4; revisit at M7.
    pub activity: Option<f32>,
}

/// Everything one frame of the editor reads out of the world.
#[derive(Clone, Debug, Default)]
pub struct WorldSnapshot {
    pub tree: Vec<TreeRow>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
}

/// Placeholder until Task 3. Rows of the world hierarchy pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub entity: Entity,
    pub depth: usize,
    pub label: String,
    pub node_id: Option<NodeId>,
}

/// Shortens a Rust type path to its last segment, preserving generics.
///
/// `sway_nodes::lfo::LFO` -> `LFO`, and
/// `sway_nodes::material::MaterialNode<bevy_pbr::StandardMaterial>` ->
/// `MaterialNode<StandardMaterial>`.
///
/// Temporary. `NodeTypeEntry::name` is `core::any::type_name::<N>()` today;
/// M4 introduces short registered names in the project format for exactly
/// this reason, and this function is deleted when it does.
pub fn short_type_name(path: &str) -> String {
    fn last_segment(s: &str) -> &str {
        match s.rfind("::") {
            Some(i) => &s[i + 2..],
            None => s,
        }
    }

    let mut out = String::with_capacity(path.len());
    let mut segment_start = 0;
    for (i, ch) in path.char_indices() {
        if matches!(ch, '<' | '>' | ',' | ' ') {
            out.push_str(last_segment(&path[segment_start..i]));
            out.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
    out.push_str(last_segment(&path[segment_start..]));
    out
}

/// Reads one frame's worth of graph state out of the world.
///
/// Pure: takes `&World`, touches nothing. Safe to call at any point,
/// including before the graph has ever been compiled -- a world with no
/// `CompiledGraph` yields nodes and edges with no activity rather than a
/// panic, which is the state the editor is in on the very first frame.
pub fn capture(world: &World) -> WorldSnapshot {
    let nodes = capture_nodes(world);
    let index: HashMap<Entity, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.entity, i))
        .collect();
    let edges = capture_edges(world, &index);
    WorldSnapshot { tree: Vec::new(), nodes, edges }
}

/// Node order: the compiled topological order when a `CompiledGraph` exists,
/// with any node missing from it appended in `NodeId` order; plain `NodeId`
/// order otherwise. Deterministic either way, which matters because the
/// canvas's fallback grid position is indexed by this order (design §5).
fn capture_nodes(world: &World) -> Vec<NodeView> {
    let registry = world.get_resource::<NodeTypeRegistry>();

    let mut ordered: Vec<Entity> = Vec::new();
    if let Some(compiled) = world.get_resource::<CompiledGraph>() {
        ordered.extend(compiled.plans.iter().map(|plan| plan.entity));
    }
    let seen: Vec<Entity> = ordered.clone();

    let mut leftovers: Vec<(NodeId, Entity)> = world
        .iter_entities()
        .filter_map(|entity_ref| {
            let node = entity_ref.get::<GraphNode>()?;
            (!seen.contains(&entity_ref.id())).then_some((node.id, entity_ref.id()))
        })
        .collect();
    leftovers.sort_unstable();
    ordered.extend(leftovers.into_iter().map(|(_, entity)| entity));

    ordered
        .into_iter()
        .filter_map(|entity| {
            let node = world.get::<GraphNode>(entity)?;
            let name = registry
                .and_then(|reg| reg.get(node.node_type))
                .map(|entry| short_type_name(entry.name))
                .unwrap_or_else(|| format!("<type {}>", node.node_type.0));
            Some(NodeView {
                entity,
                id: node.id,
                name,
                pos: world
                    .get::<EditorPos>(entity)
                    .map(|p| Point::new(p.0.x as f64, p.0.y as f64)),
            })
        })
        .collect()
}

fn capture_edges(world: &World, index: &HashMap<Entity, usize>) -> Vec<EdgeView> {
    let plans: HashMap<Entity, &NodePlan> = world
        .get_resource::<CompiledGraph>()
        .map(|compiled| {
            compiled
                .plans
                .iter()
                .map(|plan| (plan.entity, plan))
                .collect()
        })
        .unwrap_or_default();
    let arena = world.get_resource::<PortArena>();

    let mut edges = Vec::new();
    for entity_ref in world.iter_entities() {
        let (Some(EdgeFrom(source)), Some(EdgeTo(target))) =
            (entity_ref.get::<EdgeFrom>(), entity_ref.get::<EdgeTo>())
        else {
            continue;
        };
        let (Some(&from), Some(&to)) = (index.get(source), index.get(target)) else {
            continue;
        };

        if let Some(param) = entity_ref.get::<ParamEdge>() {
            let activity = match param.kind {
                PortKind::Continuous => continuous_value(&plans, arena, *source, param.source_port),
                PortKind::Event => None,
            };
            edges.push(EdgeView { from, to, kind: EdgeKind::Param, activity });
        } else if entity_ref.get::<FeedsEdge>().is_some() {
            edges.push(EdgeView { from, to, kind: EdgeKind::Feeds, activity: None });
        }
        // `ParentEdge` is intentionally skipped -- see `EdgeKind`.
    }
    edges
}

/// The source node's output port slot, downcast to `f32`.
///
/// The arena slot for a port ordinal is `continuous_base + ordinal`; the
/// compiler uses exactly this arithmetic when it builds `continuous_copies`.
fn continuous_value(
    plans: &HashMap<Entity, &NodePlan>,
    arena: Option<&PortArena>,
    source: Entity,
    source_port: u16,
) -> Option<f32> {
    let slot = plans.get(&source)?.continuous_base + source_port as usize;
    arena?
        .continuous
        .get(slot)?
        .try_downcast_ref::<f32>()
        .copied()
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-editor snapshot::`
Expected: PASS — 7 tests.

If `nodes_carry_their_id_short_name_and_authored_position` fails on the name, print `entry.name` to check whether `core::any::type_name` renders the fixture type as `sway_editor::test_graph::Emit`; `short_type_name` should reduce that to `Emit`.

- [ ] **Step 8: Verify the dependency constraint still holds**

Run: `cargo tree -p sway-editor -e normal | grep -E "bevy_render|wgpu|vello|^\s*\|?-* bevy v"`
Expected: no output. If `bevy_render` or `wgpu` appears, a dependency was added that violates the Global Constraints — stop and remove it.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-editor/Cargo.toml crates/sway-editor/src/lib.rs \
        crates/sway-editor/src/snapshot.rs crates/sway-editor/src/test_graph.rs \
        crates/sway-editor/src/scene_tree.rs
git commit -m "$(cat <<'EOF'
feat(editor): snapshot the graph's nodes and edges out of the world

capture(&World) is the whole graph->UI read path: pure, masonry-free, and
testable against a headless App. Activity is continuous-f32 only; event
edges are None by design, per design §4.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `snapshot` — the world tree

Every entity in the world, grouped and nested.

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`
- Modify: `crates/sway-editor/src/test_graph.rs` (add a `Transform`-carrying spawner)

**Interfaces:**
- Consumes: Task 2's `WorldSnapshot`, `short_type_name`.
- Produces:
  - `TreeGroup::{Scene, Graph, Edges, Other}` — `Copy`, `PartialEq`, and ordered `Scene < Graph < Edges < Other`.
  - `TreeRow { entity: Entity, group: TreeGroup, depth: usize, label: String, node_id: Option<NodeId> }` — Task 2's placeholder gains `group`.
  - `capture` now fills `WorldSnapshot::tree`. Rows are emitted in group order, so the widget can insert a header whenever `group` changes.

- [ ] **Step 1: Add a spatial spawner to the fixture**

In `crates/sway-editor/src/test_graph.rs`, add to the imports:

```rust
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::name::Name;
use bevy_transform::components::Transform;
```

and append:

```rust
/// A graph node that is also a scene entity: carries a `Transform`, and so
/// lands in the tree's `Scene` group (design §8).
pub(crate) fn spawn_spatial(world: &mut World, id: u32, parent: Option<Entity>) -> Entity {
    let mut entity = world.spawn((
        GraphNode { id: NodeId(id), node_type: type_id::<Emit>(world) },
        EmitParams,
        EmitState::default(),
        Transform::default(),
    ));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    entity.id()
}

/// A plain, non-graph entity carrying a `Name` -- stands in for the camera
/// and light `sway-app`'s `setup_scene` spawns outside the graph.
pub(crate) fn spawn_named_spatial(world: &mut World, name: &str) -> Entity {
    world.spawn((Name::new(name.to_string()), Transform::default())).id()
}
```

- [ ] **Step 2: Write the failing tests**

Append to `snapshot.rs`'s `mod tests`, and extend its `use crate::test_graph::{...}` list with `spawn_named_spatial, spawn_spatial`:

```rust
    fn rows_of(snap: &WorldSnapshot, group: TreeGroup) -> Vec<&TreeRow> {
        snap.tree.iter().filter(|row| row.group == group).collect()
    }

    #[test]
    fn rows_are_emitted_in_group_order() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let groups: Vec<TreeGroup> = capture(app.world())
            .tree
            .iter()
            .map(|row| row.group)
            .collect();

        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "rows must be emitted grouped, never interleaved");
    }

    #[test]
    fn a_spatial_node_is_in_scene_and_a_signal_node_is_in_graph() {
        let mut app = app();
        let spatial = spawn_spatial(app.world_mut(), 0, None);
        let signal = spawn_emit(app.world_mut(), 1, None);
        recompile(&mut app);

        let snap = capture(app.world());

        assert!(rows_of(&snap, TreeGroup::Scene).iter().any(|r| r.entity == spatial));
        assert!(rows_of(&snap, TreeGroup::Graph).iter().any(|r| r.entity == signal));
    }

    #[test]
    fn scene_rows_nest_by_child_of() {
        let mut app = app();
        let parent = spawn_spatial(app.world_mut(), 0, None);
        let child = spawn_spatial(app.world_mut(), 1, Some(parent));
        recompile(&mut app);

        let snap = capture(app.world());
        let scene = rows_of(&snap, TreeGroup::Scene);
        let parent_idx = scene.iter().position(|r| r.entity == parent).unwrap();
        let child_idx = scene.iter().position(|r| r.entity == child).unwrap();

        assert!(parent_idx < child_idx, "a parent must precede its child");
        assert_eq!(scene[parent_idx].depth, 0);
        assert_eq!(scene[child_idx].depth, 1);
    }

    #[test]
    fn a_name_component_wins_over_the_node_type() {
        let mut app = app();
        let named = spawn_named_spatial(app.world_mut(), "key light");

        let snap = capture(app.world());
        let row = snap.tree.iter().find(|r| r.entity == named).unwrap();

        assert_eq!(row.label, "key light");
        assert_eq!(row.node_id, None);
    }

    #[test]
    fn a_graph_node_row_is_labelled_by_type_and_node_id() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 7, None);
        recompile(&mut app);

        let snap = capture(app.world());
        let row = snap.tree.iter().find(|r| r.entity == emit).unwrap();

        assert_eq!(row.label, "Emit #7");
        assert_eq!(row.node_id.map(|id| id.0), Some(7));
    }

    #[test]
    fn edge_entities_are_grouped_under_edges() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 0, None);
        let recv = spawn_recv(app.world_mut(), 1, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let snap = capture(app.world());
        assert_eq!(rows_of(&snap, TreeGroup::Edges).len(), 1);
    }

    #[test]
    fn every_entity_in_the_world_gets_exactly_one_row() {
        let mut app = app();
        spawn_spatial(app.world_mut(), 0, None);
        spawn_emit(app.world_mut(), 1, None);
        recompile(&mut app);

        let snap = capture(app.world());
        let entity_count = app.world().iter_entities().count();

        assert_eq!(snap.tree.len(), entity_count);
        let mut entities: Vec<_> = snap.tree.iter().map(|r| r.entity).collect();
        entities.sort();
        let before = entities.len();
        entities.dedup();
        assert_eq!(entities.len(), before, "no entity may appear twice");
    }
```

- [ ] **Step 3: Run them to make sure they fail**

Run: `cargo test -p sway-editor snapshot::`
Expected: FAIL to compile — `cannot find type TreeGroup`, `no field group on TreeRow`.

- [ ] **Step 4: Implement the tree**

In `snapshot.rs`, extend the imports:

```rust
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::name::Name;
use bevy_transform::components::Transform;
use sway_graph::ParentEdge;
```

Replace the placeholder `TreeRow` with:

```rust
/// Which section of the tree pane a row belongs to.
///
/// Grouping is what makes "all entities" readable: a flat forest of several
/// hundred roots is not. `Ord` is derived and load-bearing -- `capture`
/// emits rows in this order so the widget can insert a section header
/// whenever the group changes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum TreeGroup {
    /// Has a `Transform`; nested by `ChildOf`.
    Scene,
    /// A `GraphNode` without a `Transform` -- geometry operators, signal nodes.
    Graph,
    /// `ParamEdge` / `FeedsEdge` / `ParentEdge` entities.
    Edges,
    /// Everything else, including Bevy's own internals.
    Other,
}

/// One row of the world hierarchy pane.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    pub entity: Entity,
    pub group: TreeGroup,
    /// Indentation level. Always 0 outside [`TreeGroup::Scene`], which is the
    /// only group that nests.
    pub depth: usize,
    pub label: String,
    /// `Some` when this entity is a graph node, which is what lets a tree
    /// selection highlight a node box in the canvas.
    pub node_id: Option<NodeId>,
}
```

Change `capture` to fill the tree:

```rust
pub fn capture(world: &World) -> WorldSnapshot {
    let nodes = capture_nodes(world);
    let index: HashMap<Entity, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.entity, i))
        .collect();
    let edges = capture_edges(world, &index);
    let tree = capture_tree(world);
    WorldSnapshot { tree, nodes, edges }
}
```

Append:

```rust
fn group_of(world: &World, entity: Entity) -> TreeGroup {
    if world.get::<Transform>(entity).is_some() {
        TreeGroup::Scene
    } else if world.get::<GraphNode>(entity).is_some() {
        TreeGroup::Graph
    } else if world.get::<ParamEdge>(entity).is_some()
        || world.get::<FeedsEdge>(entity).is_some()
        || world.get::<ParentEdge>(entity).is_some()
    {
        TreeGroup::Edges
    } else {
        TreeGroup::Other
    }
}

/// Best-effort row label: a `Name` wins; then a `GraphNode`'s shortened type
/// name and `NodeId`; then the entity index and its first three component
/// names, shortened the same way.
fn row_label(world: &World, entity: Entity) -> String {
    if let Some(name) = world.get::<Name>(entity) {
        return name.to_string();
    }
    if let Some(node) = world.get::<GraphNode>(entity) {
        let type_name = world
            .get_resource::<NodeTypeRegistry>()
            .and_then(|reg| reg.get(node.node_type))
            .map(|entry| short_type_name(entry.name))
            .unwrap_or_else(|| format!("<type {}>", node.node_type.0));
        return format!("{type_name} #{}", node.id.0);
    }
    let components: Vec<String> = world
        .inspect_entity(entity)
        .map(|infos| {
            infos
                .take(3)
                .map(|info| short_type_name(&info.name().shortname().to_string()))
                .collect()
        })
        .unwrap_or_default();
    if components.is_empty() {
        format!("e{}", entity.index())
    } else {
        format!("e{} [{}]", entity.index(), components.join(", "))
    }
}

fn capture_tree(world: &World) -> Vec<TreeRow> {
    let mut rows: Vec<TreeRow> = Vec::new();

    // Scene: roots first, then their `Children` depth-first. A `Transform`
    // entity whose parent has no `Transform` is a root here too -- it has
    // nowhere else to nest.
    let mut scene_roots: Vec<Entity> = world
        .iter_entities()
        .filter(|entity_ref| entity_ref.contains::<Transform>())
        .filter(|entity_ref| match entity_ref.get::<ChildOf>() {
            Some(ChildOf(parent)) => world.get::<Transform>(*parent).is_none(),
            None => true,
        })
        .map(|entity_ref| entity_ref.id())
        .collect();
    scene_roots.sort_unstable();
    for root in scene_roots {
        push_scene_subtree(world, root, 0, &mut rows);
    }

    // The flat groups, each sorted by entity for a stable order across frames.
    for group in [TreeGroup::Graph, TreeGroup::Edges, TreeGroup::Other] {
        let mut members: Vec<Entity> = world
            .iter_entities()
            .map(|entity_ref| entity_ref.id())
            .filter(|&entity| group_of(world, entity) == group)
            .collect();
        members.sort_unstable();
        rows.extend(members.into_iter().map(|entity| TreeRow {
            entity,
            group,
            depth: 0,
            label: row_label(world, entity),
            node_id: world.get::<GraphNode>(entity).map(|node| node.id),
        }));
    }

    rows
}

fn push_scene_subtree(world: &World, entity: Entity, depth: usize, rows: &mut Vec<TreeRow>) {
    rows.push(TreeRow {
        entity,
        group: TreeGroup::Scene,
        depth,
        label: row_label(world, entity),
        node_id: world.get::<GraphNode>(entity).map(|node| node.id),
    });
    if let Some(children) = world.get::<Children>(entity) {
        let mut spatial: Vec<Entity> = children
            .iter()
            .filter(|&child| world.get::<Transform>(child).is_some())
            .collect();
        spatial.sort_unstable();
        for child in spatial {
            push_scene_subtree(world, child, depth + 1, rows);
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-editor snapshot::`
Expected: PASS — 14 tests.

Two things to check if `every_entity_in_the_world_gets_exactly_one_row` fails: `Children::iter()` in bevy 0.19 may yield `Entity` by value or by reference (adjust the closure), and a `Transform` entity parented under a non-`Transform` entity must be reached exactly once — as a root, not also as a child.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-editor/src/snapshot.rs crates/sway-editor/src/test_graph.rs
git commit -m "$(cat <<'EOF'
feat(editor): snapshot every entity in the world, grouped

Scene (nested by ChildOf), Graph, Edges, Other. Grouping is what makes
"all entities" readable; rows are emitted in group order so the widget can
insert section headers on a group change.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `NodeBox` draws its label, and stops inventing edges

`NodeBox` has carried a `label` field since M1b and never drawn it — it reaches only the accessibility node. It also owns a drag-to-connect gesture that appends to a local `Vec`, inventing connections that exist in no graph.

**Files:**
- Modify: `crates/sway-editor/src/node_box.rs`
- Modify: `crates/sway-editor/src/canvas.rs` (remove the `ConnectStart`/`ConnectMove`/`ConnectEnd` arms and `pending_edge`)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `NodeBox::new(label: String) -> Self` — unchanged signature, now builds a `Label` child.
  - `NodeBox::set_label(this: &mut WidgetMut<'_, Self>, label: &str)` — used by Task 5 when a snapshot renames a node.
  - `NodeBoxAction::{Selected, DraggedBy(Vec2)}` — the connect variants are gone.

- [ ] **Step 1: Write the failing test**

Append to `node_box.rs` a test module:

```rust
#[cfg(test)]
mod tests {
    use super::{NodeBox, SIZE};
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::Point;
    use masonry_testing::TestHarness;

    #[test]
    fn a_node_box_has_a_label_child_carrying_its_text() {
        let node = NodeBox::new("LFO #3".to_string());
        let harness = TestHarness::create(DefaultProperties::default(), node.prepare());
        assert_eq!(harness.root_widget().label_text(), "LFO #3");
        assert_eq!(harness.root_widget().children_ids().len(), 1);
    }

    #[test]
    fn a_press_in_the_right_hand_quarter_selects_rather_than_connects() {
        // Drag-to-connect is gone: the whole box is now one gesture, so a
        // press anywhere -- including where the connector dot used to be --
        // selects and drags.
        let node = NodeBox::new("n".to_string());
        let mut harness = TestHarness::create(DefaultProperties::default(), node.prepare());

        harness.mouse_move(Point::new(SIZE.width - 8.0, SIZE.height / 2.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        let action = harness.take_action();
        assert!(action.is_some(), "a press must still submit an action");
    }
}
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p sway-editor node_box::`
Expected: FAIL — `no method named label_text`.

- [ ] **Step 3: Give `NodeBox` a `Label` child**

In `crates/sway-editor/src/node_box.rs`, extend the imports:

```rust
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, RoundedRect, Size, Stroke};
```

`Circle` is no longer used — remove it from the `kurbo` import.

Replace the struct and its constructor:

```rust
/// Inset of the label from the box's top-left corner, in logical pixels.
const LABEL_INSET: f64 = 10.0;

/// A node box in the graph canvas: a rounded rectangle with a border and a
/// text label, drawn through `imaging::Painter` and one `Label` child.
///
/// `Label` rather than painting text directly: `imaging::Painter` exposes
/// only `glyphs`, which takes *pre-shaped* glyphs, and shaping is masonry's
/// job. `Label::accepts_pointer_interaction` is `false`, so the child never
/// steals a press from this widget's own gesture handling.
pub struct NodeBox {
    label: WidgetPod<Label>,
    label_text: String,
    selected: bool,
    gesture: Gesture,
}

impl NodeBox {
    /// Creates a new, unselected node box with the given label.
    pub fn new(label: String) -> Self {
        Self {
            label: Label::new(label.clone()).prepare().to_pod(),
            label_text: label,
            selected: false,
            gesture: Gesture::None,
        }
    }

    /// The text this box currently displays.
    pub fn label_text(&self) -> &str {
        &self.label_text
    }
}
```

Replace the `Gesture` enum's `Connecting` variant — it now has only two states:

```rust
/// What the pointer is currently doing to this node box, between a `Down`
/// that started a gesture and the `Up`/`Cancel` that ends it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Gesture {
    /// No button is down, or the last gesture already ended.
    None,
    /// Dragging the node body. Carries the last-seen window-space (logical)
    /// pointer position, so `Move` can report an *incremental* delta rather
    /// than a delta-since-`Down` (see the module doc for why this can't be
    /// derived from `ctx.local_position` instead).
    Dragging { last_window: Point },
}
```

Replace `NodeBoxAction`:

```rust
/// The action a [`NodeBox`] reports to its parent [`GraphCanvas`] through
/// [`EventCtx::submit_action`]/[`Widget::on_action`]. Deltas are window-space
/// (logical pixels); see the module doc.
///
/// Drag-to-connect is deliberately absent. It appended to a local `Vec` of
/// edges, inventing connections that exist in no graph -- harmless against
/// placeholder boxes, a lie against real ones. Topology editing arrives at M7.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeBoxAction {
    /// This node was pressed: the canvas should select it.
    Selected,
    /// The pointer moved by this delta while dragging the node.
    DraggedBy(masonry_core::kurbo::Vec2),
}
```

Add `set_label` next to `set_selected`:

```rust
    /// Replaces the displayed text. Called by `GraphCanvas` when a snapshot
    /// renames a node -- which happens on a node-type change under a
    /// surviving `NodeId`.
    pub fn set_label(this: &mut WidgetMut<'_, Self>, label: &str) {
        if this.widget.label_text == label {
            return;
        }
        label.clone_into(&mut this.widget.label_text);
        let mut child = this.ctx.get_mut(&mut this.widget.label);
        Label::set_text(&mut child, label.to_string());
    }
```

- [ ] **Step 4: Update the `Widget` impl**

```rust
    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.label);
    }
```

```rust
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let inner = Size::new(
            (size.width - 2.0 * LABEL_INSET).max(0.0),
            (size.height - 2.0 * LABEL_INSET).max(0.0),
        );
        ctx.run_layout(&mut self.label, inner);
        ctx.place_child(&mut self.label, Point::new(LABEL_INSET, LABEL_INSET));
        ctx.set_clip_path(size.to_rect());
    }
```

```rust
    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::from_slice(&[self.label.id()])
    }
```

In `on_pointer_event`, the `Down` arm loses its zone test:

```rust
            PointerEvent::Down(PointerButtonEvent { button, state, .. }) => {
                if *button != Some(PointerButton::Primary) {
                    // Anything other than the primary button -- in
                    // particular the middle button, which `GraphCanvas`
                    // uses to pan directly -- is not a node gesture. Leave it
                    // unhandled so it bubbles up to
                    // `GraphCanvas::on_pointer_event`.
                    return;
                }
                ctx.capture_pointer();
                self.gesture = Gesture::Dragging { last_window: window_point(state) };
                ctx.submit_action::<Self::Action>(NodeBoxAction::Selected);
                // Stop this from also bubbling to `GraphCanvas::on_pointer_event`,
                // which treats an unhandled `Down` as "background click, clear
                // selection" -- see that method's doc comment.
                ctx.set_handled();
            }
```

The `Move` arm loses its `Connecting` branch:

```rust
            PointerEvent::Move(PointerUpdate { current, .. }) if ctx.is_active() => {
                let window = window_point(current);
                if let Gesture::Dragging { last_window } = &mut self.gesture {
                    let delta = window - *last_window;
                    *last_window = window;
                    ctx.submit_action::<Self::Action>(NodeBoxAction::DraggedBy(delta));
                }
                ctx.set_handled();
            }
            PointerEvent::Up(..) => {
                self.gesture = Gesture::None;
                ctx.set_handled();
            }
```

In `paint`, delete the connector-dot block (the `Circle` fill) and leave the rounded rect and its stroke.

In `accessibility`, `self.label` is now a `WidgetPod` — use the cached text:

```rust
    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, node: &mut Node) {
        node.set_description(self.label_text.as_str());
    }
```

Delete the `CONNECT_ZONE_FRACTION` constant and the `# Child -> parent communication` doc paragraph's reference to drag-to-connect.

- [ ] **Step 5: Strip drag-to-connect out of `GraphCanvas`**

In `crates/sway-editor/src/canvas.rs`:

- Delete the `pending_edge` field, its initializer in `new`, and its doc comment.
- In `paint`, delete the `if let Some((src_idx, cursor_canvas)) = self.pending_edge` block.
- In `on_action`, delete the `ConnectStart`, `ConnectMove`, and `ConnectEnd` arms, leaving `Selected` and `DraggedBy`.
- Delete `node_at_canvas_point` and `window_to_canvas` — both existed only for drag-to-connect, and `cargo clippy -D warnings` will flag them as dead code otherwise.
- Remove now-unused imports (`Rect` if nothing else uses it).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-editor`
Expected: PASS. The M1b gate tests `press_under_zoom_reaches_the_scaled_node`, `press_outside_any_node_clears_selection`, `middle_drag_pans_the_canvas_by_the_raw_delta`, `middle_drag_over_a_node_pans_instead_of_dragging_it` and both scroll tests must still pass — they are the masonry bet's assertions and must not regress.

If `a_press_in_the_right_hand_quarter_selects_rather_than_connects` cannot find `harness.take_action()`, check `masonry_testing::TestHarness`'s action-inspection method name at the pinned rev and use the one that exists; the assertion is "a press submits an action", however that is spelled.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-editor/src/node_box.rs crates/sway-editor/src/canvas.rs
git commit -m "$(cat <<'EOF'
feat(editor): node boxes draw their label; drag-to-connect removed

The label has been carried since M1b and reached only the accessibility
node. Text goes through a Label child because imaging::Painter only takes
pre-shaped glyphs.

Drag-to-connect appended to a local Vec, inventing edges that exist in no
graph. Harmless against placeholders, a lie against real nodes. M7.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `GraphCanvas` is driven by the snapshot

The identity model is the point: nodes are keyed by `NodeId`, so a node keeps its `WidgetId` — and therefore its drag position and selection — across snapshots.

**Files:**
- Modify: `crates/sway-editor/src/canvas.rs`

**Interfaces:**
- Consumes: `snapshot::{WorldSnapshot, NodeView, EdgeView, EdgeKind}`, `sway_graph::NodeId`, `NodeBox::set_label`.
- Produces:
  - `GraphCanvas::new() -> Self` — unchanged, now creates an empty canvas with no builder methods.
  - `GraphCanvas::apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot)`
  - `GraphCanvas::selected_node(&self) -> Option<NodeId>` — **changed** from `Option<usize>`.
  - `GraphCanvas::set_selected(this: &mut WidgetMut<'_, Self>, selected: Option<NodeId>)` — **changed**.
  - `GraphCanvas::pan(&self) -> Vec2` — unchanged.
  - `GraphCanvas::set_zoom` / `set_pan` — unchanged.

- [ ] **Step 1: Write the failing tests**

Replace `canvas.rs`'s test module contents that use `with_node` (all of them do) with snapshot-driven equivalents, and add the identity tests. The full new test module:

```rust
#[cfg(test)]
mod tests {
    use super::GraphCanvas;
    use crate::snapshot::{EdgeKind, EdgeView, NodeView, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::{Point, Vec2};
    use masonry_testing::TestHarness;
    use sway_graph::NodeId;

    fn node(id: u32, name: &str, pos: Option<Point>) -> NodeView {
        NodeView {
            entity: Entity::from_raw_u32(id).expect("valid entity id"),
            id: NodeId(id),
            name: name.to_string(),
            pos,
        }
    }

    fn snapshot(nodes: Vec<NodeView>, edges: Vec<EdgeView>) -> WorldSnapshot {
        WorldSnapshot { tree: Vec::new(), nodes, edges }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<GraphCanvas> {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), GraphCanvas::new().prepare());
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });
        harness
    }

    /// The claim spec §2.8 makes for masonry, reduced to an assertion.
    ///
    /// A node sits at canvas-space (100, 100). The canvas is zoomed 2x, so it
    /// occupies window space around (200, 200). A press at (210, 210) must
    /// reach *that node's* widget -- not the canvas, not a neighbour. If
    /// masonry's `window_transform` inverse did not drive hit-testing, this
    /// press would land on whatever is at unscaled (210, 210) instead, and a
    /// node editor built on it would be subtly, unfixably wrong under zoom.
    #[test]
    fn press_under_zoom_reaches_the_scaled_node() {
        let mut harness = harness_with(snapshot(
            vec![
                node(0, "a", Some(Point::new(100.0, 100.0))),
                node(1, "b", Some(Point::new(400.0, 100.0))),
            ],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_zoom(&mut canvas, 2.0);
        });

        harness.mouse_move(Point::new(210.0, 210.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected_node(), Some(NodeId(0)));
    }

    #[test]
    fn press_outside_any_node_clears_selection() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(0)));
        });

        harness.mouse_move(Point::new(20.0, 20.0));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected_node(), None);
    }

    #[test]
    fn middle_drag_pans_the_canvas_by_the_raw_delta() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));

        harness.mouse_move(Point::new(50.0, 50.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(80.0, 65.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(30.0, 15.0));
    }

    /// A middle-drag that starts *over* a node must still pan the canvas,
    /// not drag the node -- `NodeBox` only claims the primary button.
    #[test]
    fn middle_drag_over_a_node_pans_instead_of_dragging_it() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Auxiliary));
        harness.mouse_move(Point::new(170.0, 150.0));
        harness.mouse_button_release(Some(PointerButton::Auxiliary));

        assert_eq!(harness.root_widget().pan(), Vec2::new(20.0, 20.0));
        assert_eq!(harness.root_widget().selected_node(), None);
    }

    #[test]
    fn scroll_pixel_delta_converts_physical_to_logical() {
        use masonry::core::{PointerEvent, PointerScrollEvent, PointerState, ScrollDelta};
        use masonry::dpi::PhysicalPosition;
        use masonry_testing::PRIMARY_MOUSE;

        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        let state = PointerState {
            scale_factor: 2.0,
            position: PhysicalPosition { x: 100.0, y: 100.0 },
            ..Default::default()
        };

        harness.process_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
            pointer: PRIMARY_MOUSE,
            delta: ScrollDelta::PixelDelta(PhysicalPosition { x: 40.0, y: 20.0 }),
            state,
        }));

        assert_eq!(harness.root_widget().pan(), Vec2::new(-20.0, -10.0));
    }

    #[test]
    fn scroll_line_delta_is_dpi_invariant() {
        use masonry::core::{PointerEvent, PointerScrollEvent, PointerState, ScrollDelta};
        use masonry::dpi::PhysicalPosition;
        use masonry_testing::PRIMARY_MOUSE;

        let mut harness = harness_with(snapshot(
            vec![node(0, "a", Some(Point::new(100.0, 100.0)))],
            vec![],
        ));
        let state = PointerState {
            scale_factor: 2.0,
            position: PhysicalPosition { x: 100.0, y: 100.0 },
            ..Default::default()
        };

        harness.process_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
            pointer: PRIMARY_MOUSE,
            delta: ScrollDelta::LineDelta(0.0, 1.0),
            state,
        }));

        assert_eq!(harness.root_widget().pan(), Vec2::new(0.0, -32.0));
    }

    #[test]
    fn a_node_surviving_a_snapshot_keeps_its_widget_id() {
        let first = snapshot(vec![node(0, "a", Some(Point::new(10.0, 10.0)))], vec![]);
        let mut harness = harness_with(first);
        let before = harness.root_widget().widget_id_of(NodeId(0)).unwrap();

        let second = snapshot(
            vec![
                node(0, "a", Some(Point::new(10.0, 10.0))),
                node(1, "b", Some(Point::new(300.0, 10.0))),
            ],
            vec![],
        );
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &second);
        });

        assert_eq!(harness.root_widget().widget_id_of(NodeId(0)), Some(before));
        assert!(harness.root_widget().widget_id_of(NodeId(1)).is_some());
    }

    #[test]
    fn a_node_dropped_from_a_snapshot_is_removed() {
        let mut harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snapshot(vec![node(1, "b", None)], vec![]));
        });

        assert_eq!(harness.root_widget().widget_id_of(NodeId(0)), None);
        assert!(harness.root_widget().widget_id_of(NodeId(1)).is_some());
    }

    /// Design §5: `EditorPos` is a *seed*, read when a node box first appears
    /// and never again. Without this, the next frame's snapshot would snap a
    /// dragged node straight back to its authored position.
    #[test]
    fn a_dragged_node_is_not_snapped_back_by_the_next_snapshot() {
        let snap = snapshot(vec![node(0, "a", Some(Point::new(100.0, 100.0)))], vec![]);
        let mut harness = harness_with(snap.clone());

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_move(Point::new(200.0, 180.0));
        harness.mouse_button_release(Some(PointerButton::Primary));
        let dragged = harness.root_widget().position_of(NodeId(0)).unwrap();
        assert_ne!(dragged, Point::new(100.0, 100.0), "the drag must have moved it");

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });

        assert_eq!(harness.root_widget().position_of(NodeId(0)), Some(dragged));
    }

    /// A node with no `EditorPos` lands on the fallback grid, and two such
    /// nodes never collide.
    #[test]
    fn unpositioned_nodes_land_on_distinct_fallback_slots() {
        let harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![],
        ));

        let a = harness.root_widget().position_of(NodeId(0)).unwrap();
        let b = harness.root_widget().position_of(NodeId(1)).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn edges_are_kept_only_when_both_endpoints_exist() {
        let harness = harness_with(snapshot(
            vec![node(0, "a", None), node(1, "b", None)],
            vec![EdgeView { from: 0, to: 1, kind: EdgeKind::Param, activity: Some(0.5) }],
        ));
        assert_eq!(harness.root_widget().edge_count(), 1);

        let mut harness = harness;
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(
                &mut canvas,
                &snapshot(
                    vec![node(0, "a", None)],
                    vec![EdgeView { from: 0, to: 9, kind: EdgeKind::Param, activity: None }],
                ),
            );
        });
        assert_eq!(harness.root_widget().edge_count(), 0);
    }
}
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p sway-editor canvas::`
Expected: FAIL to compile — `no function apply_snapshot`, `no method widget_id_of`, `position_of`, `edge_count`.

- [ ] **Step 3: Rework `GraphCanvas`'s state**

In `crates/sway-editor/src/canvas.rs`, replace the struct, its builders, and the viewport slot. Delete `ViewportSlot`, `with_node`, `with_edge`, `with_viewport`, and the `use crate::external::ViewportPlaceholder;` import; add:

```rust
use crate::snapshot::{EdgeKind, WorldSnapshot};
use masonry::core::WidgetId;
use sway_graph::NodeId;
use std::collections::HashMap;

/// Fallback grid for nodes with no authored `EditorPos` (design §5): column
/// `i / FALLBACK_ROWS`, row `i % FALLBACK_ROWS`, at the node box's own pitch.
/// Deterministic, so an unpositioned node is misplaced rather than invisible,
/// and two of them never land on top of each other.
const FALLBACK_ROWS: usize = 6;
const FALLBACK_PITCH: Size = Size::new(220.0, 120.0);

/// One node box and everything the canvas knows about it.
struct NodeSlot {
    pod: WidgetPod<NodeBox>,
    /// Canvas-space position. Seeded from the snapshot's `EditorPos` when the
    /// slot is created and owned by the widget thereafter -- see
    /// `apply_snapshot`.
    pos: Point,
    label: String,
}

/// One painted edge, resolved to node keys rather than snapshot indices so it
/// survives a reordering.
struct EdgeSlot {
    from: NodeId,
    to: NodeId,
    kind: EdgeKind,
    /// The source port's current value, or `None` when this edge carries no
    /// readable activity (design §4).
    value: Option<f32>,
    /// Running observed range for this edge, used to normalise `value` into
    /// 0..1. Auto-ranging avoids a per-node-type table of expected ranges,
    /// which would be one more thing to keep in sync with the node set.
    range: Option<(f32, f32)>,
}

/// The node-editor canvas: owns pan/zoom, lays out its `NodeBox` children at
/// explicit canvas-space positions, and paints the edges between them.
///
/// Driven entirely by [`WorldSnapshot`] through [`GraphCanvas::apply_snapshot`].
/// Nodes are keyed by [`NodeId`], not by insertion index, so a node that
/// survives a snapshot keeps its `WidgetId` -- and therefore its dragged
/// position and its selection. That identity model is what M7 needs.
pub struct GraphCanvas {
    nodes: Vec<NodeId>,
    slots: HashMap<NodeId, NodeSlot>,
    edges: Vec<EdgeSlot>,
    pan: Vec2,
    zoom: f64,
    selected: Option<NodeId>,
    panning: Option<Point>,
}

impl GraphCanvas {
    /// Creates an empty canvas. Content arrives through `apply_snapshot`.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            slots: HashMap::new(),
            edges: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            selected: None,
            panning: None,
        }
    }
}
```

`self.nodes` holds the draw/layout order (mirroring the snapshot's order); `self.slots` holds the per-node state. Everywhere the old code indexed `self.positions[idx]` or `self.nodes[idx]`, it now looks up `self.slots.get(&id)`.

- [ ] **Step 4: Implement `apply_snapshot` and the inspection accessors**

In the `WIDGETMUT` block:

```rust
    /// Reconciles the canvas against one frame's snapshot.
    ///
    /// Nodes present in both keep their `WidgetId`, their dragged position,
    /// and their selection; nodes only in the snapshot are created (seeded
    /// from `EditorPos`, or the fallback grid); nodes only in the canvas are
    /// removed. Edge geometry is rebuilt outright -- an edge has no identity
    /// worth preserving -- but each edge's observed value range is carried
    /// across by `(from, to, kind)` so auto-ranging does not restart.
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let mut ranges: HashMap<(NodeId, NodeId, EdgeKind), (f32, f32)> = HashMap::new();
        for edge in &this.widget.edges {
            if let Some(range) = edge.range {
                ranges.insert((edge.from, edge.to, edge.kind), range);
            }
        }

        // Nodes: create, update, remove.
        let wanted: Vec<NodeId> = snap.nodes.iter().map(|node| node.id).collect();
        let stale: Vec<NodeId> = this
            .widget
            .nodes
            .iter()
            .copied()
            .filter(|id| !wanted.contains(id))
            .collect();
        for id in stale {
            if let Some(mut slot) = this.widget.slots.remove(&id) {
                this.ctx.remove_child(&mut slot.pod);
            }
            if this.widget.selected == Some(id) {
                this.widget.selected = None;
            }
        }

        for (index, view) in snap.nodes.iter().enumerate() {
            match this.widget.slots.get_mut(&view.id) {
                Some(slot) => {
                    if slot.label != view.name {
                        view.name.clone_into(&mut slot.label);
                        let mut child = this.ctx.get_mut(&mut slot.pod);
                        NodeBox::set_label(&mut child, &view.name);
                    }
                }
                None => {
                    // `EditorPos` seeds a *new* box only; an existing box owns
                    // its position from here on (design §5).
                    let pos = view.pos.unwrap_or_else(|| fallback_pos(index));
                    let pod = NodeBox::new(view.name.clone()).prepare().to_pod();
                    this.widget.slots.insert(
                        view.id,
                        NodeSlot { pod, pos, label: view.name.clone() },
                    );
                    this.ctx.children_changed();
                }
            }
        }
        this.widget.nodes = wanted;

        // Edges: rebuilt outright, carrying observed ranges across.
        this.widget.edges = snap
            .edges
            .iter()
            .filter_map(|edge| {
                let from = snap.nodes.get(edge.from)?.id;
                let to = snap.nodes.get(edge.to)?.id;
                let mut range = ranges.get(&(from, to, edge.kind)).copied();
                if let Some(value) = edge.activity {
                    range = Some(match range {
                        Some((lo, hi)) => (lo.min(value), hi.max(value)),
                        None => (value, value),
                    });
                }
                Some(EdgeSlot { from, to, kind: edge.kind, value: edge.activity, range })
            })
            .collect();

        Self::retransform_via_mutate_ctx(this);
        this.ctx.request_render();
    }

    /// The `WidgetId` of a node's box, for tests and for M7's selection
    /// plumbing. `None` if the canvas has no such node.
    pub fn widget_id_of(&self, id: NodeId) -> Option<WidgetId> {
        self.slots.get(&id).map(|slot| slot.pod.id())
    }

    /// A node's current canvas-space position.
    pub fn position_of(&self, id: NodeId) -> Option<Point> {
        self.slots.get(&id).map(|slot| slot.pos)
    }

    /// How many edges are currently painted.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
```

and the free function:

```rust
/// Design §5's fallback grid slot for the node at snapshot index `index`.
fn fallback_pos(index: usize) -> Point {
    Point::new(
        (index / FALLBACK_ROWS) as f64 * FALLBACK_PITCH.width,
        (index % FALLBACK_ROWS) as f64 * FALLBACK_PITCH.height,
    )
}
```

- [ ] **Step 5: Rekey selection and the transform helpers**

`set_selected`, `selected_node`, `clear_selection`, `select_from_action`, `child_transform`, `retransform_all_from_event`, `retransform_one_from_action` and `retransform_via_mutate_ctx` all move from `usize` to `NodeId`. `child_transform` becomes:

```rust
    fn child_transform(&self, id: NodeId) -> Affine {
        let pos = self.slots.get(&id).map(|slot| slot.pos).unwrap_or_default();
        Affine::translate(self.pan) * Affine::scale(self.zoom) * Affine::translate(pos.to_vec2())
    }
```

`on_action` resolves the source `WidgetId` to a `NodeId`:

```rust
        let Some(id) = self
            .nodes
            .iter()
            .copied()
            .find(|id| self.slots.get(id).is_some_and(|slot| slot.pod.id() == source))
        else {
            return;
        };
        let Some(&action) = action.downcast_ref::<NodeBoxAction>() else {
            return;
        };
        match action {
            NodeBoxAction::Selected => self.select_from_action(ctx, id),
            NodeBoxAction::DraggedBy(delta) => {
                if let Some(slot) = self.slots.get_mut(&id) {
                    slot.pos += delta / self.zoom;
                }
                self.retransform_one_from_action(ctx, id);
                ctx.request_paint_only();
            }
        }
        ctx.set_handled();
```

`register_children`, `layout` and `children_ids` iterate `self.nodes` and look each up in `self.slots`, with the viewport gone entirely:

```rust
    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for id in &self.nodes {
            if let Some(slot) = self.slots.get_mut(id) {
                ctx.register_child(&mut slot.pod);
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for id in self.nodes.clone() {
            if let Some(slot) = self.slots.get_mut(&id) {
                ctx.run_layout(&mut slot.pod, node_box::SIZE);
                ctx.place_child(&mut slot.pod, Point::ZERO);
            }
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn children_ids(&self) -> ChildrenIds {
        self.nodes
            .iter()
            .filter_map(|id| self.slots.get(id).map(|slot| slot.pod.id()))
            .collect()
    }
```

- [ ] **Step 6: Paint edges by kind and activity**

Replace `paint`:

```rust
    /// Paints the edges. Runs *before* children paint, so edges sit behind
    /// the node boxes -- the correct z-order for a node editor, and why this
    /// uses `paint` rather than `post_paint`.
    ///
    /// `GraphCanvas` itself carries no transform (it's the root of its pane),
    /// so edges are painted in the same window frame the children's
    /// transforms map into: `to_visual` applies `pan`/`zoom` by hand to every
    /// point, mirroring what each child's `set_transform` does for itself.
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let half_height = Vec2::new(0.0, node_box::SIZE.height / 2.0);
        let right_edge = Vec2::new(node_box::SIZE.width, 0.0) + half_height;

        for edge in &self.edges {
            let (Some(from_slot), Some(to_slot)) =
                (self.slots.get(&edge.from), self.slots.get(&edge.to))
            else {
                continue;
            };
            let from = self.to_visual(from_slot.pos + right_edge);
            let to = self.to_visual(to_slot.pos + half_height);
            let (brush, width) = edge_style(edge);
            self.paint_edge(painter, from, to, brush, width);
        }
    }
```

and the helpers:

```rust
/// Base colour per edge kind, brightened and thickened by activity.
///
/// An edge with no readable activity (every event edge, every `Feeds` edge,
/// every continuous edge carrying something other than an `f32`) draws at the
/// base weight. Design §4 -- that is a decision, not an omission.
fn edge_style(edge: &EdgeSlot) -> (Color, f64) {
    let base = match edge.kind {
        EdgeKind::Param => Color::from_rgb8(140, 140, 155),
        EdgeKind::Feeds => Color::from_rgb8(120, 165, 140),
    };
    let Some(t) = normalised(edge) else {
        return (base, 2.0);
    };
    let lift = |c: u8| (c as f32 + (255.0 - c as f32) * t).round().clamp(0.0, 255.0) as u8;
    let [r, g, b, _] = base.to_rgba8().to_u8_array();
    (Color::from_rgb8(lift(r), lift(g), lift(b)), 2.0 + 3.0 * t as f64)
}

/// The edge's current value mapped into 0..1 through its observed range.
/// `None` when there is no value; `0.5` when the range is degenerate (every
/// sample so far identical), which is the only neutral answer available.
fn normalised(edge: &EdgeSlot) -> Option<f32> {
    let value = edge.value?;
    let (lo, hi) = edge.range?;
    if (hi - lo).abs() < f32::EPSILON {
        return Some(0.5);
    }
    Some(((value - lo) / (hi - lo)).clamp(0.0, 1.0))
}
```

and widen `paint_edge`:

```rust
    fn paint_edge(&self, painter: &mut Painter<'_>, from: Point, to: Point, brush: Color, width: f64) {
        let dx = ((to.x - from.x) * 0.5).abs().max(30.0);
        let mut path = BezPath::new();
        path.move_to(from);
        path.curve_to(Point::new(from.x + dx, from.y), Point::new(to.x - dx, to.y), to);
        painter.stroke(&path, &Stroke::new(width), brush).draw();
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-editor canvas::`
Expected: PASS — 11 tests, including all six carried from M1b.

If `Color::to_rgba8().to_u8_array()` does not exist at the pinned `peniko` version, use whatever accessor does — the requirement is "read the base colour's channels", not that exact spelling.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-editor/src/canvas.rs
git commit -m "$(cat <<'EOF'
feat(editor): drive the graph canvas from the world snapshot

Nodes are keyed by NodeId, so a node surviving a snapshot keeps its
WidgetId, its dragged position, and its selection -- the identity model M7
needs. EditorPos seeds a new box only; the widget owns its position after.

Edge weight and brightness come from the source port's live value,
auto-ranged per edge. Param and Feeds render in distinct colours; ParentEdge
is not drawn, since the tree pane shows parenting.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: The `SceneTree` widget

**Files:**
- Modify: `crates/sway-editor/src/scene_tree.rs` (currently a doc-comment stub)

**Interfaces:**
- Consumes: `snapshot::{WorldSnapshot, TreeRow, TreeGroup}`, `sway_graph::NodeId`.
- Produces:
  - `SceneTree::new() -> Self`
  - `SceneTree::apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot)`
  - `SceneTree::row_count(&self) -> usize` — headers included.
  - `SceneTree::selected(&self) -> Option<Entity>`
  - `SceneTree::set_selected(this: &mut WidgetMut<'_, Self>, entity: Option<Entity>)`
  - `SceneTreeAction::RowSelected { entity: Entity, node_id: Option<NodeId> }`
  - `pub const ROW_HEIGHT: f64`

- [ ] **Step 1: Write the failing tests**

Create the test module in `crates/sway-editor/src/scene_tree.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{ROW_HEIGHT, SceneTree};
    use crate::snapshot::{TreeGroup, TreeRow, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::Point;
    use masonry_testing::TestHarness;

    fn entity(i: u32) -> Entity {
        Entity::from_raw_u32(i).expect("valid entity id")
    }

    fn row(i: u32, group: TreeGroup, depth: usize, label: &str) -> TreeRow {
        TreeRow {
            entity: entity(i),
            group,
            depth,
            label: label.to_string(),
            node_id: None,
        }
    }

    fn tree(rows: Vec<TreeRow>) -> WorldSnapshot {
        WorldSnapshot { tree: rows, nodes: Vec::new(), edges: Vec::new() }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<SceneTree> {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), SceneTree::new().prepare());
        harness.edit_root_widget(|mut tree| {
            SceneTree::apply_snapshot(&mut tree, &snap);
        });
        harness
    }

    #[test]
    fn a_header_is_inserted_wherever_the_group_changes() {
        let harness = harness_with(tree(vec![
            row(0, TreeGroup::Scene, 0, "root"),
            row(1, TreeGroup::Scene, 1, "mesh"),
            row(2, TreeGroup::Graph, 0, "LFO #3"),
        ]));

        // Three entity rows plus two headers.
        assert_eq!(harness.root_widget().row_count(), 5);
    }

    #[test]
    fn rows_track_the_snapshot_across_a_change() {
        let mut harness = harness_with(tree(vec![row(0, TreeGroup::Scene, 0, "root")]));
        assert_eq!(harness.root_widget().row_count(), 2);

        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(
                &mut t,
                &tree(vec![
                    row(0, TreeGroup::Scene, 0, "root"),
                    row(1, TreeGroup::Scene, 1, "mesh"),
                ]),
            );
        });

        assert_eq!(harness.root_widget().row_count(), 3);
    }

    #[test]
    fn an_unchanged_snapshot_rebuilds_nothing() {
        let snap = tree(vec![row(0, TreeGroup::Scene, 0, "root")]);
        let mut harness = harness_with(snap.clone());
        let before = harness.root_widget().generation();

        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(&mut t, &snap);
        });

        assert_eq!(harness.root_widget().generation(), before);
    }

    #[test]
    fn a_press_selects_the_row_under_the_pointer() {
        let mut harness = harness_with(tree(vec![
            row(0, TreeGroup::Scene, 0, "root"),
            row(1, TreeGroup::Scene, 1, "mesh"),
        ]));

        // Row 0 is the "Scene" header; row 1 is `root`; row 2 is `mesh`.
        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 2.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected(), Some(entity(1)));
    }

    #[test]
    fn a_press_on_a_header_selects_nothing() {
        let mut harness = harness_with(tree(vec![row(0, TreeGroup::Scene, 0, "root")]));

        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 0.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected(), None);
    }
}
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p sway-editor scene_tree::`
Expected: FAIL to compile — `cannot find type SceneTree`.

- [ ] **Step 3: Implement `SceneTree`**

Prepend to `crates/sway-editor/src/scene_tree.rs`:

```rust
//! `SceneTree` -- the world hierarchy pane.
//!
//! Enumerates every entity in the world, grouped into Scene / Graph / Edges /
//! Other with a header per section (design §8). Rows are `Label` children
//! rather than painted text, for the same reason `NodeBox` uses one:
//! `imaging::Painter` takes only pre-shaped glyphs.
//!
//! The row set is rebuilt only when it differs from the previous frame, so a
//! steady-state world costs one comparison. `Portal` (Task 7) supplies
//! scrolling; this widget reports its full content height through `measure`
//! so `Portal` knows how far it can scroll. If measured entity counts ever
//! make the rebuild comparison too slow, `VirtualScroll` is the escape hatch
//! -- measure before reaching for it.

use bevy_ecs::entity::Entity;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, NewWidget, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetMut,
    WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;
use sway_graph::NodeId;

use crate::snapshot::{TreeGroup, WorldSnapshot};

/// Height of one row, in logical pixels.
pub const ROW_HEIGHT: f64 = 20.0;
/// Horizontal indent per depth level.
const INDENT: f64 = 14.0;
/// Left padding before the first indent level.
const PADDING: f64 = 8.0;
/// Natural width reported when nothing constrains this widget.
const NATURAL_WIDTH: f64 = 240.0;

/// What a [`SceneTree`] reports upward when a row is pressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneTreeAction {
    pub entity: Entity,
    /// `Some` when the row is a graph node, which is what lets a tree
    /// selection highlight a node box in the canvas.
    pub node_id: Option<NodeId>,
}

/// One laid-out row: either a section header or an entity.
struct Row {
    pod: WidgetPod<Label>,
    depth: usize,
    /// `None` for a section header, which is not selectable.
    entity: Option<Entity>,
    node_id: Option<NodeId>,
}

/// The world hierarchy pane.
pub struct SceneTree {
    rows: Vec<Row>,
    /// The `(entity, label, depth)` triples the current rows were built from,
    /// compared against the next snapshot to decide whether to rebuild.
    signature: Vec<(Option<Entity>, String, usize)>,
    /// Bumped on every actual rebuild; lets a test assert that an unchanged
    /// snapshot did nothing.
    generation: u64,
    selected: Option<Entity>,
}

impl Default for SceneTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneTree {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            signature: Vec::new(),
            generation: 0,
            selected: None,
        }
    }

    /// Total rows, headers included.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// How many times the row set has actually been rebuilt.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The currently selected entity, if any.
    pub fn selected(&self) -> Option<Entity> {
        self.selected
    }

    fn content_height(&self) -> f64 {
        self.rows.len() as f64 * ROW_HEIGHT
    }
}

fn group_header(group: TreeGroup) -> &'static str {
    match group {
        TreeGroup::Scene => "SCENE",
        TreeGroup::Graph => "GRAPH",
        TreeGroup::Edges => "EDGES",
        TreeGroup::Other => "OTHER",
    }
}

/// The `(entity, label, depth)` signature a snapshot would produce, headers
/// included. Computed without touching the widget tree so it can be compared
/// against the current one before deciding to rebuild.
fn signature_of(snap: &WorldSnapshot) -> Vec<(Option<Entity>, String, usize)> {
    let mut out = Vec::with_capacity(snap.tree.len() + 4);
    let mut current: Option<TreeGroup> = None;
    for row in &snap.tree {
        if current != Some(row.group) {
            current = Some(row.group);
            out.push((None, group_header(row.group).to_string(), 0));
        }
        out.push((Some(row.entity), row.label.clone(), row.depth));
    }
    out
}

// --- MARK: WIDGETMUT
impl SceneTree {
    /// Rebuilds the row set from a snapshot, but only if it actually differs
    /// from the current one.
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let signature = signature_of(snap);
        if signature == this.widget.signature {
            return;
        }

        for row in &mut this.widget.rows {
            this.ctx.remove_child(&mut row.pod);
        }
        this.widget.rows.clear();

        let mut current: Option<TreeGroup> = None;
        for row in &snap.tree {
            if current != Some(row.group) {
                current = Some(row.group);
                this.widget.rows.push(Row {
                    pod: Label::new(group_header(row.group)).prepare().to_pod(),
                    depth: 0,
                    entity: None,
                    node_id: None,
                });
            }
            this.widget.rows.push(Row {
                pod: Label::new(row.label.clone()).prepare().to_pod(),
                depth: row.depth,
                entity: Some(row.entity),
                node_id: row.node_id,
            });
        }

        this.widget.signature = signature;
        this.widget.generation += 1;
        if this
            .widget
            .selected
            .is_some_and(|sel| !snap.tree.iter().any(|row| row.entity == sel))
        {
            this.widget.selected = None;
        }
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    /// Sets which entity is highlighted. Used by the selection sync in Task 8.
    pub fn set_selected(this: &mut WidgetMut<'_, Self>, entity: Option<Entity>) {
        if this.widget.selected == entity {
            return;
        }
        this.widget.selected = entity;
        this.ctx.request_paint_only();
    }
}

impl Widget for SceneTree {
    type Action = SceneTreeAction;

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for row in &mut self.rows {
            ctx.register_child(&mut row.pod);
        }
    }

    /// Reports the full content height on `MaxContent`, which is what
    /// `Portal` asks for when deciding how far it can scroll (its `layout`
    /// calls `compute_size` with `LenDef::MaxContent` on any unconstrained
    /// axis).
    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        match (axis, len_req) {
            (_, LenReq::FitContent(space)) => space,
            (_, LenReq::MinContent) => Length::ZERO,
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(self.content_height()),
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(NATURAL_WIDTH),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, row) in self.rows.iter_mut().enumerate() {
            let x = PADDING + row.depth as f64 * INDENT;
            let row_size = Size::new((size.width - x).max(0.0), ROW_HEIGHT);
            ctx.run_layout(&mut row.pod, row_size);
            ctx.place_child(&mut row.pod, Point::new(x, index as f64 * ROW_HEIGHT));
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Paints the selection band and the header backgrounds; the row text is
    /// each `Label` child's own job, painted after this.
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let width = NATURAL_WIDTH.max(
            self.rows
                .iter()
                .map(|row| PADDING + row.depth as f64 * INDENT)
                .fold(0.0, f64::max),
        );
        for (index, row) in self.rows.iter().enumerate() {
            let band = Rect::new(0.0, index as f64 * ROW_HEIGHT, width, (index + 1) as f64 * ROW_HEIGHT);
            if row.entity.is_none() {
                painter.fill_rect(band, Color::from_rgb8(44, 46, 54));
            } else if row.entity == self.selected {
                painter.fill_rect(band, Color::from_rgb8(90, 120, 200));
            }
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        let PointerEvent::Down(PointerButtonEvent { button: Some(PointerButton::Primary), state, .. }) =
            event
        else {
            return;
        };
        let local = ctx.local_position(state.position);
        let index = (local.y / ROW_HEIGHT).floor();
        let Some(row) = usize::try_from(index as i64).ok().and_then(|i| self.rows.get(i)) else {
            return;
        };
        // A header is not selectable.
        let Some(entity) = row.entity else {
            ctx.set_handled();
            return;
        };
        self.selected = Some(entity);
        ctx.submit_action::<Self::Action>(SceneTreeAction { entity, node_id: row.node_id });
        ctx.request_paint_only();
        ctx.set_handled();
    }

    fn accessibility_role(&self) -> Role {
        Role::Tree
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        self.rows.iter().map(|row| row.pod.id()).collect()
    }

    fn accepts_pointer_interaction(&self) -> bool {
        true
    }
}
```

Remove the now-unused `NewWidget` import if clippy flags it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-editor scene_tree::`
Expected: PASS — 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-editor/src/scene_tree.rs
git commit -m "$(cat <<'EOF'
feat(editor): the SceneTree pane

Every entity in the world, grouped Scene / Graph / Edges / Other with a
header per section. Rows rebuild only when the row set actually differs, so
a steady-state world costs one comparison per frame.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Three panes, wired to the live world

The first task that puts something new on screen.

**Files:**
- Modify: `crates/sway-editor/src/lib.rs`
- Modify: `crates/sway-app/src/presenter.rs:104-146`

**Interfaces:**
- Consumes: `SceneTree::apply_snapshot`, `GraphCanvas::apply_snapshot`, `snapshot::capture`.
- Produces:
  - `sway_editor::EditorUi::apply_snapshot(&mut self, snap: &WorldSnapshot)`
  - `sway_editor::SCENE_TREE_TAG: WidgetTag<SceneTree>`, `sway_editor::GRAPH_CANVAS_TAG: WidgetTag<GraphCanvas>`

- [ ] **Step 1: Build the new root**

In `crates/sway-editor/src/lib.rs`, delete `VIEWPORT_WIDTH`, `VIEWPORT_HEIGHT`, and the whole body of `graph_root`, replacing it with:

```rust
use masonry::core::WidgetTag;
use masonry::widgets::{Portal, Split};
use masonry_core::kurbo::Axis;
use masonry::layout::AsUnit;

use crate::scene_tree::SceneTree;
use crate::snapshot::WorldSnapshot;

/// Reaches the hierarchy pane from `EditorUi::apply_snapshot`.
pub const SCENE_TREE_TAG: WidgetTag<SceneTree> = WidgetTag::named("sway-scene-tree");
/// Reaches the graph pane from `EditorUi::apply_snapshot`.
pub const GRAPH_CANVAS_TAG: WidgetTag<GraphCanvas> = WidgetTag::named("sway-graph-canvas");

/// Builds the root widget: three panes, split twice.
///
/// ```text
/// +--------+------------------------------+
/// | SCENE  |      bevy viewport           |
/// |        |                              |
/// | v root +------------------------------+
/// |  v rig |  graph canvas (pan/zoom)     |
/// +--------+------------------------------+
/// ```
///
/// The Bevy viewport is a sibling of the graph canvas now, not a child of it
/// at a hardcoded rect. `external::viewport_rect` locates it by scanning the
/// `VisualLayerPlan` for the `External` layer, which does not care where in
/// the tree the widget sits, so the presenter needs no change for this.
///
/// Both content panes carry a `WidgetTag` so `apply_snapshot` can reach them
/// typed, without downcasting through the `Split`s.
fn graph_root() -> NewWidget<dyn Widget> {
    let tree = Portal::new(SceneTree::new().prepare().with_tag(SCENE_TREE_TAG))
        .constrain_horizontal(true)
        .prepare();

    let viewport = ViewportPlaceholder::new().prepare();
    let canvas = GraphCanvas::new().prepare().with_tag(GRAPH_CANVAS_TAG);

    let right = Split::new(viewport, canvas)
        .split_axis(Axis::Vertical)
        .split_fraction(0.55)
        .draggable(true)
        .solid_bar(true)
        .prepare();

    Split::new(tree, right)
        .split_axis(Axis::Horizontal)
        .split_point_from_start(260.0.px())
        .draggable(true)
        .solid_bar(true)
        .prepare()
        .erased()
}
```

Adjust the imports at the top of the file: `NewWidget`, `Widget`, `GraphCanvas`, `ViewportPlaceholder` are already there; drop `masonry::properties::Dimensions` if nothing else uses it.

- [ ] **Step 2: Add `EditorUi::apply_snapshot`**

In `impl EditorUi`, before `redraw`:

```rust
    /// Pushes one frame's world snapshot into both content panes.
    ///
    /// Called by the host immediately before [`redraw`](Self::redraw). Each
    /// pane decides for itself whether the snapshot actually changed anything
    /// -- `SceneTree` compares its row signature, `GraphCanvas` reconciles by
    /// `NodeId` -- so calling this every frame is cheap in the steady state.
    pub fn apply_snapshot(&mut self, snap: &WorldSnapshot) {
        self.root.edit_widget_with_tag(SCENE_TREE_TAG, |mut tree| {
            SceneTree::apply_snapshot(&mut tree, snap);
        });
        self.root.edit_widget_with_tag(GRAPH_CANVAS_TAG, |mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, snap);
        });
    }
```

- [ ] **Step 3: Wire the presenter**

In `crates/sway-app/src/presenter.rs`, add the forwarding method to `EditorPresenter`, next to `handle_winit_event`:

```rust
    /// Reads one frame's graph state out of the Bevy world and pushes it into
    /// the widget tree.
    ///
    /// Called from `present` between the previous frame's `app.update()` and
    /// this frame's masonry redraw, which is the one place the two halves of
    /// the process meet. The snapshot therefore reflects the world as of the
    /// *previous* frame's update: `present` redraws masonry first so a
    /// viewport resize costs no frame of lag (see step 1 below), and that
    /// ordering is load-bearing. A one-frame lag in a diagnostic view is
    /// invisible; reordering `present` for it would not be.
    fn apply_snapshot(&mut self, app: &App) {
        let snapshot = sway_editor::snapshot::capture(app.world());
        self.editor.apply_snapshot(&snapshot);
    }
```

and call it at the top of `present`, before the masonry redraw:

```rust
        // 0. The world snapshot, from the previous frame's `app.update()`.
        self.apply_snapshot(app);

        // 1. Masonry first.
        let plan = self.editor.redraw();
```

`present` already takes `app: &mut App`, so `&*app` coerces; if the borrow checker objects to the later `app.update()`, call `self.apply_snapshot(app)` on its own statement before any other use of `app`, which the ordering above already does.

- [ ] **Step 4: Build and run the app**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: PASS.

Run: `cargo run -p sway-app -- --editor`
Expected, by eye:
- A left pane listing `SCENE`, `GRAPH`, `EDGES`, `OTHER` sections with real entity rows under each; the demo graph's `Group #5` and `Mesh #2` nested under Scene.
- A top-right pane showing the live Bevy render of the demo scene.
- A bottom-right pane showing ten node boxes with real names (`Grid`, `Displace`, `MeshNode`, `StandardMaterialNode`, `Rgb`, `Group`, `MidiCC`, `MidiNote`, `Envelope`, `LFO`) at the positions authored in Task 1, connected by param and `Feeds` edges in two colours.
- The `LFO -> Group.rotation_y` edge visibly thickening and brightening as the LFO sweeps. This is the whole point of the feature; if nothing moves, check that `capture` is being called after `app.update()` and that `EdgeView::activity` is `Some` for that edge.
- Both split bars draggable.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-editor/src/lib.rs crates/sway-app/src/presenter.rs
git commit -m "$(cat <<'EOF'
feat(editor): three panes, wired to the live world

Hierarchy left, Bevy viewport top-right, graph canvas bottom-right, split
twice with draggable bars. The viewport is a sibling of the canvas now
rather than a child of it at a hardcoded 640x360 rect; viewport_rect finds
it by scanning for the External layer either way.

The presenter captures a snapshot each frame and pushes it into both panes
through their WidgetTags.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: Selection sync between the panes

What makes two panes better than two windows.

**Files:**
- Modify: `crates/sway-editor/src/lib.rs`
- Modify: `crates/sway-editor/src/canvas.rs`

**Interfaces:**
- Consumes: `SceneTreeAction`, `GraphCanvas::set_selected`, `SceneTree::set_selected`, `GraphCanvas::selected_node`.
- Produces:
  - `GraphCanvas::entity_of(&self, id: NodeId) -> Option<Entity>` — the canvas must remember each node's entity so a canvas selection can address a tree row.
  - `EditorUi::sync_selection(&mut self)` — called from `redraw`.

- [ ] **Step 1: Write the failing test**

Append to `crates/sway-editor/src/lib.rs`'s `mod tests`:

```rust
    use crate::canvas::GraphCanvas;
    use crate::scene_tree::SceneTree;
    use crate::snapshot::{NodeView, TreeGroup, TreeRow, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use kurbo::Point as KurboPoint;
    use sway_graph::NodeId;
    use winit::dpi::PhysicalSize;

    fn one_node_snapshot() -> WorldSnapshot {
        let entity = Entity::from_raw_u32(3).expect("valid entity id");
        WorldSnapshot {
            tree: vec![TreeRow {
                entity,
                group: TreeGroup::Graph,
                depth: 0,
                label: "LFO #1".to_string(),
                node_id: Some(NodeId(1)),
            }],
            nodes: vec![NodeView {
                entity,
                id: NodeId(1),
                name: "LFO".to_string(),
                pos: Some(KurboPoint::new(10.0, 10.0)),
            }],
            edges: Vec::new(),
        }
    }

    #[test]
    fn selecting_a_node_box_highlights_its_tree_row() {
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0);
        let snap = one_node_snapshot();
        ui.apply_snapshot(&snap);

        ui.root.edit_widget_with_tag(crate::GRAPH_CANVAS_TAG, |mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(1)));
        });
        ui.sync_selection();

        let selected = ui
            .root
            .edit_widget_with_tag(crate::SCENE_TREE_TAG, |tree| tree.widget.selected());
        assert_eq!(selected, Some(snap.nodes[0].entity));
    }

    #[test]
    fn selecting_a_graph_node_row_highlights_its_node_box() {
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0);
        let snap = one_node_snapshot();
        ui.apply_snapshot(&snap);

        ui.root.edit_widget_with_tag(crate::SCENE_TREE_TAG, |mut tree| {
            SceneTree::set_selected(&mut tree, Some(snap.nodes[0].entity));
        });
        ui.sync_selection();

        let selected = ui
            .root
            .edit_widget_with_tag(crate::GRAPH_CANVAS_TAG, |canvas| canvas.widget.selected_node());
        assert_eq!(selected, Some(NodeId(1)));
    }
```

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p sway-editor tests::selecting`
Expected: FAIL — `no method sync_selection`.

- [ ] **Step 3: Have the canvas remember entities**

In `crates/sway-editor/src/canvas.rs`, add `entity: Entity` to `NodeSlot`, set it in `apply_snapshot` (both the create and the update branch — a `NodeId` can outlive an entity across a recompile), and add:

```rust
    /// The world entity behind a node, so a canvas selection can address the
    /// matching tree row.
    pub fn entity_of(&self, id: NodeId) -> Option<Entity> {
        self.slots.get(&id).map(|slot| slot.entity)
    }
```

with `use bevy_ecs::entity::Entity;` added to the imports.

- [ ] **Step 4: Implement `sync_selection`**

In `crates/sway-editor/src/lib.rs`, add to `impl EditorUi`:

```rust
    /// Mirrors selection between the two panes.
    ///
    /// Whichever pane changed since the last call wins; if both changed, the
    /// canvas does, arbitrarily but deterministically. `NodeId` is the shared
    /// key, and a tree row that is not a graph node (a Bevy internal, an edge
    /// entity) selects within the tree and highlights nothing in the canvas.
    pub fn sync_selection(&mut self) {
        let canvas_selection = self
            .root
            .edit_widget_with_tag(GRAPH_CANVAS_TAG, |canvas| {
                canvas.widget.selected_node().and_then(|id| {
                    canvas.widget.entity_of(id).map(|entity| (id, entity))
                })
            });
        let tree_selection = self
            .root
            .edit_widget_with_tag(SCENE_TREE_TAG, |tree| tree.widget.selected());

        match (canvas_selection, tree_selection) {
            (Some((_, entity)), tree) if tree != Some(entity) => {
                self.root.edit_widget_with_tag(SCENE_TREE_TAG, |mut tree| {
                    SceneTree::set_selected(&mut tree, Some(entity));
                });
            }
            (None, Some(entity)) => {
                let node_id = self.last_snapshot_node_id(entity);
                self.root.edit_widget_with_tag(GRAPH_CANVAS_TAG, |mut canvas| {
                    GraphCanvas::set_selected(&mut canvas, node_id);
                });
            }
            _ => {}
        }
    }

    /// The `NodeId` for an entity, from the most recent snapshot. `None` for
    /// a row that is not a graph node.
    fn last_snapshot_node_id(&self, entity: Entity) -> Option<NodeId> {
        self.node_ids.get(&entity).copied()
    }
```

Give `EditorUi` a `node_ids: HashMap<Entity, NodeId>` field, initialised empty in `new`, and populate it at the top of `apply_snapshot`:

```rust
        self.node_ids = snap
            .nodes
            .iter()
            .map(|node| (node.entity, node.id))
            .collect();
```

Add `use std::collections::HashMap;`, `use bevy_ecs::entity::Entity;`, and `use sway_graph::NodeId;`.

- [ ] **Step 5: Call it every frame**

In `EditorUi::redraw`, before the anim-frame pump:

```rust
    pub fn redraw(&mut self) -> VisualLayerPlan {
        self.sync_selection();

        let now = Instant::now();
        // ... unchanged
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-editor`
Expected: PASS.

If `edit_widget_with_tag`'s closure cannot return a value borrowed from the widget, copy it out inside the closure — every value read here (`Option<NodeId>`, `Option<Entity>`) is `Copy`.

- [ ] **Step 7: Verify by eye**

Run: `cargo run -p sway-app -- --editor`
Expected: clicking a node box highlights its row in the left pane; clicking a graph-node row in the left pane highlights its box; clicking an `EDGES` or `OTHER` row highlights the row and nothing in the canvas.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-editor/src/lib.rs crates/sway-editor/src/canvas.rs
git commit -m "$(cat <<'EOF'
feat(editor): sync selection between the tree and the canvas

NodeId is the shared key. A tree row that is not a graph node -- a Bevy
internal, an edge entity -- selects within the tree and highlights nothing
in the canvas.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §2 crate boundary, stale crate-doc claim | 2 (steps 1–2), verified by 2 step 8 |
| §3 `capture(&World)`, called from `present`, ordering caveat | 2, 7 |
| §4 activity, continuous-only, auto-ranging | 2 (`continuous_value`), 5 (`normalised`, `edge_style`) |
| §5 `EditorPos`, fallback grid, seeding-not-binding | 1, 5 |
| §6 display names, `short_type_name` | 2 |
| §7 widget tree, viewport as sibling, deleted constants | 7 |
| §8 `SceneTree`, four groups, row labels, `Portal` | 3 (data), 6 (widget), 7 (`Portal`) |
| §9 snapshot-driven canvas, `NodeId` identity, `Label` child, edge kinds, `ParentEdge` excluded, drag-to-connect removed | 4, 5 |
| §10 selection sync | 8 |
| §11 testing | every task; §11's three widget assertions are Task 5 step 1 and Task 6 step 1 |
| §12 out of scope | nothing implements these |

**Known deviations from the spec, both deliberate:**

- Spec §8 says rows are `Vec<WidgetPod<Label>>` inside a `Portal`. The `Portal` wraps `SceneTree` from outside (Task 7) rather than living inside it, so `SceneTree` stays a plain widget that reports its content height — simpler, and it keeps `SceneTree`'s tests free of `Portal`.
- Spec §5's fallback is "indexed by its position in `CompiledGraph::plans`". Implemented as "indexed by its position in `WorldSnapshot::nodes`", which *is* the compiled order when a `CompiledGraph` exists (Task 2, `capture_nodes`) and a deterministic `NodeId` order when it does not. The spec's rule with a defined answer for the uncompiled case.

**Placeholder scan:** none. Every code step contains the code.

**Type consistency:** `WorldSnapshot`/`NodeView`/`EdgeView`/`EdgeKind` (Task 2) and `TreeRow`/`TreeGroup` (Task 3) are used unchanged in Tasks 5–8. `TreeRow` is defined in Task 2 without `group` and gains it in Task 3 — deliberate, and Task 3 step 4 replaces the whole struct rather than patching it. `GraphCanvas::selected_node` changes from `Option<usize>` to `Option<NodeId>` in Task 5 and is used at that type in Task 8. `NodeBox::set_label(&mut WidgetMut, &str)` is defined in Task 4 and called with `&view.name` in Task 5.
