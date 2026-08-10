# Sway — M6, the editor write half

**Date:** 2026-08-10
**Status:** Design approved; implementation plan to follow
**Milestone:** M6 in [`2026-08-09-mvp-roadmap-design.md`](2026-08-09-mvp-roadmap-design.md)
**Architecture:** [`docs/architecture.md`](../../architecture.md) is the authority
on current-state design. Two amendments this milestone makes to it are recorded
under "Decisions" below.

## The deliverable

**A node is created, wired, edited, saved and reopened without leaving the
editor.**

Today the editor is a pure read path: `EditorPresenter::present` runs
`capture(world)`, pushes a `WorldSnapshot` into masonry, and calls
`app.update()`. No widget can reach the world, and no widget ever will — a
`Widget::on_pointer_event` has no `&mut World`. Everything below follows from
building the return path.

## Decisions taken for this milestone

### M6-1 — Edits travel as commands over a channel; the ECS stays the truth

The editor produces plain data and sends it. A system in `sway-graph` drains the
channel and applies it to the world.

```rust
// sway-graph
pub enum EditorCommand {
    Create     { component: &'static str, pos: Vec2 },
    Delete     { entity: Entity },
    SetField   { entity: Entity, component: &'static str, field: String, value: FieldValue },
    MoveNode   { entity: Entity, pos: Vec2 },
    Connect    { wire: &'static str, src: Entity, dst: Entity },
    Disconnect { wire: &'static str, dst: Entity },
}

#[derive(Resource)]
pub struct EditorRx(pub Receiver<EditorCommand>);

fn apply_editor_commands(world: &mut World);  // exclusive
```

`component` and `wire` are the `&'static str` keys already carried by
`ComponentEntry::name` and `WireEntry::name`, so a command names a type without
carrying one.

`FieldValue` is a small owned enum — `Float(f32)`, `Int(i64)`, `Bool(bool)`,
`Enum(String)` (the variant name), `Str(String)`, `Vec3(Vec3)` — mirroring the
`FieldKind` the inspector renders from. Deliberately not `Box<dyn Reflect>`:
the channel payload stays `Send` and plainly comparable, and the applier does
the reflect work on the world side where the type registry is in hand.

`MoveNode` carries the canvas back to the world. Node positions are currently
owned by the `NodeBox` widget once a slot exists and are lost on exit; with this
they land in `EditorPos` and therefore in the saved document.

Exclusive, because applying spawns, despawns and inserts relationship
components. Scheduled in `PreUpdate` **before** `watch::WatchSet`, so this
frame's rewires are seen by the per-wire topology watches and mark
`TopologyDirty`; the rebuild then happens in the following `FixedUpdate`
exactly as it does for a document reload. No new rebuild trigger is introduced.

`crossbeam-channel`, not `std::sync::mpsc`: its `Receiver` is `Send + Sync`,
which a Bevy resource requires. The codebase already carries this exact shape —
`main.rs` builds `MidiRx(rx)` from a crossbeam channel and drains it in a
`PreUpdate` system, and this is the same pattern with a different payload.

Rejected: handing widgets an `Rc<RefCell<World>>` (breaks Bevy's scheduling
model and makes widgets untestable), and having the presenter drain a queue and
call an apply function (works, but puts world mutation in the host's frame loop
rather than in the schedule where it can be ordered against `WatchSet`).

### M6-2 — `ComponentDocRegistry` belongs to `sway-graph`, not the document

The roadmap admits the `sway-document` extraction. Doing it exposes a
misfiling: `ComponentDocRegistry` and `register_authorable` live under
`sway-graph/src/project/` today, but "which component types are authorable and
what are they called" is a property of the ECS authoring surface. The palette
needs it, the inspector needs it, and neither depends on there being a document
at all.

So the split is:

- **stays in `sway-graph`** — `registry.rs` (`ComponentDocRegistry`,
  `ComponentEntry`, `register_authorable`), joined by `command.rs`.
- **moves to `sway-document`** — `doc.rs`, `apply.rs`, `emit.rs`, `asset.rs`,
  `diagnostics.rs`.

`sway-document` becomes one of three consumers of the registry rather than its
owner. `sway-graph` sheds `ron`, `serde` and `bevy_asset`, which architecture §5
requires of it ("must not depend on … the document format") and which is not
true today.

`sway-editor`'s dependency list does not change: `bevy_ecs`, `bevy_reflect`,
`bevy_transform`, `sway-graph`. It never learns that `ProjectDoc`, `DocId` or
RON exist.

Clean break, no compatibility re-exports. Call sites in `sway-nodes`,
`sway-app` and `sway-editor` are updated.

### M6-3 — `sway-document` claims entities; the editor never writes `DocId`

`to_document` emits only entities carrying a `DocId`, and a palette-created
entity has none — but `DocId` is a document component and M6-2 puts the document
out of the editor's reach.

Resolution: a `sway-document` system assigns a derived unique `DocId` to any
entity carrying `EditorPos` and lacking one. The stem is the name of the first
component the entity carries in `ComponentDocRegistry.entries` order — that
order is registration order, which is fixed at startup and therefore
deterministic — suffixed `.001`, `.002`, … until unique against every `DocId`
already in the world. The editor spawns a component; the document layer notices
and claims it.

`EditorPos` is the right marker because it already means "authored on the
canvas" (M6-4) and runtime-spawned entities never carry one — so
`emit.rs`'s existing guarantee that runtime-owned entities stay out of the
document survives untouched, which
`an_entity_without_a_doc_id_is_not_in_the_document` pins.

**Renaming is out of M6.** `DocId` doubles as the entity's `Name`, so a rename
is a document operation wearing an ECS costume. Hand-edit the `.ron`.

### M6-4 — The canvas draws every entity with an `EditorPos`

`capture_nodes` currently walks `graph_entities(world)` — entities appearing in
`GraphOrder`, i.e. endpoints of a wire propagation or owners of a registered
behaviour. M5's findings recorded the consequence: a camera or light is
structurally invisible on the canvas because nothing wires into or out of it.
A palette-created `SceneCamera` would appear to do nothing.

`capture_nodes` switches to every entity carrying `EditorPos`. `EditorPos`
becomes the explicit "this is a canvas node" marker; the palette inserts it for
foreign types like `DirectionalLight` and `PointLight`, which cannot carry
`#[require(EditorPos)]` because they are Bevy's own types.

### M6-5 — Driven fields are **not** made inert; D2 is dropped

Roadmap D2 and architecture §7 say the editor treats wire-driven fields as
read-only, rendering them inert. **M6 does not implement this.** Every field is
editable; `to_document` keeps dumping the whole component, driven fields
included.

The rationale D2 was built on has already been conceded elsewhere in the
roadmap: "a save bakes in the instantaneous driven value. Harmless — the first
tick after load overwrites it". The file was never the problem. All D2 bought
was inspector polish — a driven field you edit snaps back on the next tick
rather than refusing the edit.

The cost of buying it was a `Wire::FIELD` associated constant naming the reflect
path on `Target`, a matching literal in every one of the twenty-odd
`field_wire!` invocations, a `target_type_id` on `WireEntry`, and a second
rendering path in the inspector. Not worth it for that.

**Amendments required:** roadmap D2 and architecture §7's "The editor therefore
treats wire-driven fields as read-only" both become inaccurate and are updated
as part of this milestone. Architecture §10's "Out of MVP" entry for
"Restore authored value on disconnect (see §7 — wire-driven fields are read-only
in the editor instead)" loses its justification and is restated.

**Inherited by M7:** the gizmo was to refuse driven axes under the same rule. It
now has no detection machinery to build on and must decide for itself.

### M6-6 — Sockets gain identity from data the registry already holds

No new registry fields. `WireEntry` already carries `name`, `has_source`,
`has_target`, `read`, `insert` and `remove`, and that is the whole requirement.

**Inlets — one per registered wire type `W` where `has_target(world, e)`.** The
count is already correct today: a `Transform`-carrying cube has four inlets
(`TranslationFrom`, `RotationFrom`, `ScaleFrom`, `ChildOf`) and those are four
genuinely distinct sockets. What is missing is the label (`WireEntry::name`)
and a stable socket **ordinal** — the wire's position in the filtered list,
taken from `WireRegistry` iteration order, which is registration order and
therefore fixed at startup.

The real defect is elsewhere: `capture_edges` hardcodes `to_field: 0`, so every
inbound edge is drawn into the top inlet socket whatever wire it is — a `Vec3`
wired into `scale` renders as a line into `translation`. Fixed by the ordinal
lookup above.

**Outlets — one, or none.**

```rust
let outlets = registry.entries.iter().any(|e| (e.has_source)(world, entity)) as u16;
```

`count()` becomes `any()`. Today the count is one per *wire* that could read
the entity, so an `Lfo` draws seven outlet dots (`FloatOut` sources
`AmplitudeFrom`, the three `Vec3*From`, both `Math*From`, and `RemapInputFrom`)
and a `Vec3` draws three. Since `capture_edges` also hardcodes `from_field: 0`,
sockets 1..n have never had an edge attached and are pure noise; collapsing
them makes the existing edge rendering correct rather than accidentally so.

This is also what architecture §2 means — "outlets are components". Per entity
that resolves to one: `Lfo` → `FloatOut`, `Vec3` → `Vec3Out`, cube →
`Transform` (it can be a parent, through `ChildOf`). No node in the current set
carries two distinct source component types.

**Legality needs no source type.** A drag from `A` may land on `B`'s inlet for
wire `W` iff `(W.has_source)(world, A) && (W.has_target)(world, B)`.
`has_source` resolves `A.contains::<W::Source>()` internally, so the editor
holds an `Entity` and asks each wire entry; it never names the type.
`inserting_over_an_existing_wire_replaces_its_source` already pins that rewire
needs no prior disconnect.

**Accepted limit.** A hand-authored entity carrying both `Transform` and
`FloatOut` would offer parenting inlets and float inlets from one dot —
visually coarser, behaviourally identical, since each inlet accepts exactly one
wire type and an illegal drop is refused. If that ever needs splitting, a
`source_type_id` derived from `TypeId::of::<W::Source>()` inside `register_wire`
is one line, added then.

Drag legality then falls out of data already present: a drag from `A`'s outlet
may land on `B`'s inlet for wire `W` iff `has_source(world, A)` and
`has_target(world, B)`. `WireEntry` already carries `insert`, `remove`, `read`,
`has_source` and `has_target`, and `inserting_over_an_existing_wire_replaces_its_source`
already pins that rewire needs no disconnect first.

### M6-7 — The `RenderRootSignal` sink stops being a no-op

`EditorUi::new` passes `|_signal: RenderRootSignal| {}`, documented as an
accepted M1b simplification. It is fatal to a writing editor:

- `RenderRootSignal::Action(ErasedAction, WidgetId)` is how a widget's action
  reaches the host when no ancestor consumes it.
- `NewLayer(LayerType, NewWidget<dyn Widget>, Point)` and `RemoveLayer(WidgetId)`
  are how popups exist at all. `ctx.create_layer` only *emits* the signal;
  masonry expects the host to call back into `RenderRoot`. `masonry_winit` does
  exactly this (`event_loop_runner.rs`, `RenderRootSignal::RemoveLayer(root_id)
  => window.render_root.remove_layer(root_id)`).

Verified against the pinned checkout (`xilem @ c5950bc`): with a no-op sink, no
popup, tooltip or `Selector` dropdown can appear.

The sink becomes an `Rc<RefCell<Vec<RenderRootSignal>>>` drained after each
dispatch. `NewLayer`/`RemoveLayer` are serviced against the `RenderRoot`;
`SetCursor` forwards to the shell (`Window::set_cursor`), which drag-to-connect
wants for feedback.

### M6-8 — File I/O bypasses the `AssetServer`

Masonry supplies no file dialog. Confirmed by search of the pinned checkout: no
`rfd`, no `FileDialog`, no `NSOpenPanel` in masonry, masonry_core,
masonry_winit or xilem; `RenderRootSignal` has no file-picking variant; there is
no file-open example. The host must supply it, as it must supply menus.

`sway-app` gains `rfd`, used in its **async** form — `AsyncFileDialog` returns a
future the shell polls from its redraw loop. The blocking form spins a nested
`NSApplication` modal on the main thread, which is where the winit event loop
already lives; the async form avoids that interaction entirely and keeps
rendering alive behind the dialog.

File operations never touch the world, so they are a second, separate channel:

```rust
// sway-editor
pub enum FileCommand { Open(PathBuf), Save, SaveAs(PathBuf) }
```

read by the presenter, since only `sway-app` owns `rfd` and the filesystem. It
lives in `sway-editor` rather than `sway-document` because it is a UI intent,
not a document operation — the editor asks for a file to be opened without
knowing what parsing one means. `Open(PathBuf)` and `SaveAs(PathBuf)` carry the
path the shell's dialog already resolved.

**Open and Save use `std::fs`, not the `AssetServer`.** Asset paths resolve
against the `assets/` root, so an `rfd`-picked absolute path cannot round-trip
through it. `sway-document` gains `CurrentDocument { path: Option<PathBuf> }`.
Open parses and applies directly and clears `ProjectHandle`, so the file watcher
stops mattering. Save is `to_ron(&to_document(world))` written to
`CurrentDocument.path`, falling back to Save As when it is `None`.

**Self-triggered reload** is suppressed by keeping the last applied `ProjectDoc`
in a resource and skipping an incoming document equal to it. `ProjectDoc` is
already `PartialEq` and `document_to_world_to_document_is_stable` already pins
that equality holds across a round trip, so this is a comparison, not new
machinery.

## The four capabilities

### Palette

Right-click on the graph canvas emits `NewLayer` with a filterable list built
from `ComponentDocRegistry.entries`. Picking a type sends
`Create { component, pos }` with the pointer's canvas-space position.

`apply_editor_commands` spawns an entity, inserts the component by `TypeId`
through `ReflectComponent::insert` with its `ReflectDefault` value (both are
guaranteed present — `register_authorable` panics at startup without them), and
inserts `EditorPos`. Bevy's `#[require]` supplies the companions.

This is the flow M5's `apply` fix already protects: `apply_components`'s removal
pass exempts the transitive `required_components()` of every component the
document names, so a created `Lfo` keeps its `FloatOut` across the first
save/reload rather than silently losing its outlet.

### Delete

`Delete { entity }` reparents children to the deleted entity's own parent — or
to root when it has none — **before** despawning, because Bevy's despawn
cascades through `Children`. Consumer-side wires drop with the consumer
automatically.

### Inspector editing

`InspectorComponent.fields` grows from `Vec<(String, String)>` to a struct
carrying the field's name, its formatted value, and a `FieldKind`:
`Float` / `Int` / `Bool` / `Enum(Vec<String>)` / `String` / `Vec3`.

One widget per kind: text input committing on Enter and on blur, `Checkbox`,
`Selector` (which is why M6-7 is a prerequisite), three numeric inputs for a
`Vec3`. Anything the walk cannot classify keeps today's read-only `Label`
rendering, which remains the signal that a type wants editor `TypeData`.

Commit sends `SetField`, applied through `ReflectComponent::reflect_mut` and the
field path.

### Drag-to-connect

Press on an outlet socket starts an edge drag. While dragging, every inlet
satisfying M6-6's legality predicate highlights and every other socket is inert.
Release on a legal inlet sends `Connect`; release elsewhere cancels. Clicking a
connected inlet socket sends `Disconnect`.

## Testing

Per architecture §9. No pixel tests.

- **Commands** (`sway-graph`, headless) — create inserts the component and its
  `#[require]` companions and an `EditorPos`; delete reparents children before
  despawning and leaves no dangling wire; set-field writes through reflect for
  each `FieldKind`; connect/disconnect/rewire go through `WireEntry`.
- **Ordering** — a `Connect` applied in `PreUpdate` leaves `TopologyDirty` set
  and the next `FixedUpdate` rebuild contains the new edge.
- **Claiming** (`sway-document`) — an entity with `EditorPos` and no `DocId`
  gets a unique one; a runtime entity without `EditorPos` does not; ids do not
  collide with ones the document already names.
- **Round trip** — create, connect and edit through commands, then
  `to_document` → `to_ron` → `parse` → `apply` reproduces the same world.
- **Reload suppression** — applying a document equal to the last applied one is
  skipped; an actually-changed one is not.
- **Widgets** (`masonry_testing`) — palette filtering; the widget kind chosen
  per `FieldKind`; a commit emits exactly one `SetField`; drag legality
  highlights only legal inlets.
- **Sockets** — an entity sourcing several wire types reports exactly one
  outlet; an inbound edge is drawn at its own wire's inlet ordinal, not at
  socket 0 (the `to_field` defect M6-6 names).
- **By eye** — the exit criterion itself, run through
  `cargo run -p sway-app -- --editor --windowed`.

## Verify before implementing

1. **Does Bevy clean up wires where the *deleted* entity was the source?**
   Consumer-side cleanup is pinned by existing characterization tests; the
   source side is not. If it does not, `Delete` must walk the
   `RelationshipTarget` collections first. Characterize before relying on
   either answer.
2. **`RenderRoot`'s layer API surface.** `RenderRootSignal::NewLayer` and
   `remove_layer` are confirmed present; the exact `RenderRoot` method the host
   calls to *add* a layer, and the coordinate space its `Point` is in, are read
   off `masonry_winit::event_loop_runner` rather than assumed.
3. **`rfd`'s `AsyncFileDialog` under a borrowed winit event loop.** The future
   must be pollable from the shell's redraw without an executor; confirm before
   building the Open path on it.
4. **`ReflectComponent::insert` needs a `&AppTypeRegistry`** held across the
   world mutation. Confirm the borrow works from an exclusive system, since the
   registry is itself a resource.

## Phasing

Each phase leaves the workspace green and is reviewable on its own.

1. **`sway-document` extraction.** Pure move plus M6-2's registry split. No new
   behaviour; the existing test suite is the acceptance criterion.
2. **The write path, headless.** Command channel, `apply_editor_commands`,
   snapshot extensions (M6-4, M6-6, `FieldKind`). No registry changes.
   No UI.
3. **Signal sink + inspector editing.** First visible capability: edit a value,
   watch the scene change.
4. **File I/O + `DocId` claiming.** Edit and save, end to end.
5. **Palette, create, delete.**
6. **Drag-to-connect and disconnect.** Exit criterion met.

## Out of scope for M6

- Renaming a node (M6-3).
- Making driven fields inert (M6-5) — and with it, roadmap D2.
- Undo/redo. Not in the roadmap, and not implied by the exit criterion.
- A dirty-state marker or save-before-quit prompt.
- Comment and ordering preservation on save — the roadmap already records both
  as deliberately not wanted.
- Multi-select and box-select edits; the canvas's existing box-select stays a
  selection tool only.
- Everything M7 owns: viewport pointer forwarding, the editor camera,
  click-to-select, the TRS gizmo.
