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
    /// Viewport-local logical pixels, origin at the viewport's top-left.
    Down   { button: ViewportButton, pos: Vec2, modifiers: ViewportModifiers },
    Move   { pos: Vec2, modifiers: ViewportModifiers },
    Up     { button: ViewportButton, pos: Vec2 },
    Cancel,
    /// Positive `y` dollies in. Already normalised to logical pixels by the
    /// widget, the same way `GraphCanvas` normalises its own scroll.
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

**Positions are viewport-local logical pixels.** The widget knows its own
rectangle; the world side knows the viewport texture's size in physical pixels
and the scale factor. Sending window-space coordinates would force the world
side to learn the pane layout.

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
- A queue of `ViewportInput`, drained by `EditorUi::take_viewport_input()`.

The widget holds no interaction state beyond that queue: no drag anchor, no
"is orbiting" flag. Those live on the world side, because that is where the
gesture is resolved. The widget does call `ctx.capture_pointer()` on `Down` so
a drag that leaves the rectangle keeps delivering `Move`.

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
  clamped.
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

**The one-frame lag is handled explicitly, not ignored.** A command sent during
frame *n* is applied at the start of frame *n+1* and appears in the snapshot
captured for frame *n+2*. `EditorUi` therefore holds
`pending_selection: Option<Option<Entity>>`, set when it forwards a `Select`,
and ignores the snapshot's selection until the snapshot agrees — then clears it.
Without this the clicked row would visibly revert for one frame, which is
exactly the bug being fixed.

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
`Camera::viewport_to_world`, with the viewport-local position scaled from
logical to the camera's viewport pixels. The nearest hit is the selection; a
miss clears it. No ancestor walk: `MeshAsset` puts `Mesh3d` on the node entity
itself, so the hit entity *is* the node.

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

### M7-8 — The gizmo is drawn with `Gizmos`, hit-tested analytically

`bevy_gizmos_render` is on by default (via `3d_bevy_render`), so handles are
drawn in immediate mode. Nothing is spawned: no handle entities to appear in
the scene tree, to be picked by `MeshRayCast`, to be claimed by
`sway-document`, or to be torn down.

```rust
// sway-runtime
#[derive(Resource, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode { #[default] Translate, Rotate, Scale }
```

W, E and R switch modes while the viewport holds focus. Always world space —
no local/world toggle, because the roadmap never asked for one and it doubles
the handle maths.

The gizmo draws at the selection's `GlobalTransform` translation, sized
proportionally to its distance from the active camera so it holds a constant
screen size. Hit-testing is analytic and lives in pure functions:

- **Translate and scale** — distance between the picking ray and each axis
  segment, in world units, compared against the same distance-scaled threshold
  the drawing uses.
- **Rotate** — intersect the ray with each axis's plane through the gizmo
  origin, then compare the hit's distance from the origin against the ring
  radius.

Dragging resolves in the same geometry: translate and scale project the ray
onto the axis line (translate adds the delta, scale multiplies by the ratio);
rotate measures the angle swept about the axis and pre-multiplies
`Quat::from_axis_angle`. Rotation is written as a quaternion straight into
`Transform`, so no euler bookkeeping is needed — `RotationFrom`'s euler-degrees
convention concerns the wire, not the gizmo.

**Parented entities are handled, not deferred.** The gizmo displays at
`GlobalTransform` and writes local `Transform`, converting the world-space
delta through the parent's inverse `GlobalTransform` affine. The demo document's
own cube is parented, so an unconverted version would be visibly wrong on the
first by-eye run.

The gizmo writes `Transform` directly from its own system rather than routing
through `EditorCommand::SetField`. It is already a world system holding the
query it needs, and a per-frame drag would otherwise put one reflect round trip
per pointer-move on the channel.

## The four capabilities

### Input forwarding

`Viewport` queues; `EditorUi::take_viewport_input` drains; `EditorPresenter`
pushes onto the channel inside `present`, next to where it already drains
`take_file_requests`; `PreUpdate` consumes. Pointer events outside the
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

Translate / rotate / scale, world space, W/E/R, drawn with `Gizmos`, writing
local `Transform` through the parent's inverse where one exists.

## Testing

Per architecture §9 — no pixel-diff tests; rendering is verified by eye.

- **Pure maths, unit tested in `sway-runtime` with no app at all:**
  `orbit_transform`, the orbit/pan/dolly state updates, ray-vs-axis-segment
  distance, ray-vs-plane ring hits, axis projection for a translate drag, the
  scale ratio, the swept angle for a rotate drag, and the parent-inverse delta
  conversion.
- **Widget behaviour, through real masonry dispatch:** a primary `Down` inside
  the viewport queues a `Down` with viewport-local coordinates; Alt is carried
  through; a `Down` requests focus and a subsequent `W` arrives as
  `ViewportInput::Key` (the M6 focus-routing failure class, tested the way M6's
  fixes had to be); `Cancel` is forwarded so a lost drag cannot wedge the world
  side; and the viewport still appears as an `External` layer in the plan after
  the interaction changes.
- **Headless Bevy:** a cube plus a camera in a real `build_app` world — a ray
  through the cube's centre selects it, a ray past it clears the selection,
  `ViewportCamera` leaves exactly one camera active in both positions, and an
  `EditorCamera` appears in neither `capture_nodes` nor `to_document`.
- **Snapshot and sync:** the tree and the canvas both render selection from the
  snapshot; a `Select` command's one-frame lag does not flicker
  (`pending_selection`); and the previously-flickering case — a tree row whose
  entity has no canvas node — holds.
- **By eye, walked by the human partner:** the exit criterion below. GUI
  click/drag automation is established as unreliable in this sandbox (M6 Tasks
  8, 11, 13), so this is run live, not by an agent.

## Verify before implementing

M5 and M6 both lost time to APIs assumed rather than read. Three assumptions
here are load-bearing and each is checked against the pinned checkout in the
plan's first task of the phase that needs it:

1. **`Camera::viewport_to_world`'s coordinate convention** — logical or
   physical pixels, and which corner is the origin. Everything about picking
   and gizmo hit-testing is wrong by a scale factor if this is guessed.
2. **`Gizmos` in a headless, manual-`RenderPlugin` app** — that `GizmoPlugin`
   is present in `DefaultPlugins` as configured here and that its lines reach
   the viewport texture. This is a render-path assumption in an app whose
   render path is unusual.
3. **`Camera::is_active` versus `retarget_cameras`** — that an inactive camera
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
5. **Gizmo** — modes, drawing, hit-testing, dragging, the parent-inverse write.
   Exit: the exit criterion.
6. **Documents** — the amendments below, and the findings report.

## Out of scope for M7

- Multi-select, box-select in the viewport, and selection of anything without a
  mesh (a camera or a light is selected from the tree, as today).
- Snapping, numeric entry during a drag, and a local/world space toggle.
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
