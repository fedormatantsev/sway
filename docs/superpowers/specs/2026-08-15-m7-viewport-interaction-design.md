# Sway — M7, viewport interaction

**Date:** 2026-08-15
**Status:** Design approved; implementation plan to follow
**Milestone:** M7 in [`2026-08-09-mvp-roadmap-design.md`](2026-08-09-mvp-roadmap-design.md)
**Architecture:** [`docs/architecture.md`](../../architecture.md) is the authority
on current-state design. The amendments this milestone makes to it are recorded
under "What this changes in the documents" below.
**Inherits:** [`2026-08-10-m6-editor-write-half-findings.md`](../reports/2026-08-10-m6-editor-write-half-findings.md)
§"What M7 inherits".

## The deliverable

**The scene is composed by dragging, not by typing numbers.**

Today the viewport is an inert hole. `ViewportPlaceholder` exists only to mark
a rectangle for the compositor to fill with Bevy's texture; it declines pointer
interaction outright (`accepts_pointer_interaction` returns `false`) so that
clicks fall through to the widgets behind it. Nothing forwards a pointer or a
key into the Bevy world, there is exactly one camera and the document owns it,
and selection lives in two masonry widgets that reconcile against each other
every frame.

Four things follow from making that rectangle live: an input path, a camera to
navigate with, a way to say which entity is selected that all three views agree
on, and a gizmo.

## Decisions taken for this milestone

### M7-1 — Viewport input travels as plain data on its own channel

`sway-editor` depends on `bevy_ecs`, `bevy_math`, `bevy_reflect`,
`bevy_transform` and `sway-graph`, and deliberately on none of `bevy` (the
facade), `bevy_render`, `wgpu` or `vello`. That invariant decides the shape of
this milestone: the widget under the pointer cannot cast a ray, cannot see a
`Camera`, and cannot own a `Transform` drag. It can only produce data.

```rust
// sway-graph, beside command.rs
pub enum ViewportInput {
    /// Normalized to the viewport rect: [0,1]², origin top-left.
    Down   { button: ViewportButton, pos: Vec2, modifiers: ViewportModifiers },
    Move   { pos: Vec2, modifiers: ViewportModifiers },
    Up     { button: ViewportButton, pos: Vec2 },
    Cancel,
    /// Positive `y` dollies in. Reduced to logical pixels by the widget, the
    /// same way `GraphCanvas` reduces its own scroll; `pos` is normalized.
    Scroll { delta: Vec2, pos: Vec2, modifiers: ViewportModifiers },
    /// A pinch magnification delta, from a trackpad.
    Pinch  { delta: f32 },
    /// Gizmo-mode keys only; every other key bubbles past the viewport.
    Key    { key: ViewportKey },
}

#[derive(Resource)]
pub struct ViewportInputRx(pub Receiver<ViewportInput>);
```

`ViewportButton` (`Primary` / `Secondary`), `ViewportModifiers` (`alt`, `shift`,
`control`, `meta` flags) and `ViewportKey` (`W` / `E` / `R`) are `sway-graph`'s
own small types. `sway-graph` cannot name masonry's `PointerButton` or
`Modifiers`, and the world side has no business knowing masonry exists; the
widget translates at the boundary, which is the same one-way conversion
`GraphCanvas` already does when it turns a `ScrollDelta` into logical pixels.

This lives in `sway-graph` for the same reason `EditorPos` and `EditorCommand`
already do: it is editor↔world plumbing expressible in plain `bevy_ecs` data,
and both ends already depend on that crate. `sway-runtime` gains a `sway-graph`
dependency to consume it — the first time it has needed one, and consistent
with the layering (runtime sits above the engine).

**A separate channel from `EditorCommand`, not a new variant.** The consumers
need `MeshRayCast`, camera queries and `Gizmos`; `apply_editor_commands` is an
exclusive `&mut World` system, and folding ray casting into it would mean
hand-rolling `SystemState` for params Bevy would otherwise supply. Two channels
drained by two systems in `PreUpdate`, commands first, costs nothing and keeps
both systems ordinary.

**Positions are normalized to the viewport rectangle** — `[0,1]²`, origin at
the top-left. Not logical window pixels, and not physical ones. The reason is
`Camera::viewport_to_ndc`, which divides by `logical_viewport_rect()`: for a
`RenderTarget::TextureView` that rect is the texture's own size, which is
physical pixels, while masonry's widget coordinates are logical. On a Retina
display those differ by a factor of two, and every ray in this milestone would
be wrong by it. Normalizing at the boundary makes the world side
`pos * camera.logical_viewport_size()` with no scale factor anywhere, and makes
drag sensitivity resolution-independent for free.

Rejected: defining the enum in `sway-editor` and having `sway-app` translate it
into a twin type in `sway-runtime`. It keeps a viewport concept out of
`sway-graph` at the cost of a duplicated enum and a hand-maintained mapping
between them, with nothing to catch a drift between the halves.

### M7-2 — `Viewport` replaces `ViewportPlaceholder`

The widget keeps everything `ViewportPlaceholder` does — `PaintLayerMode::External`
asserted every paint, the `request_anim_frame` self-dirtying loop that keeps it
in every `VisualLayerPlan`, the full-content-box clip — and adds:

- `accepts_pointer_interaction` returns `true`. This is a behaviour change with
  a documented reason to be careful: the placeholder declined interaction so
  hits would fall through to overlapping `NodeBox`es. That overlap no longer
  exists — since M1b Task 5 the viewport is a `Split` sibling of the graph
  canvas, not a child of it — so accepting is now correct rather than
  destructive.
- `accepts_focus` returns `true`, and a primary `Down` calls
  `ctx.request_focus()`, which is what makes W/E/R reach `on_text_event` at all.
- A `Sender<ViewportInput>`, handed in at construction and sent to directly
  from the event handlers — exactly what `GraphCanvas` already does with its
  `Sender<EditorCommand>`. No queue, and no drain plumbed through
  `EditorPresenter` and the shell.

The widget holds no interaction state at all: no drag anchor, no "is orbiting"
flag. Those live on the world side, because that is where the gesture is
resolved. The widget does call `ctx.capture_pointer()` on `Down` so a drag that
leaves the rectangle keeps delivering `Move`.

**Known risk, carried from M6 explicitly.** Tasks 13 and 14 of M6 each found a
defect where brief-literal masonry wiring compiled, passed its own tests, and
did nothing in the real app — a `create_layer`d widget's actions dead-ending,
and a missing `request_focus()`. Focus and event routing is a two-for-two
failure class in this codebase. Every event-path behaviour in this milestone is
therefore tested through real masonry dispatch (`masonry_testing`'s harness
driving `process_pointer_event` / `process_text_event`), never through a
`_for_test` bypass seam.

### M7-3 — An editor camera the document cannot see

```rust
// sway-runtime
#[derive(Component)]
#[require(Camera3d)]
pub struct EditorCamera {
    pub pivot: Vec3,
    pub yaw: f32,      // radians
    pub pitch: f32,    // radians, clamped just inside ±π/2
    pub distance: f32, // > 0
}
```

Spawned by `EditorViewportPlugin`, which `sway-app` adds only under `--editor`.
It carries no `EditorPos` and no `DocId`, so `capture_nodes` never draws it on
the graph canvas (M6-4 made that walk every `EditorPos` entity) and
`to_document` never emits it (emit walks `DocId` carriers). It is invisible to
every existing mechanism without any of them needing a special case.

Navigation is a pure function of those four numbers, which is what makes it
testable without a window:

```rust
pub fn orbit_transform(cam: &EditorCamera) -> Transform;
```

- **Alt + primary drag** orbits: `yaw -= dx * SENSITIVITY`, `pitch` likewise and
  clamped. `dx`/`dy` are normalized-viewport deltas (M7-1), so a full-width drag
  sweeps the same angle at any window size or DPI.
- **Alt + secondary drag** pans: the pivot moves along the camera's own right
  and up axes, scaled by `distance` so panning feels the same at every zoom.
- **Scroll and pinch** dolly: `distance *= exp(-delta * RATE)`, clamped to a
  positive minimum so the pivot can never be passed through.

The binding is Alt-first (Maya/Unity-shaped) rather than middle-mouse
(Blender-shaped) because this is developed and performed on a Mac laptop, where
there is no middle button on the trackpad, and because it leaves an unmodified
primary click free — which picking and the gizmo both need.

Pose is not persisted. There is no editor-state sidecar file and no
"frame selected"; the pivot starts at the origin and panning is how you get
elsewhere.

### M7-4 — Exactly one camera is active, chosen by a resource

`headless::retarget_cameras` points **every** camera at the viewport texture
each `Update`. With two cameras that is not a bug in itself, but both would
render into the same target and whichever ran last would win — the same
camera-collision hazard `main.rs` already documents for the `--demo` paths.

```rust
// sway-runtime
#[derive(Resource, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportCamera { #[default] Editor, Scene }
```

A system writes `Camera::is_active` from it: `EditorCamera` active iff `Editor`,
every `SceneCamera` active iff `Scene`. `retarget_cameras` is left alone.
A show build has no `EditorCamera` and no `EditorViewportPlugin`, so nothing
touches `is_active` and the scene camera renders exactly as it does today.

**The query is scoped to those two markers, never "every camera".** The gizmo
renderer spawns an overlay camera of its own (M7-8); deactivating it would take
the gizmo off the screen. `retarget_cameras` still points it at the viewport
texture, which is what it needs.

The toggle is a transport-bar button, matching M6's Open / Save / Save As
toolbar, and travels as a `FileRequest`-shaped UI intent — a new
`ViewRequest::ToggleCamera` drained by the shell, which flips the resource. No
key binding: this is a once-in-a-while action, and keeping it off the keyboard
keeps the viewport's key handling to the three gizmo-mode keys.

Selecting `Scene` when the document authors no `SceneCamera` shows nothing.
That is honest and needs no special case.

### M7-5 — Selection moves into the world

Selection is currently a field on `SceneTree` and a field on `GraphCanvas`,
reconciled every frame by `EditorUi::sync_selection`. A third participant that
lives on the far side of a channel — the viewport — cannot join that scheme:
its answer arrives a frame after the click, and `sync_selection` would treat the
stale widget value as the winner and undo it. The same reconciliation is
already the cause of the inspector flicker M6 left open (selecting a tree row
whose entity has no canvas node reverts after one frame).

```rust
// sway-graph
#[derive(Resource, Default)]
pub struct Selection(pub Option<Entity>);

// EditorCommand gains:
Select { entity: Option<Entity> },
```

`WorldSnapshot` gains `selection: Option<Entity>`. The tree, the canvas and the
gizmo all read it; the tree, the canvas and the picker all write it through the
command channel. `sync_selection` is deleted.

**Widgets stop owning selection entirely.** `SceneTree::selected` and
`GraphCanvas::selected` become pure render state, pushed from the snapshot
every frame by `EditorUi::apply_snapshot` (both setters already early-return
when unchanged). A click no longer sets them; it only sends
`EditorCommand::Select`, so `SceneTree` gains the `Sender<EditorCommand>` the
canvas and the inspector already hold. The highlight therefore appears about
two frames after the click — the command lands in the next `app.update()`, and
the snapshot that reports it is captured the frame after — which is 16–33 ms
and imperceptible for a selection highlight.

This is what removes the flicker rather than papering over it: widget state
becomes a pure function of the snapshot, so there is no second opinion left to
revert to. It also collapses the presenter's two-step capture — `capture` reads
`Selection` itself and fills `inspector` in the same pass, so
`EditorPresenter::apply_snapshot` no longer has to ask the widget tree who is
selected before it can inspect anything.

`Selection` is a `sway-graph` resource rather than a `sway-runtime` one because
`sway-editor`'s snapshot code reads it and must not depend on `bevy_render`, and
because `EditorPos` establishes the precedent of editor-facing state living
beside the engine.

### M7-6 — Picking uses `MeshRayCast` directly; no plugin is needed

The roadmap's open question — whether `MeshRayCast` needs resources only
`MeshPickingPlugin` initialises — is answered by reading the pinned source
rather than by building the hand-rolled fallback:

- `bevy_picking-0.19.0/src/mesh_picking/ray_cast/mod.rs:173` declares
  `MeshRayCast` as a `SystemParam` over `Res<Assets<Mesh>>`, three `Local`s and
  two `Query`s. There is no plugin-initialised resource among them.
  `Assets<Mesh>` comes from `MeshPlugin`, already in `DefaultPlugins`.
- `bevy`'s default `3d` feature includes `picking`, which includes
  `mesh_picking`, so the type is reachable with no manifest change.

`MeshPickingPlugin` is **not** added. It exists to run pointer-driven hover and
`Pointer<Click>` observers off `bevy_picking`'s own input backend, which needs
`bevy_winit` — disabled in this app. Only the ray-cast `SystemParam` is used.

The ray comes from whichever camera `ViewportCamera` has active, via
`Camera::viewport_to_world(gt, pos * camera.logical_viewport_size())` — the
normalized position of M7-1 needs no other conversion. The nearest hit is the
selection; a miss clears it. No ancestor walk: `MeshAsset` puts `Mesh3d` on the
node entity itself, so the hit entity *is* the node.

The cast passes a `MeshRayCastSettings::with_filter` that rejects the gizmo's
own handle meshes (M7-8), which are `Mesh3d` entities like any other and sit
directly under the cursor whenever a gizmo is up.

The hand-rolled ray-vs-AABB fallback the roadmap held in reserve is not built.
The gizmo needs ray-vs-*handle* maths, which is analytic and unrelated.

### M7-7 — The gizmo writes through; "driven axes inert" is dropped

M6-5 dropped D2's read-only rule for the inspector and left the gizmo's version
of the question open, since the gizmo was to reuse detection machinery that no
longer exists. It is now settled the same way the inspector was: **the gizmo
edits `Transform` unconditionally.**

Dragging an axis a wire drives works for exactly one tick, and the wire
overwrites it on the next — the object springs back under the cursor. That is
the same contract the inspector already has (M6-5: "editing a driven field
holds only until the next tick"), it needs no detection code, and it keeps one
rule for the whole editor rather than two.

The cost is accepted and stated plainly: on a wire-driven object the gesture
appears to do nothing. The alternative — one wire-registry lookup per TRS
channel on drag start, dimming and refusing those handles — was considered and
rejected in favour of consistency with M6-5.

This retires "driven axes render inert" from the roadmap and from
`2026-07-25-sway-design.md`, and closes architecture §7's open note.

### M7-8 — Bevy's transform gizmo is reused for everything except its input

`bevy_gizmos-0.19.0/src/transform_gizmo.rs` is a complete interactive TRS
gizmo, and `bevy_gizmos_render`'s companion renderer is **already in this app**:
`GizmoPlugin::build` adds `TransformGizmoRenderPlugin` unconditionally when
`PbrPlugin` is present, and its systems are gated on
`resource_exists::<TransformGizmoSettings>`. Reading both crates splits cleanly
along exactly the line this app cares about:

| Half | Reusable here? |
|---|---|
| `TransformGizmoSettings`, `TransformGizmoState`, `TransformGizmoFocus`, `TransformGizmoCamera`, `TransformGizmoMode` / `Space` / `Axis` | Yes — all `pub`, no window |
| `intersect_plane`, `translation_plane_normal`, `axis_direction`, `point_to_segment_dist`, `point_to_ring_screen_dist`, `gizmo_rotation`, `effective_space` | Yes — all `pub` free functions |
| `TransformGizmoRenderPlugin` — handle meshes, hover highlight, screen-constant scale, overlay camera | Yes — no `Window` dependency anywhere in it |
| `transform_gizmo_hover`, `transform_gizmo_drag` | **No** — private, and both take `Single<&Window, With<PrimaryWindow>>` plus `ButtonInput<MouseButton>` |

So M7 reuses everything but the input half — which is the half that had to be
ours regardless, because the cursor arrives over a channel rather than from a
window. `TransformGizmoPlugin` is **not** added, and no fake `Window` entity is
spawned. `EditorViewportPlugin` instead does:

- `init_resource::<TransformGizmoSettings>()` and
  `init_resource::<TransformGizmoState>()`, which is what switches the renderer
  on. It must happen at plugin-build time, because `spawn_gizmo_meshes` runs in
  `Startup` behind that run condition.
- Adds and removes `TransformGizmoFocus` as `Selection` changes, and keeps
  `TransformGizmoCamera` on whichever camera `ViewportCamera` has active —
  required, not optional, because this world has more than one camera.
- Runs two systems of our own, `viewport_gizmo_hover` and
  `viewport_gizmo_drag`, reading `ViewportEvents` and writing
  `TransformGizmoState` (`hovered_axis`, `active`, `axis`, `start_transform`,
  `drag_start_world`, `gizmo_origin`) so the renderer highlights and the drag
  resolves. They are the private systems reimplemented against the public
  helpers, with our normalized viewport coordinates in place of
  `window.cursor_position()`.

Settings are left at their defaults except `space: World` and `confine_cursor:
false` (nothing here owns a cursor to confine). Snapping stays off — `snap_value`
is private, and snapping is out of scope anyway.

**Three consequences of the renderer being mesh-based, each handled explicitly:**

1. The handles are real entities carrying `Transform` and `Mesh3d`.
   `capture_tree` walks every `Transform` entity, so it filters
   `TransformGizmoRoot` / `TransformGizmoMeshMarker` out; and the pick ray-cast
   passes a `MeshRayCastSettings::with_filter` that rejects them, or clicking a
   handle would select the handle.
2. The renderer spawns its own overlay camera (`order: 1`, render layer 15) to
   draw handles over the scene. `retarget_cameras` correctly points it at the
   viewport texture, but M7-4's `is_active` toggle must be scoped to
   `EditorCamera` and `SceneCamera` only — deactivating the overlay camera
   would delete the gizmo from the screen.
3. That overlay camera is spawned with a default `clear_color`, while its own
   source comment says it draws "without clearing the color buffer". Which is
   true depends on Bevy's per-target first-camera-clears rule, so it is on the
   verify list below; if it does clear, the fix is one `ClearColorConfig::None`
   written onto the spawned camera.

Rotation is written as a quaternion straight into `Transform` —
`RotationFrom`'s euler-degrees convention concerns the wire, not the gizmo.
Bevy's drag maths writes local `Transform` while reading `GlobalTransform`, so
parented entities are handled by the code being reused rather than by ours; the
demo document's cube is parented, which is what would have exposed a
hand-rolled version that forgot it.

The gizmo writes `Transform` directly rather than routing through
`EditorCommand::SetField`: it is already a world system holding the query it
needs, and a per-frame drag would otherwise put one reflect round trip per
pointer-move on the channel.

Rejected: adding `TransformGizmoPlugin` with a headless `Window` +
`PrimaryWindow` + `CursorOptions` entity and synthesized
`ButtonInput<MouseButton>`. It is viable — `extract_windows` requires
`RawHandleWrapper`, so a handle-less window is invisible to the render app —
and it would buy snapping and view-plane handles for free. It was rejected
because a fake window is a load-bearing lie in an app whose whole shape (M1b)
is that Bevy owns no window, and because the two systems it saves are the two
we most want to control.

Also rejected: hand-rolling the gizmo with immediate-mode `Gizmos`, which was
this spec's first draft. Reading the pinned crates made it redundant — it would
have reimplemented public, tested geometry to avoid mesh entities that two
one-line filters handle.

## The four capabilities

### Input forwarding

`Viewport` sends on its own channel the moment an event arrives; `main.rs`
creates that channel beside the `EditorCommand` one and inserts
`ViewportInputRx` under `--editor`; a `PreUpdate` system consumes. Pointer events outside the
viewport's rectangle never reach the widget at all — masonry's hit-testing is
the gate, and no rect maths is duplicated on the world side.

### Camera navigation

Alt+drag orbit, Alt+right-drag pan, scroll/pinch dolly, against `EditorCamera`'s
four numbers; a transport-bar button toggles between the editor camera and the
scene camera.

### Click-to-select

A plain primary `Down` with no gizmo handle under it casts a ray and sends
`EditorCommand::Select`. Tree, canvas and viewport show one selection because
there is one selection.

### The gizmo

Translate / rotate / scale, world space, W/E/R — Bevy's own gizmo meshes and
geometry, driven by two input systems of ours instead of its two window-coupled
ones.

## Testing

Per architecture §9 — no pixel-diff tests; rendering is verified by eye.

- **Pure maths, unit tested in `sway-runtime` with no app at all:**
  `orbit_transform` and the orbit/pan/dolly state updates. The gizmo's geometry
  is Bevy's and already tested upstream, so what gets tested here is our use of
  it: which axis a given cursor position resolves to, and that a drag along an
  axis moves the transform along that axis and no other.
- **Widget behaviour, through real masonry dispatch:** a primary `Down` inside
  the viewport queues a `Down` with viewport-local coordinates; Alt is carried
  through; a `Down` requests focus and a subsequent `W` arrives as
  `ViewportInput::Key` (the M6 focus-routing failure class, tested the way M6's
  fixes had to be); `Cancel` is forwarded so a lost drag cannot wedge the world
  side; and the viewport still appears as an `External` layer in the plan after
  the interaction changes.
- **Headless Bevy:** a cube plus a camera in a real `build_app` world — a ray
  through the cube's centre selects it, a ray past it clears the selection, a
  ray through a gizmo handle selects neither, `ViewportCamera` leaves exactly
  one *scene* camera active in both positions while leaving the gizmo's overlay
  camera alone, and an `EditorCamera` appears in neither `capture_nodes`,
  `capture_tree` nor `to_document` — nor do the gizmo's handle meshes.
- **Snapshot and sync:** `capture` reports the world's `Selection` and inspects
  it in one pass; the tree and the canvas both render selection from the
  snapshot and neither sets its own on click; and the previously-flickering
  case — a tree row whose entity has no canvas node — stays selected across
  repeated snapshots.
- **By eye, walked by the human partner:** the exit criterion below. GUI
  click/drag automation is established as unreliable in this sandbox (M6 Tasks
  8, 11, 13), so this is run live, not by an agent.

## Verify before implementing

M5 and M6 both lost time to APIs assumed rather than read. Three assumptions
here are load-bearing and each is checked against the pinned checkout in the
plan's first task of the phase that needs it:

1. **`logical_viewport_size()` for a `RenderTarget::TextureView`** — M7-1 reads
   `viewport_to_ndc` as dividing by `logical_viewport_rect()`, which for a
   manual texture view should be the texture's own size. Everything about
   picking and gizmo hit-testing is wrong by the DPI factor if this is guessed.
2. **The gizmo renderer switching on from resources alone** — that
   `init_resource::<TransformGizmoSettings>()` plus
   `init_resource::<TransformGizmoState>()`, with no `TransformGizmoPlugin`, is
   enough for `spawn_gizmo_meshes` to run and handles to appear on a focused
   entity in this headless, manual-`RenderPlugin` app.
3. **The overlay camera's clear behaviour** — whether Bevy's per-target
   first-camera-clears rule means the `order: 1` overlay camera leaves the
   scene beneath it intact, as its own comment claims (M7-8, consequence 3).
4. **`Camera::is_active` versus `retarget_cameras`** — that an inactive camera
   neither renders nor clears the shared target.

## Phasing

Each phase leaves the app working and is independently reviewable.

1. **Input path** — `ViewportInput`, the channel, `Viewport`, the presenter and
   shell plumbing. Exit: pointer events over the viewport arrive in the world
   and can be logged.
2. **Editor camera** — `EditorCamera`, `orbit_transform`, the navigation
   systems, `ViewportCamera` and the toolbar toggle. Exit: the scene can be
   orbited, panned and dollied, and the toggle shows the show's framing.
3. **Selection** — `Selection`, `EditorCommand::Select`, the snapshot field,
   `pending_selection`, deleting `sync_selection`. Exit: tree and canvas agree
   through the world; the flicker is gone.
4. **Picking** — the ray cast and its wiring into `Select`. Exit: clicking a
   cube in the viewport selects it in all three views.
5. **Gizmo** — the two resources and the focus/camera markers that turn Bevy's
   renderer on, mode keys, then `viewport_gizmo_hover` and
   `viewport_gizmo_drag`, plus the two filters that keep handle meshes out of
   the tree and out of picking. Exit: the exit criterion.
6. **Documents** — the amendments below, and the findings report.

## Out of scope for M7

- Multi-select, box-select in the viewport, and selection of anything without a
  mesh (a camera or a light is selected from the tree, as today).
- Snapping, numeric entry during a drag, and a local/world space toggle.
  `TransformGizmoSettings` exposes all three, and M7 leaves them at their
  defaults — using them would mean reimplementing the private `snap_value` and
  adding UI this milestone has no room for.
- "Frame selected", camera bookmarks, and any persistence of the editor
  camera's pose.
- Undo. Nothing in the editor has it yet, and a gizmo does not change that.
- Driven-axis detection of any kind (M7-7).
- `MeshPickingPlugin`, `bevy_picking`'s pointer backend, and hover/highlight
  states.
- Gizmos for lights and cameras, and any viewport overlay beyond the TRS gizmo
  (no grid, no origin axes).
- The M6-inherited items that are not selection: the disconnect gesture's
  press-side real-dispatch test, `FieldValue::Enum`'s missing coverage, the
  `SOCKET_RADIUS * 2.5` duplication, and the growth of `canvas.rs` /
  `snapshot.rs`. They stay open and stay recorded in M6's findings.

## What this changes in the documents

- **`2026-08-09-mvp-roadmap-design.md`** — M7's "Driven axes render inert" is
  struck per M7-7, and the open question "`MeshRayCast` outside its plugin" is
  marked resolved per M7-6.
- **`2026-07-25-sway-design.md`** — the M7 line loses "with driven axes inert",
  and the same open question is closed.
- **`docs/architecture.md`** — §7's "Whether a future gizmo (M7) follows the
  same rule is open" becomes settled: it writes through. §5's ownership table
  gains selection as world state. §8's crate layout note for `sway-runtime`
  gains the editor viewport module, and the `sway-runtime` → `sway-graph`
  dependency is recorded.

## Exit criterion

In one editor session, with no RON editing and no inspector typing: orbit the
camera to frame the demo cube, click it in the viewport and watch the scene
tree and graph canvas select it, drag it to a new position with the translate
gizmo, press E and rotate it, press R and scale it, save, quit, relaunch, open
the file, and see the cube where it was left.
