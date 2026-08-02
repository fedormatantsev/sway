# Scene and Graph Views — Design

**Date:** 2026-08-02
**Status:** Approved, pre-implementation
**Slots into:** the roadmap between M2b and M3 (`2026-07-25-sway-design.md` §5)

## 1. What this is

The editor currently shows six hardcoded boxes labelled `Source`, `Filter`,
`Transform`, `Output`, `Debug View`, `Camera`. None of them correspond to
anything. The real graph — node entities, edge entities, the compiled order,
the live port arena — exists in the Bevy world and is visible nowhere.

This adds the first two views onto that real state: a **scene (world)
hierarchy pane** and a **graph pane** showing the actual topology with live
activity on its edges. Both are read-only.

The purpose is diagnostic. M3 (transport and beat lock) is about a phase
estimate that is either right or wrong, and debugging it against a black
window and a log file is worse than debugging it against a picture of the
graph. Getting this in first is cheap and pays for itself immediately.

### Relationship to M7

This is **the first real slice of M7's editor**, not a throwaway overlay. The
graph→UI read path built here is the one M7 extends. Concretely: M7 adds an
inspector, topology editing, and per-port detail *on top of* this; it does not
replace the snapshot, the pane layout, or the widget identity model.

What it is *not* is M7's feature set. Nothing here edits anything.

## 2. Crate boundary

`sway-editor` gains `sway-graph`, `bevy_ecs`, `bevy_transform`, and
`bevy_reflect`.

The main design §3 already licenses this — "The editor links `sway-graph`
regardless" — and §2.8 requires it: "The editor walks `TypeRegistry` directly
for node types and their field metadata, and reads the live port arena to
animate values on edges." That is not expressible across a crate boundary that
forbids `bevy_ecs`.

`sway-editor` still does **not** depend on `bevy` (the full facade),
`bevy_render`, `wgpu`, `vello`, or `imaging_vello`. The M1b invariant that
survives is the *rendering* one: nothing in `sway-editor` creates a device or
touches a pipeline. `sway-editor`'s crate doc currently claims it depends on
none of `wgpu`, `vello`, `imaging_vello`, or `bevy`; that claim is rewritten
rather than left standing, because it will otherwise read as a violated
invariant instead of a narrowed one.

## 3. Data flow — one snapshot per frame

A new module, `sway_editor::snapshot`, is the entire graph→UI read path.

```rust
pub struct WorldSnapshot {
    pub tree:  Vec<TreeRow>,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<EdgeView>,
}

pub fn capture(world: &World) -> WorldSnapshot;
```

`capture` is a pure function of `&World`. It reads:

- `GraphNode` entities, for `NodeId` and `NodeTypeId`;
- `ParamEdge` / `FeedsEdge` / `ParentEdge` entities with their `EdgeFrom` /
  `EdgeTo`;
- `ChildOf` / `Children`, for the scene tree;
- `NodeTypeRegistry`, for display names;
- `PortArena` and `CompiledGraph`, for live continuous values and for the
  fallback node ordering (§6).

Nothing is pushed to it. This satisfies main design §2.11's "The editor
likewise reads rather than receives: live port values come from the arena and
live node values from components, with nothing pushed to it."

`capture` returns a plain struct with no masonry types in it, so it is
testable against a headless `App` with no widget tree at all. That is where
most of the tests live (§10).

### Where it is called

`EditorPresenter::present` calls `capture` and applies the result through
`RenderRoot::edit_root_widget`, immediately before `editor.redraw()`. That one
call site is the only place `sway-app` glues the two halves together.

**Ordering caveat, accepted deliberately.** `present` redraws masonry *before*
`app.update()`, so that a viewport resize costs no frame of lag (this ordering
is load-bearing and predates this work). The snapshot therefore reflects the
world as of the previous frame's update. For a diagnostic view a one-frame lag
is invisible, and `present`'s order is not changed for it.

## 4. Edge activity — continuous only

An `EdgeView` carries `activity: Option<f32>`: the raw value in the source
port's arena slot, when that slot downcasts to `f32`.

**Event edges get no activity, by design.** An event occupies exactly one
tick. At a 120 Hz tick and a 60 fps frame rate, a frame-rate sampler observes
roughly half of them, so a MIDI note would pulse or not pulse at random — a
worse signal than no signal, for the one question ("is my MIDI arriving") the
view would be consulted about. The alternative, an activity accumulator
written by `graph_tick`, is rejected here: it puts an editor-only write path
in the hot tick, in direct contradiction of §2.11.

Consequently event edges are drawn in a distinct, static style, and so are
continuous edges whose value is not an `f32` (colour, vector). The code says
so at the point where `activity` is `None`, so this reads as a decision rather
than an oversight.

Making event activity correct is M7 work, and the honest fix there is a
per-edge ring buffer written by the tick — worth doing once the editor is a
first-class consumer, not before.

### Normalisation

Port values have no common scale: an LFO's amplitude here is π, a MIDI CC is
0..1, an envelope is 0..1. The canvas keeps a running observed min/max per
edge and maps the current value into that range, driving stroke width and
brightness. Auto-ranging avoids a per-node-type table of expected ranges,
which would be one more thing to keep in sync with the node set.

## 5. `EditorPos`

```rust
#[derive(Component, Reflect)]
pub struct EditorPos(pub Vec2);   // sway-graph
```

Node positions are authored by hand in the graph builder (today,
`demo_graph.rs`), as a component on the node entity rather than a field in the
builder. Same authoring ergonomics; a durable home.

The progression is: authored in Rust now → serialized by M4's project format →
written back by drag at M7. A field in `demo_graph.rs` would have to be
rebuilt at each of those steps.

**Fallback.** A node with no `EditorPos` is placed at a deterministic slot on
a fixed grid, indexed by its position in `CompiledGraph::plans` — column
`i / 6`, row `i % 6`, at the node box's own pitch. A node added without a
position is then misplaced rather than invisible, and two such nodes never
land on top of each other.

**Seeding, not binding.** `EditorPos` is read when a node box first appears
and never again; the widget owns its position from then on. Otherwise the next
frame's snapshot would snap a dragged node straight back, which is the
concrete reason dragging survives while position persistence does not (§12).
Positions therefore reset on restart until M4 serializes them and M7 writes
them back.

## 6. Display names

`NodeTypeEntry::name` is `core::any::type_name::<N>()`:
`sway_nodes::lfo::LFO`, and for the generated node types of §2.4,
`sway_nodes::material::MaterialNode<bevy_pbr::StandardMaterial>`.

The snapshot shortens this to the last path segment, preserving the last
segment of any generic argument — `LFO`, `MaterialNode<StandardMaterial>`.

This shortening is temporary. M4 introduces short registered names in the
project format for exactly this reason ("which no one should have to type or
read in a hand-authored document"), and when it does, the shortening is
deleted and the registered name used directly.

## 7. Widget tree

```
Split(horizontal)
├── SceneTree
└── Split(vertical)
    ├── ViewportPlaceholder
    └── GraphCanvas
```

`masonry::widgets::split::Split` supplies draggable dividers, so pane sizing
needs no code of ours.

The Bevy viewport stops being a `GraphCanvas` child at a hardcoded
640×360 rect and becomes a direct child of the vertical split, sized by
layout. `sway_editor::external::viewport_rect` locates it by scanning the
`VisualLayerPlan` for the `External` layer, which is unaffected by where in
the tree the widget sits — so `EditorPresenter` needs no change beyond the
`capture` call.

Deleted by this: `GraphCanvas::with_viewport`, `ViewportSlot`, and the
`VIEWPORT_WIDTH` / `VIEWPORT_HEIGHT` constants in `sway_editor` that had to be
kept in step with `EDITOR_VIEWPORT_SIZE` in `sway-app`.

## 8. `SceneTree`

Enumerates **every entity in the world**, not only spatial ones — the honest
world view, including the entities the graph did not make.

Rows are flattened depth-first and grouped under four headers:

| Group | Contents |
|---|---|
| **Scene** | entities with a `Transform`, nested by `ChildOf` |
| **Graph** | `GraphNode` entities without a `Transform` — geometry operators, signal nodes |
| **Edges** | `ParamEdge` / `FeedsEdge` / `ParentEdge` entities |
| **Other** | everything else, including Bevy's own internals |

Grouping is what makes "all entities" readable; a flat forest of several
hundred roots is not.

### Row labels

Best-effort, in order: a `Name` component wins; otherwise a `GraphNode`'s
shortened type name plus its `NodeId`; otherwise the entity id plus its first
three component type names, shortened the same way (§6).

### Structure

A `Vec<WidgetPod<Label>>` inside a `Portal` for scrolling. `Label` rather than
painting text directly, because `imaging::Painter` exposes only `glyphs`
(pre-shaped) and shaping is masonry's job, not ours.

The row set is rebuilt only when it differs from the previous frame; a
steady-state world costs one comparison per frame. If measured entity counts
make that too slow, `masonry::widgets::virtual_scroll::VirtualScroll` is the
escape hatch. Measure before reaching for it.

## 9. `GraphCanvas`

Driven by the snapshot instead of by builder calls.

- `with_node` / `with_edge` and the insertion-index node identity are removed.
  Nodes are keyed by `NodeId`, so a node keeps its `WidgetId` — and therefore
  its drag state and selection — across snapshots. This is the identity model
  M7 needs, and getting it right now is most of the reason this is a slice of
  M7 rather than an overlay.
- `NodeBox` gains a `Label` child. This is what finally makes it draw the
  label it has been carrying since M1b (its `paint` renders a rounded rect and
  nothing else; the label reaches only the accessibility node).
- Edges carry their kind. Param and `Feeds` edges render in distinct colours.
  **`ParentEdge` is excluded from the canvas** — the tree pane shows parenting
  already, and drawing it twice makes the canvas harder to read for no gain.
- Activity is applied per §4.

**Drag-to-connect is removed.** It currently appends to a local `Vec` of
edges, inventing connections that exist in no graph. Against real data that is
a lie, and it stays removed until M7 makes topology editing real.

Pan, zoom, node dragging, and selection are kept.

## 10. Selection

Selecting a node box highlights its row in the `SceneTree`; selecting a
graph-node row highlights its box. `NodeId` is the shared key, and both panes
already have a selection concept.

Rows that are not graph nodes (Bevy internals, edge entities) select within
the tree and highlight nothing in the canvas.

## 11. Testing

Following main design §4, and its rule that rendering is verified by eye.

**`capture(&World)` — the bulk of it, entirely masonry-free.** Built against a
headless `App` in the shape `demo_graph.rs`'s tests already use:

- rows group correctly into Scene / Graph / Edges / Other;
- `ChildOf` nesting order is depth-first and stable;
- name shortening handles both a plain and a generic node type;
- a node without `EditorPos` gets its `CompiledGraph::plans` fallback slot,
  and two such nodes do not collide;
- a dragged node keeps its dragged position across a snapshot that still
  carries its original `EditorPos` (§5's seeding rule);
- `activity` is `Some` for a continuous `f32` edge, `None` for an event edge,
  and `None` for a continuous edge carrying a non-`f32` value.

**Widgets, via `masonry_testing::TestHarness`:**

- the tree's rows track the snapshot across a change;
- a node box whose `NodeId` survives a snapshot keeps its `WidgetId`;
- the existing zoom hit-test and pan tests still pass under the new tree
  (these are M1b's gate assertions for the masonry bet and must not regress).

**No pixel tests.**

## 12. Out of scope

Inspector panel. Any editing — of params, of topology, of the scene.
Per-port values displayed on nodes. Event-edge animation. Persisting dragged
positions. A transport readout: M3 will want one, but there is no transport
yet, and inventing the display before the thing it displays is backwards.
