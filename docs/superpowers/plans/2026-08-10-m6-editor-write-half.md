# M6 — Editor Write Half Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A node is created, wired, edited, saved and reopened without leaving the editor.

**Architecture:** The editor stays a pure data producer. Widgets send an `EditorCommand` over a crossbeam channel; an exclusive `PreUpdate` system in `sway-graph` drains it and mutates the world, ordered before the topology watches so rewires reach the next `FixedUpdate` rebuild. The document format extracts into `sway-document`, but `ComponentDocRegistry` stays in `sway-graph` — so `sway-editor` never links the document model.

**Tech Stack:** Rust 2024, Bevy 0.19 (pinned `=0.19.0`), masonry (git rev `c5950bcb03d4f3d187a20d1159f6aa276fd056bf`), crossbeam-channel, ron 0.12, rfd (new).

**Spec:** [`docs/superpowers/specs/2026-08-10-m6-editor-write-half-design.md`](../specs/2026-08-10-m6-editor-write-half-design.md)

## Global Constraints

- **Never write an equal value.** Any wire or command that writes a component field must compare first (`set_if_neq`, or `reflect_partial_eq` before `insert`). `get_mut` marks `Changed` unconditionally and `Changed<T>` is the whole dirty story downstream (architecture §7).
- **`sway-graph` must not depend on `bevy_render`, MIDI types, or the document format.** After Task 1 it must not depend on `ron`, `serde` or `bevy_asset` either.
- **`sway-editor` must not depend on** `bevy` (the facade), `bevy_render`, `wgpu`, `vello`, `imaging_vello`, or `sway-document`. Its manifest is where that is enforced.
- **Bevy dependency versions are exact-pinned** (`=0.19.0`). Do not relax a pin. Do not add a dependency that pulls a second `wgpu` or `winit` version — duplicate detection is a build failure by design.
- **Panics are allowed at startup only** (registration-time asserts). Runtime failures go to a diagnostics resource, never `unwrap`.
- **Tests:** no pixel-diff tests (architecture §9). Rendering is verified by eye.
- **Commit after every task.** Run `cargo test --workspace` before each commit.
- **A fixed-timestep app needs two `app.update()` calls** before `FixedUpdate` runs — frame 0 only primes the accumulator. This bites every scheduling test; see `crates/sway-graph/src/watch.rs` `watched_app`.

---

## File Structure

**New crate `crates/sway-document/`** — the on-disk format only.

| File | Responsibility |
|---|---|
| `src/lib.rs` | Re-exports; `ProjectPlugin`. |
| `src/doc.rs` | `ProjectDoc`, `EntityDoc`, `parse`. Moved verbatim. |
| `src/diagnostics.rs` | `DocId`, `ItemError`, `ProjectDiagnostics`. Moved verbatim. |
| `src/apply.rs` | Document → world. Moved verbatim. |
| `src/emit.rs` | World → document. Moved verbatim. |
| `src/asset.rs` | `ProjectAsset`, loader, hot reload. Moved, then extended (Task 10). |
| `src/claim.rs` | **New.** Assigns `DocId` to `EditorPos` entities that lack one. |
| `src/file.rs` | **New.** `CurrentDocument`, `save_to_path`, `open_from_path`. |

**`crates/sway-graph/`** — gains the command path, loses the document.

| File | Responsibility |
|---|---|
| `src/registry_components.rs` | **Moved** from `src/project/registry.rs`, unchanged content. |
| `src/command.rs` | **New.** `EditorCommand`, `FieldValue`, `EditorRx`, `apply_editor_commands`. |
| `src/project/` | **Deleted.** |

**`crates/sway-editor/`** — gains the write path and real widgets.

| File | Responsibility |
|---|---|
| `src/snapshot.rs` | Extended: `EditorPos` node population, `InletView`, `FieldKind`, `palette`. |
| `src/lib.rs` | Extended: signal sink, command sender, `FileRequest`. |
| `src/inspector.rs` | Rewritten: typed editable field rows. |
| `src/palette.rs` | **New.** The filterable component list, a masonry `Layer`. |
| `src/canvas.rs` | Extended: right-click palette, socket hit-testing, edge drag, legality overlay. |
| `src/node_box.rs` | Extended: socket press hit-test, `DragEnded`, connect gestures. |
| `src/transport_bar.rs` | Extended: Open / Save / Save As buttons and their outbox. |
| `src/test_graph.rs` | Extended: fixtures carry `EditorPos`; two multi-wire fixtures. |

**`crates/sway-app/`** — hosts the command channel and the file dialogs.

| File | Responsibility |
|---|---|
| `src/presenter.rs` | Forwards `FileRequest`s and cursor requests. |
| `src/shell.rs` | Owns the `rfd` future and calls `sway_document`'s open/save. |
| `src/main.rs` | Builds the editor command channel; adds `sway_document::ProjectPlugin`. |

---

## Phase 1 — The extraction

### Task 1: Extract `sway-document`; keep the component registry in `sway-graph`

Pure move. No behaviour changes. The existing suite is the acceptance criterion.

**Files:**
- Create: `crates/sway-document/Cargo.toml`, `crates/sway-document/src/lib.rs`
- Move: `crates/sway-graph/src/project/{doc,apply,emit,asset,diagnostics}.rs` → `crates/sway-document/src/`
- Move: `crates/sway-graph/src/project/registry.rs` → `crates/sway-graph/src/registry_components.rs`
- Delete: `crates/sway-graph/src/project/mod.rs`
- Modify: `Cargo.toml` (workspace members + deps), `crates/sway-graph/Cargo.toml`, `crates/sway-graph/src/lib.rs`, `crates/sway-app/Cargo.toml`, `crates/sway-app/src/main.rs`, `crates/sway-app/tests/demo_document.rs`, `crates/sway-app/tests/demo_renders.rs`

**Interfaces:**
- Produces: crate `sway_document` exporting `ProjectDoc`, `EntityDoc`, `FORMAT_VERSION`, `ParseError`, `parse`, `apply`, `to_document`, `to_ron`, `DocId`, `ItemError`, `ProjectDiagnostics`, `ProjectAsset`, `ProjectHandle`, `ProjectLoader`, `ProjectPlugin`.
- Produces: `sway_graph::{ComponentDocRegistry, ComponentEntry, register_authorable}` (unchanged paths — `sway-nodes` and `sway-editor` need no edits).

- [ ] **Step 1: Create the crate manifest**

`crates/sway-document/Cargo.toml`:

```toml
[package]
name = "sway-document"
version.workspace = true
edition.workspace = true

# The on-disk format only. Depends on sway-graph for the registries it reads
# (ComponentDocRegistry, WireRegistry) — never the other way round.
[dependencies]
bevy_app.workspace = true
bevy_asset.workspace = true
bevy_ecs.workspace = true
bevy_math.workspace = true
bevy_reflect.workspace = true
sway-graph.workspace = true
ron.workspace = true
serde.workspace = true
```

- [ ] **Step 2: Register the crate in the workspace**

In the root `Cargo.toml`, add `"crates/sway-document"` to `members`, and under `[workspace.dependencies]`:

```toml
sway-document = { path = "crates/sway-document" }
```

- [ ] **Step 3: Move the five document modules**

```bash
git mv crates/sway-graph/src/project/doc.rs         crates/sway-document/src/doc.rs
git mv crates/sway-graph/src/project/diagnostics.rs crates/sway-document/src/diagnostics.rs
git mv crates/sway-graph/src/project/apply.rs       crates/sway-document/src/apply.rs
git mv crates/sway-graph/src/project/emit.rs        crates/sway-document/src/emit.rs
git mv crates/sway-graph/src/project/asset.rs       crates/sway-document/src/asset.rs
git mv crates/sway-graph/src/project/registry.rs    crates/sway-graph/src/registry_components.rs
git rm crates/sway-graph/src/project/mod.rs
```

- [ ] **Step 4: Write `sway-document`'s lib.rs**

`crates/sway-document/src/lib.rs`:

```rust
//! The project document: reading it, applying it, writing it.
//! Spec: docs/superpowers/specs/2026-08-06-project-format-design.md
//!
//! Extracted from `sway-graph` in M6 (spec M6-2). The component registry
//! deliberately did *not* come with it: which component types are authorable
//! is a property of the ECS authoring surface, and the palette and inspector
//! both read it without any document existing.

pub mod apply;
pub mod asset;
pub mod diagnostics;
pub mod doc;
pub mod emit;

pub use apply::apply;
pub use asset::{ProjectAsset, ProjectHandle, ProjectLoader, ProjectPlugin};
pub use diagnostics::{DocId, ItemError, ProjectDiagnostics};
pub use doc::{EntityDoc, FORMAT_VERSION, ParseError, ProjectDoc, parse};
pub use emit::{to_document, to_ron};
```

- [ ] **Step 5: Rewrite the moved files' internal `use` paths**

In each moved file, `crate::project::X` becomes `crate::X`, and references to
things still in `sway-graph` become `sway_graph::…`. Concretely:

- `apply.rs`: `use crate::order::TopologyDirty` → `use sway_graph::TopologyDirty`;
  `use crate::project::diagnostics::…` → `use crate::diagnostics::…`;
  `use crate::project::doc::…` → `use crate::doc::…`;
  `use crate::project::registry::ComponentDocRegistry` → `use sway_graph::ComponentDocRegistry`;
  `use crate::registry_wires::WireRegistry` → `use sway_graph::WireRegistry`.
- `emit.rs`: `use crate::project::diagnostics::DocId` → `use crate::diagnostics::DocId`;
  `use crate::project::doc::…` → `use crate::doc::…`;
  `use crate::project::registry::ComponentDocRegistry` → `use sway_graph::ComponentDocRegistry`;
  `use crate::registry_wires::WireRegistry` → `use sway_graph::WireRegistry`.
- `asset.rs`: `use crate::project::apply::apply` → `use crate::apply::apply`;
  `use crate::project::diagnostics::ProjectDiagnostics` → `use crate::diagnostics::ProjectDiagnostics`;
  `use crate::project::doc::…` → `use crate::doc::…`.
- `emit.rs`'s test module: `use crate::order::TopologyDirty` → `use sway_graph::TopologyDirty`;
  `use crate::project::apply::apply` → `use crate::apply::apply`;
  `use crate::project::registry::register_authorable` → `use sway_graph::register_authorable`;
  `use crate::registry_wires::register_wire` → `use sway_graph::register_wire`;
  `use crate::test_wires::{FloatOut, Gain, GainFrom}` — **see Step 6.**
- `registry_components.rs`: no path edits needed; it referenced nothing under `project`.

- [ ] **Step 6: Move the shared test fixtures out of `#[cfg(test)]`**

`emit.rs` and `apply.rs`'s tests use `crate::test_wires::{FloatOut, Gain, GainFrom}`,
which is `#[cfg(test)] pub(crate)` in `sway-graph` and therefore invisible to
`sway-document`. Expose it behind a feature so both crates share one fixture.

In `crates/sway-graph/Cargo.toml`:

```toml
[features]
test-wires = []
```

In `crates/sway-graph/src/lib.rs`, replace the `#[cfg(test)] pub(crate) mod test_wires;` line with:

```rust
#[cfg(any(test, feature = "test-wires"))]
pub mod test_wires;
```

In `crates/sway-document/Cargo.toml`:

```toml
[dev-dependencies]
bevy_app.workspace = true
sway-graph = { workspace = true, features = ["test-wires"] }
```

Then in the moved tests, `use crate::test_wires::…` becomes `use sway_graph::test_wires::…`.

- [ ] **Step 7: Update `sway-graph`'s lib.rs and manifest**

`crates/sway-graph/src/lib.rs` — replace the `project` module line and its re-export:

```rust
pub mod registry_components;
```

```rust
pub use registry_components::{ComponentDocRegistry, ComponentEntry, register_authorable};
```

Delete `pub mod project;` and the `pub use project::{…}` line.

In `crates/sway-graph/Cargo.toml`, delete the `bevy_asset`, `ron` and `serde`
dependencies and update the header comment — the constraint it documents is now
actually true:

```toml
# Spec §2: bevy_app/ecs/math/reflect/time/transform only. NOT the `bevy`
# facade, NOT bevy_render, NOT the document format (M6-2 moved it to
# sway-document, taking bevy_asset/ron/serde with it). This manifest is the
# only place that constraint is enforced.
```

- [ ] **Step 8: Update `sway-app`**

Add `sway-document.workspace = true` to `crates/sway-app/Cargo.toml`.

In `src/main.rs`: `sway_graph::ProjectHandle` → `sway_document::ProjectHandle`,
and `sway_graph::ProjectPlugin` → `sway_document::ProjectPlugin`.

In `tests/demo_document.rs` and `tests/demo_renders.rs`: every
`sway_graph::project::X` → `sway_document::X`, and
`use sway_graph::project::{DocId, to_document}` → `use sway_document::{DocId, to_document}`.

- [ ] **Step 9: Verify the dependency constraint actually holds**

Run: `cargo tree -p sway-graph --depth 1`
Expected: no `ron`, no `serde`, no `bevy_asset` in the output.

- [ ] **Step 10: Run the full suite**

Run: `cargo test --workspace`
Expected: PASS, same count as before the move (269 passed, 1 ignored doctest).
A pure move changes no test outcomes; any change in count is a mistake in this task.

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "refactor: extract sway-document; component registry stays in sway-graph

The document format moves out of sway-graph per architecture §5, taking
bevy_asset, ron and serde with it. ComponentDocRegistry deliberately stays:
which component types are authorable is a property of the ECS authoring
surface, read by the palette and inspector without any document existing.

sway-nodes and sway-editor need no changes as a result."
```

---

## Phase 2 — The write path, headless

### Task 2: The command channel and its ordering

**Files:**
- Create: `crates/sway-graph/src/command.rs`
- Modify: `crates/sway-graph/src/lib.rs`, `crates/sway-graph/src/run.rs`, `crates/sway-graph/Cargo.toml`

**Interfaces:**
- Produces: `sway_graph::{EditorCommand, FieldValue, EditorRx, apply_editor_command, apply_editor_commands}`.
- Consumes: `sway_graph::{TopologyDirty, WatchSet, Authoring}` from Task 1's tree.

- [ ] **Step 1: Add the dependency**

In `crates/sway-graph/Cargo.toml`, add `crossbeam-channel.workspace = true`.

- [ ] **Step 2: Write the failing test**

`crates/sway-graph/src/command.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::EditorPos;
    use crate::run::WiresPlugin;
    use crate::watch::Authoring;
    use bevy_app::App;
    use bevy_math::Vec2;
    use bevy_time::{Fixed, Time};

    fn command_app() -> (App, crossbeam_channel::Sender<EditorCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(Authoring)
            .insert_resource(EditorRx(rx))
            .add_plugins(WiresPlugin);
        // Two updates: frame 0 only primes the fixed-time accumulator.
        app.update();
        app.update();
        (app, tx)
    }

    #[test]
    fn a_move_node_command_writes_editor_pos() {
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::new(40.0, 90.0) })
            .expect("the receiver is alive in the world");
        app.update();

        assert_eq!(
            app.world().get::<EditorPos>(entity).map(|p| p.0),
            Some(Vec2::new(40.0, 90.0)),
        );
    }

    #[test]
    fn an_unchanged_position_does_not_mark_the_component_changed() {
        // Global constraint: never write an equal value.
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::new(7.0, 7.0))).id();
        app.update();

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::new(7.0, 7.0) }).unwrap();
        app.update();

        assert!(!app.world().entity(entity).get_ref::<EditorPos>().unwrap().is_changed());
    }

    #[test]
    fn a_command_naming_a_dead_entity_is_ignored_not_a_panic() {
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        app.world_mut().despawn(entity);

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::ONE }).unwrap();
        app.update();
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p sway-graph command::`
Expected: FAIL — `command` module does not exist.

- [ ] **Step 4: Write the module**

At the top of `crates/sway-graph/src/command.rs`:

```rust
//! The editor's write path. Spec M6-1.
//!
//! The editor produces plain data and sends it; this drains the channel and
//! mutates the world. `sway-editor` never sees a `World`, and nothing here
//! knows the document format exists.

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use crossbeam_channel::Receiver;

use crate::ctx::EditorPos;

/// One edited field's new value. Deliberately not `Box<dyn Reflect>`: the
/// channel payload stays `Send` and plainly comparable, and the applier does
/// the reflect work on the world side where the type registry is in hand.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Float(f32),
    Int(i64),
    Bool(bool),
    /// A unit enum variant, by name.
    Enum(String),
    Str(String),
    Vec3(Vec3),
}

/// One edit, from the editor to the world.
///
/// `component` and `wire` are the `&'static str` keys already carried by
/// `ComponentEntry::name` and `WireEntry::name`, so a command names a type
/// without carrying one.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    Create { component: &'static str, pos: Vec2 },
    Delete { entity: Entity },
    SetField { entity: Entity, component: &'static str, field: String, value: FieldValue },
    MoveNode { entity: Entity, pos: Vec2 },
    Connect { wire: &'static str, src: Entity, dst: Entity },
    Disconnect { wire: &'static str, dst: Entity },
}

/// The receiving half, held by the world. Present only in an editor build.
#[derive(Resource)]
pub struct EditorRx(pub Receiver<EditorCommand>);

/// Drains every queued command. Exclusive, because applying spawns, despawns
/// and inserts relationship components.
///
/// Scheduled in `PreUpdate` **before** `WatchSet`, so this frame's rewires are
/// seen by the per-wire topology watches and mark `TopologyDirty`; the rebuild
/// then happens in the following `FixedUpdate` exactly as it does for a
/// document reload.
pub fn apply_editor_commands(world: &mut World) {
    let Some(rx) = world.get_resource::<EditorRx>() else {
        return;
    };
    let commands: Vec<EditorCommand> = rx.0.try_iter().collect();
    for command in &commands {
        apply_editor_command(world, command);
    }
}

/// One command. Split out from [`apply_editor_commands`] so tests can drive it
/// directly without a channel.
pub fn apply_editor_command(world: &mut World, command: &EditorCommand) {
    match command {
        EditorCommand::MoveNode { entity, pos } => {
            let Ok(mut entity_mut) = world.get_entity_mut(*entity) else {
                return;
            };
            let Some(mut editor_pos) = entity_mut.get_mut::<EditorPos>() else {
                return;
            };
            // Never write an equal value (architecture §7).
            if editor_pos.0 != *pos {
                editor_pos.0 = *pos;
            }
        }
        // Tasks 3-5 fill these in.
        EditorCommand::Create { .. }
        | EditorCommand::Delete { .. }
        | EditorCommand::SetField { .. }
        | EditorCommand::Connect { .. }
        | EditorCommand::Disconnect { .. } => {}
    }
}
```

`get_mut` on `EditorPos` marks it `Changed` unconditionally, so the guard above
must compare *before* taking the mutable borrow's write. `Mut::set_if_neq` is
the idiomatic form; here the field write is guarded directly because
`EditorPos` wraps a `Vec2` rather than being one.

Replace the two `bevy_ecs::change_detection` lines with `set_if_neq` if you
prefer — either satisfies the test.

- [ ] **Step 5: Wire the module and the schedule**

In `crates/sway-graph/src/lib.rs`:

```rust
pub mod command;
```

```rust
pub use command::{EditorCommand, EditorRx, FieldValue, apply_editor_command, apply_editor_commands};
```

In `crates/sway-graph/src/run.rs`, inside `WiresPlugin::build`, after the
`configure_sets` call:

```rust
app.add_systems(
    bevy_app::PreUpdate,
    crate::command::apply_editor_commands
        .before(crate::watch::WatchSet)
        .run_if(bevy_ecs::schedule::common_conditions::resource_exists::<
            crate::command::EditorRx,
        >),
);
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-graph command::`
Expected: PASS (3 tests).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(graph): the editor command channel

Commands arrive over a crossbeam channel and are applied by an exclusive
PreUpdate system ordered before WatchSet, so a rewire reaches the next
FixedUpdate rebuild. MoveNode is the first variant; the rest follow."
```

---

### Task 3: `Create` and `Delete`

**Files:**
- Modify: `crates/sway-graph/src/command.rs`
- Create: `crates/sway-graph/tests/despawn_semantics.rs`

**Interfaces:**
- Consumes: `EditorCommand`, `apply_editor_command` from Task 2.
- Produces: `Create` spawning an entity with the named component, its `#[require]` companions and an `EditorPos`; `Delete` reparenting children before despawn.

- [ ] **Step 1: Characterize what Bevy does to wires whose SOURCE despawns**

Spec "Verify before implementing" item 1. Consumer-side cleanup is already
pinned by `crates/sway-graph/tests/relationship_semantics.rs`; the source side
is not, and `Delete` depends on the answer.

`crates/sway-graph/tests/despawn_semantics.rs`:

```rust
//! What Bevy does to a relationship when the *producer* despawns.
//!
//! `relationship_semantics.rs` pins the consumer side. M6's `Delete` command
//! needs the other direction: if a wire component survives on a consumer whose
//! producer is gone, `Delete` must clear it by hand.

use bevy_ecs::world::World;
use sway_graph::test_wires::{GainFrom, spawn_float, spawn_gain};

#[test]
fn despawning_a_producer_removes_the_wire_from_its_consumers() {
    let mut world = World::new();
    let src = spawn_float(&mut world, 1.0);
    let dst = spawn_gain(&mut world, 0.0);
    world.entity_mut(dst).insert(GainFrom(src));

    world.despawn(src);

    assert!(
        world.get::<GainFrom>(dst).is_none(),
        "if this fails, EditorCommand::Delete must walk the producer's \
         RelationshipTarget and remove each consumer's wire component itself"
    );
    assert!(world.get_entity(dst).is_ok(), "the consumer itself must survive");
}
```

Add `sway-graph = { workspace = true, features = ["test-wires"] }` to
`crates/sway-graph`'s own `[dev-dependencies]` — a crate can depend on itself
with a feature enabled for integration tests. If that proves awkward, move this
test into `src/command.rs`'s `#[cfg(test)]` module instead, where `test_wires`
is directly visible.

- [ ] **Step 2: Run it and record the answer**

Run: `cargo test -p sway-graph --test despawn_semantics`

**If it PASSES:** Bevy cleans up both directions, and Step 5's `Delete` needs no
extra work. **If it FAILS:** invert the assertion to document the real
behaviour, and add a wire-clearing loop to `Delete` in Step 5 that walks
`WireRegistry` and calls `(entry.remove)(world, consumer)` for every consumer
naming the deleted entity. Either way the test is committed — it pins the
behaviour `Delete` relies on.

- [ ] **Step 3: Write the failing tests for Create and Delete**

Add to `crates/sway-graph/src/command.rs`'s test module:

```rust
    use bevy_ecs::component::Component;
    use bevy_ecs::hierarchy::{ChildOf, Children};
    use bevy_reflect::Reflect;
    use bevy_reflect::std_traits::ReflectDefault;
    use bevy_ecs::reflect::ReflectComponent;

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Blip(f32);

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    #[require(Blip, EditorPos)]
    struct Widget { size: f32 }

    fn registry_app() -> App {
        let (_, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(Authoring)
            .insert_resource(EditorRx(rx))
            .add_plugins(WiresPlugin);
        crate::register_authorable::<Widget>(&mut app, "Widget");
        crate::register_authorable::<Blip>(&mut app, "Blip");
        crate::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn create_spawns_the_component_its_requires_and_an_editor_pos() {
        let mut app = registry_app();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create { component: "Widget", pos: Vec2::new(12.0, 34.0) },
        );

        let entity = app
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<Widget>>()
            .single(app.world())
            .expect("exactly one Widget was created");
        assert!(app.world().get::<Blip>(entity).is_some(), "#[require] supplied Blip");
        assert_eq!(
            app.world().get::<EditorPos>(entity).map(|p| p.0),
            Some(Vec2::new(12.0, 34.0)),
            "the palette's click position becomes the canvas position",
        );
    }

    #[test]
    fn create_uses_the_components_reflect_default() {
        let mut app = registry_app();
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create { component: "Widget", pos: Vec2::ZERO },
        );
        let entity = app
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<Widget>>()
            .single(app.world())
            .unwrap();
        assert_eq!(app.world().get::<Widget>(entity), Some(&Widget::default()));
    }

    #[test]
    fn create_with_an_unregistered_name_does_nothing() {
        let mut app = registry_app();
        let before = app.world().entities().len();
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create { component: "Nonexistent", pos: Vec2::ZERO },
        );
        assert_eq!(app.world().entities().len(), before);
    }

    #[test]
    fn delete_reparents_children_to_the_grandparent_before_despawning() {
        // Bevy's despawn cascades through Children, so a child would be
        // destroyed with its parent unless it is moved first.
        let mut app = registry_app();
        let grandparent = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let parent = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(grandparent))).id();
        let child = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(parent))).id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(app.world().get_entity(parent).is_err(), "the target despawned");
        assert!(app.world().get_entity(child).is_ok(), "the child survived");
        assert_eq!(
            app.world().get::<ChildOf>(child).map(|c| c.0),
            Some(grandparent),
            "the child was reparented to its grandparent",
        );
    }

    #[test]
    fn deleting_a_root_makes_its_children_roots() {
        let mut app = registry_app();
        let parent = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let child = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(parent))).id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(app.world().get_entity(child).is_ok());
        assert!(app.world().get::<ChildOf>(child).is_none());
    }
```

- [ ] **Step 4: Run to verify they fail**

Run: `cargo test -p sway-graph command::`
Expected: the five new tests FAIL (`Create`/`Delete` arms are still no-ops).

- [ ] **Step 5: Implement the two arms**

Replace the placeholder match arms in `apply_editor_command`:

```rust
        EditorCommand::Create { component, pos } => {
            let Some(registry) = world.get_resource::<crate::ComponentDocRegistry>() else {
                return;
            };
            let Some(type_id) = registry.by_name(component).map(|entry| entry.type_id) else {
                return; // an unregistered name is a no-op, not a panic
            };
            let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
                return;
            };

            let entity = world.spawn(EditorPos(*pos)).id();
            {
                // `AppTypeRegistry` is cloned above (it is an Arc) so the read
                // guard does not borrow `world` while the world is mutated.
                let registry = type_registry.read();
                let Some(registration) = registry.get(type_id) else {
                    return;
                };
                let (Some(reflect_component), Some(reflect_default)) = (
                    registration.data::<ReflectComponent>(),
                    registration.data::<ReflectDefault>(),
                ) else {
                    return;
                };
                let value = reflect_default.default();
                let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
                    return;
                };
                reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &registry);
            }
            // `EditorPos` is inserted before the component so a component that
            // `#[require]`s it does not overwrite the click position with a
            // default. Re-assert it afterwards in case it did.
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                entity_mut.insert(EditorPos(*pos));
            }
        }
        EditorCommand::Delete { entity } => {
            let Ok(entity_ref) = world.get_entity(*entity) else {
                return;
            };
            let parent = entity_ref.get::<ChildOf>().map(|c| c.0);
            let children: Vec<Entity> = entity_ref
                .get::<Children>()
                .map(|c| c.iter().copied().collect())
                .unwrap_or_default();

            for child in children {
                let Ok(mut child_mut) = world.get_entity_mut(child) else {
                    continue;
                };
                match parent {
                    Some(grandparent) => {
                        child_mut.insert(ChildOf(grandparent));
                    }
                    None => {
                        child_mut.remove::<ChildOf>();
                    }
                }
            }
            world.despawn(*entity);
        }
```

Add to the module's imports:

```rust
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_reflect::std_traits::ReflectDefault;
```

`register_authorable` already panics at startup for a type missing
`#[reflect(Component)]` or `#[reflect(Default)]`, so both `data::<…>()` lookups
above succeed for anything in the registry; the `else` arms are defensive, not
expected paths.

**If Step 2's test FAILED**, insert before `world.despawn(*entity)`:

```rust
            // Bevy does not clear wires naming a despawned producer (pinned by
            // tests/despawn_semantics.rs), so clear them here.
            let entries: Vec<(fn(&World, Entity) -> Option<Entity>, fn(&mut World, Entity))> = world
                .get_resource::<crate::WireRegistry>()
                .map(|r| r.entries.iter().map(|e| (e.read, e.remove)).collect())
                .unwrap_or_default();
            let all: Vec<Entity> = world.iter_entities().map(|e| e.id()).collect();
            for (read, remove) in entries {
                for consumer in &all {
                    if read(world, *consumer) == Some(*entity) {
                        remove(world, *consumer);
                    }
                }
            }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-graph`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(graph): Create and Delete commands

Create spawns the named component via ReflectDefault and lets #[require]
supply the companions. Delete reparents children to the grandparent before
despawning, since Bevy's despawn cascades through Children.

Characterizes what Bevy does to a wire whose producer despawns — the one
behaviour Delete depends on that no existing test pinned."
```

---

### Task 4: `SetField`

**Files:**
- Modify: `crates/sway-graph/src/command.rs`

**Interfaces:**
- Consumes: `FieldValue`, `apply_editor_command`.
- Produces: `SetField` writing one named field of one named component through reflection.

- [ ] **Step 1: Write the failing tests**

Add to `crates/sway-graph/src/command.rs`'s test module:

```rust
    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Knobs { gain: f32, steps: i64, on: bool }

    fn knobs_app() -> App {
        let mut app = registry_app();
        crate::register_authorable::<Knobs>(&mut app, "Knobs");
        app
    }

    #[test]
    fn set_field_writes_a_float_through_reflection() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "gain".to_string(),
                value: FieldValue::Float(0.75),
            },
        );

        assert_eq!(app.world().get::<Knobs>(entity).map(|k| k.gain), Some(0.75));
    }

    #[test]
    fn set_field_writes_ints_and_bools() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "steps".to_string(), value: FieldValue::Int(9),
        });
        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "on".to_string(), value: FieldValue::Bool(true),
        });

        let knobs = app.world().get::<Knobs>(entity).copied().unwrap();
        assert_eq!(knobs.steps, 9);
        assert!(knobs.on);
    }

    #[test]
    fn writing_an_equal_value_does_not_mark_the_component_changed() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs { gain: 0.5, ..Default::default() }).id();
        app.update();

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "gain".to_string(), value: FieldValue::Float(0.5),
        });

        assert!(!app.world().entity(entity).get_ref::<Knobs>().unwrap().is_changed());
    }

    #[test]
    fn a_type_mismatch_leaves_the_field_alone() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs { gain: 0.25, ..Default::default() }).id();

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity,
            component: "Knobs",
            field: "gain".to_string(),
            value: FieldValue::Bool(true),
        });

        assert_eq!(app.world().get::<Knobs>(entity).map(|k| k.gain), Some(0.25));
    }

    #[test]
    fn an_unknown_field_name_is_ignored() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();
        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "nope".to_string(), value: FieldValue::Float(1.0),
        });
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-graph command::set_field`
Expected: FAIL.

- [ ] **Step 3: Implement the arm**

Replace the `SetField` placeholder:

```rust
        EditorCommand::SetField { entity, component, field, value } => {
            let Some(type_id) = world
                .get_resource::<crate::ComponentDocRegistry>()
                .and_then(|r| r.by_name(component))
                .map(|entry| entry.type_id)
            else {
                return;
            };
            let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
                return;
            };
            let registry = type_registry.read();
            let Some(reflect_component) =
                registry.get(type_id).and_then(|r| r.data::<ReflectComponent>())
            else {
                return;
            };
            let Ok(mut entity_mut) = world.get_entity_mut(*entity) else {
                return;
            };
            let Some(mut reflected) = reflect_component.reflect_mut(entity_mut.reborrow()) else {
                return;
            };

            // Reach the field without marking `Changed` yet: `reflect_mut`
            // above already took a `Mut`, so the write below must be skipped
            // entirely when the value is equal, not merely made idempotent.
            let ReflectMut::Struct(target) = reflected.reflect_mut() else {
                return;
            };
            let Some(existing) = target.field_mut(field) else {
                return;
            };

            let replacement: Box<dyn PartialReflect> = match value {
                FieldValue::Float(v) => Box::new(*v),
                FieldValue::Int(v) => Box::new(*v),
                FieldValue::Bool(v) => Box::new(*v),
                FieldValue::Str(v) => Box::new(v.clone()),
                FieldValue::Vec3(v) => Box::new(*v),
                FieldValue::Enum(variant) => {
                    // A unit variant is addressed by name against the field's
                    // own type, so the caller never needs the type path.
                    let Some(info) = existing.get_represented_type_info() else {
                        return;
                    };
                    let Some(registration) = registry.get(info.type_id()) else {
                        return;
                    };
                    let Some(reflect_default) = registration.data::<ReflectDefault>() else {
                        return;
                    };
                    let mut candidate = reflect_default.default();
                    let ReflectMut::Enum(candidate_enum) = candidate.reflect_mut() else {
                        return;
                    };
                    let Some(variant_index) = (0..candidate_enum.variant_len())
                        .find(|i| candidate_enum.variant_name_at(*i) == Some(variant.as_str()))
                    else {
                        return;
                    };
                    let _ = variant_index;
                    // `DynamicEnum` names the variant directly; applying it to
                    // the concrete field converts it back.
                    Box::new(bevy_reflect::DynamicEnum::new(
                        variant.clone(),
                        bevy_reflect::DynamicVariant::Unit,
                    ))
                }
            };

            // Type mismatch and equal-value are both no-ops.
            if existing.reflect_partial_eq(replacement.as_ref()) == Some(true) {
                return;
            }
            if existing.try_apply(replacement.as_ref()).is_err() {
                return;
            }
        }
```

Add to the module's imports:

```rust
use bevy_reflect::{PartialReflect, ReflectMut};
```

**Note on `Changed`:** `reflect_mut` returns a `Mut`, which marks `Changed` on
deref regardless of whether a write follows. If
`writing_an_equal_value_does_not_mark_the_component_changed` fails for that
reason, restructure: read the current value through `reflect_component.reflect`
(immutable) first, compare, and take the mutable path only when they differ.
That is the shape the test is asserting, and it is the cheaper one anyway.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(graph): SetField writes one field through reflection

Type mismatches, unknown fields and equal values are all no-ops — the last
because Changed<T> is the whole dirty story downstream."
```

---

### Task 5: `Connect` and `Disconnect`

**Files:**
- Modify: `crates/sway-graph/src/command.rs`

**Interfaces:**
- Consumes: `WireEntry::{name, insert, remove, has_source, has_target}` — all already present.
- Produces: `Connect`/`Disconnect` arms; illegal pairings refused.

- [ ] **Step 1: Write the failing tests**

Add to the test module:

```rust
    use crate::test_wires::{GainFrom, spawn_float, spawn_gain};

    fn wired_app() -> App {
        let mut app = registry_app();
        crate::register_wire::<GainFrom>(&mut app);
        app
    }

    #[test]
    fn connect_inserts_the_wire() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src, dst },
        );

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn connect_replaces_an_existing_source_without_a_disconnect_first() {
        let mut app = wired_app();
        let first = spawn_float(app.world_mut(), 1.0);
        let second = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src: first, dst });
        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src: second, dst });

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(second));
    }

    #[test]
    fn connect_refuses_a_source_without_the_source_component() {
        let mut app = wired_app();
        let not_a_source = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src: not_a_source, dst },
        );

        assert!(app.world().get::<GainFrom>(dst).is_none(), "legality is enforced world-side too");
    }

    #[test]
    fn connect_refuses_a_target_without_the_target_component() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let not_a_target = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src, dst: not_a_target },
        );

        assert!(app.world().get::<GainFrom>(not_a_target).is_none());
    }

    #[test]
    fn disconnect_removes_the_wire_and_is_a_no_op_when_absent() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src, dst });

        apply_editor_command(app.world_mut(), &EditorCommand::Disconnect { wire: "factor", dst });
        assert!(app.world().get::<GainFrom>(dst).is_none());

        apply_editor_command(app.world_mut(), &EditorCommand::Disconnect { wire: "factor", dst });
    }

    #[test]
    fn a_connect_marks_the_topology_dirty_for_the_next_rebuild() {
        // The ordering guarantee M6-1 rests on: apply_editor_commands runs
        // before WatchSet, so the watch sees this frame's insert.
        let (mut app, tx) = command_app();
        crate::register_wire::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.update();
        app.update();

        tx.send(EditorCommand::Connect { wire: "factor", src, dst }).unwrap();
        app.update();

        assert_eq!(
            app.world().resource::<crate::GraphOrder>().steps.len(),
            1,
            "the new edge reached the order in the same frame the command arrived",
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-graph command::`
Expected: the six new tests FAIL.

- [ ] **Step 3: Implement the arms**

Replace the `Connect`/`Disconnect` placeholders:

```rust
        EditorCommand::Connect { wire, src, dst } => {
            let Some((insert, has_source, has_target)) = world
                .get_resource::<crate::WireRegistry>()
                .and_then(|r| r.entries.iter().find(|e| e.name == *wire))
                .map(|e| (e.insert, e.has_source, e.has_target))
            else {
                return;
            };
            // The editor filters illegal drops before sending, but a command
            // is data and may arrive stale — the world enforces it too.
            if !has_source(world, *src) || !has_target(world, *dst) {
                return;
            }
            insert(world, *dst, *src);
        }
        EditorCommand::Disconnect { wire, dst } => {
            let Some(remove) = world
                .get_resource::<crate::WireRegistry>()
                .and_then(|r| r.entries.iter().find(|e| e.name == *wire))
                .map(|e| e.remove)
            else {
                return;
            };
            remove(world, *dst);
        }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(graph): Connect and Disconnect commands

Legality is re-checked world-side via has_source/has_target: the editor
filters before sending, but a command is data and can arrive stale. Rewire
needs no prior disconnect — WireEntry::insert already replaces."
```

---

### Task 6: Snapshot extensions — canvas population, sockets, field kinds

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`, `crates/sway-editor/src/test_graph.rs`, `crates/sway-editor/src/canvas.rs`, `crates/sway-editor/src/inspector.rs`, `crates/sway-editor/src/lib.rs`

**Interfaces:**
- Produces: `InletView { wire: &'static str, connected: bool }`; `NodeView.inlets: Vec<InletView>`; `FieldKind`; `InspectorField { name: String, value: String, kind: FieldKind }`; `InspectorComponent { name: &'static str, fields: Vec<InspectorField> }`.
- Consumes: `sway_graph::{EditorPos, WireRegistry, ComponentDocRegistry}`.

**The `&'static str` decision, settled here.** `EditorCommand::SetField.component` is
`&'static str` (Task 2), so `InspectorComponent.name` becomes `&'static str` too —
it genuinely *is* `ComponentEntry::name`, and Task 8's inspector has to put it in a
command without leaking. `InspectorField.name` stays `String`, because a
`TupleStruct` field's name is `i.to_string()` and has no `'static` origin;
`EditorCommand::SetField.field` is `String` to match. Do not revisit this in Task 8.

- [ ] **Step 1: Give the test fixtures an `EditorPos`**

Canvas membership stops being "appears in `GraphOrder`" and becomes "carries
`EditorPos`" (M6-4). `test_graph.rs`'s spawn helpers only insert one when a
caller passes `Some(pos)`, so without this step every snapshot test that spawns
with `None` would see an empty node list — the new rule would look broken when
it is the fixtures that are stale.

In `crates/sway-editor/src/test_graph.rs`, replace the conditional insert in
each of the three spawn helpers with an unconditional one:

```rust
pub(crate) fn spawn_emit(world: &mut World, _id: u32, pos: Option<Vec2>) -> Entity {
    world
        .spawn((
            Name::new("Emit"),
            Emit,
            FloatOut(0.75),
            EditorPos(pos.unwrap_or(Vec2::ZERO)),
        ))
        .id()
}

pub(crate) fn spawn_recv(world: &mut World, _id: u32, pos: Option<Vec2>) -> Entity {
    world
        .spawn((
            Name::new("Recv"),
            Recv,
            Gain::default(),
            EditorPos(pos.unwrap_or(Vec2::ZERO)),
        ))
        .id()
}

pub(crate) fn spawn_spatial(world: &mut World, _id: u32, parent: Option<Entity>) -> Entity {
    let mut entity = world.spawn((Transform::default(), EditorPos(Vec2::ZERO)));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    entity.id()
}
```

`spawn_named_spatial` stays as it is: `a_name_component_wins_for_tree_labels`
only reads a tree row, and the tree is not `EditorPos`-gated.

No existing assertion depends on `pos` being `None` — `nodes_use_entity_ids_names_and_authored_positions`
passes `Some(..)` explicitly, and the canvas's own fallback-grid test builds its
`NodeView`s by hand rather than through these helpers.

- [ ] **Step 2: Write the failing tests**

First add one fixture to `crates/sway-editor/src/test_graph.rs` — an entity that
is genuinely the source of *two* registered wires, which is what the outlet
collapse needs to be tested against at all. `GainFrom`'s source is `FloatOut`
and `ChildOf`'s source is `Transform`, so an entity carrying both sources both:

```rust
/// Sources two registered wires at once (`amount` via `FloatOut`, `parent` via
/// `Transform`). M6-6 collapses that to a single outlet socket.
pub(crate) fn spawn_double_source(world: &mut World) -> Entity {
    world
        .spawn((
            Name::new("BothOut"),
            FloatOut(0.5),
            Transform::default(),
            EditorPos(Vec2::ZERO),
        ))
        .id()
}

/// Targets two registered wires at once, so its inlets are `[amount, parent]`
/// and an inbound `parent` edge has a non-zero ordinal to land on.
pub(crate) fn spawn_double_target(world: &mut World, parent: Option<Entity>) -> Entity {
    let mut entity = world.spawn((
        Name::new("BothIn"),
        Gain::default(),
        Transform::default(),
        EditorPos(Vec2::ZERO),
    ));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    entity.id()
}
```

Both are `pub(crate)`, and `Gain`/`FloatOut` are private to `test_graph`, which
is where these live — that is why they are fixtures rather than inline setup.

Then add to `crates/sway-editor/src/snapshot.rs`'s test module:

```rust
    #[test]
    fn a_node_with_no_wires_still_appears_on_the_canvas() {
        // M6-4. Before M6 the canvas drew only entities in GraphOrder, so a
        // camera or light was structurally invisible.
        let mut app = app();
        let lonely = app.world_mut().spawn(EditorPos(Vec2::new(3.0, 4.0))).id();
        recompile(&mut app);

        let snapshot = capture(app.world());

        assert!(
            snapshot.nodes.iter().any(|node| node.entity == lonely),
            "an EditorPos entity is a canvas node whether or not anything wires to it",
        );
    }

    #[test]
    fn an_entity_without_an_editor_pos_is_not_a_canvas_node() {
        let mut app = app();
        let runtime_owned = app.world_mut().spawn_empty().id();
        recompile(&mut app);

        assert!(!capture(app.world()).nodes.iter().any(|n| n.entity == runtime_owned));
    }

    #[test]
    fn an_entity_sourcing_several_wires_reports_exactly_one_outlet() {
        // M6-6. Counting per wire drew seven dots on an Lfo; only socket 0 has
        // ever had an edge attached, because capture_edges hardcodes from_field.
        // `spawn_double_source` sources both `amount` and `parent`, so the old
        // `count()` reported 2 here and the new `any()` reports 1.
        let mut app = app();
        let both = spawn_double_source(app.world_mut());
        recompile(&mut app);

        let node = capture(app.world()).nodes.into_iter().find(|n| n.entity == both).unwrap();
        assert_eq!(node.outlets, 1);
    }

    #[test]
    fn inlets_carry_their_wire_name_and_whether_they_are_connected() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        connect(app.world_mut(), emit, Emit::OUT_VALUE, recv, Recv::AMOUNT);
        recompile(&mut app);

        let node = capture(app.world()).nodes.into_iter().find(|n| n.entity == recv).unwrap();
        let amount = node.inlets.iter().find(|i| i.wire == "amount").expect("named inlet");
        assert!(amount.connected);
    }

    #[test]
    fn an_edge_lands_on_its_own_wires_inlet_ordinal_not_on_socket_zero() {
        // The to_field defect M6-6 names: every inbound edge used to draw into
        // socket 0 whatever wire it was.
        //
        // This needs a target with more than one inlet, or the assertion is
        // vacuous — `spawn_double_target` carries both `Gain` (target of
        // `amount`) and `Transform` (target of `parent`), and `WireRegistry`
        // order is registration order, so its inlets are `[amount, parent]`.
        // The `parent` edge must therefore land on ordinal 1, which is exactly
        // what the hardcoded `to_field: 0` got wrong.
        let mut app = app();
        let grandparent = spawn_spatial(app.world_mut(), 1, None);
        let both = spawn_double_target(app.world_mut(), Some(grandparent));
        recompile(&mut app);

        let snapshot = capture(app.world());
        let node = snapshot.nodes.iter().find(|n| n.entity == both).unwrap();
        assert_eq!(
            node.inlets.iter().map(|i| i.wire).collect::<Vec<_>>(),
            vec!["amount", "parent"],
            "inlet ordinals come from WireRegistry order",
        );

        let edge = snapshot
            .edges
            .iter()
            .find(|e| e.wire == "parent" && e.to == NodeId::of(both))
            .expect("the parenting edge");
        assert_eq!(edge.to_field, 1);
    }

    #[test]
    fn inspector_fields_carry_a_kind_the_widget_layer_can_switch_on() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app
            .world_mut()
            .spawn(sway_nodes::Lfo {
                beats: 4.0,
                shape: sway_nodes::Waveform::Saw,
                phase: 0.25,
                amplitude: 0.5,
            })
            .id();

        let view = inspect(app.world(), entity);
        let lfo = view.components.iter().find(|c| c.name == "Lfo").unwrap();

        let beats = lfo.fields.iter().find(|f| f.name == "beats").unwrap();
        assert_eq!(beats.kind, FieldKind::Float);

        let shape = lfo.fields.iter().find(|f| f.name == "shape").unwrap();
        match &shape.kind {
            FieldKind::Enum(variants) => {
                assert!(variants.iter().any(|v| v == "Saw"), "got {variants:?}");
                assert!(variants.len() > 1, "every variant is offered, not just the current one");
            }
            other => panic!("expected an enum kind, got {other:?}"),
        }
    }
```

Extend the test module's existing import to pull in the two new fixtures:

```rust
    use crate::test_graph::{
        Emit, Recv, app, connect, fixture_with_parenting, recompile, spawn_double_source,
        spawn_double_target, spawn_emit, spawn_named_spatial, spawn_recv, spawn_spatial,
    };
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p sway-editor snapshot::`
Expected: FAIL — `InletView`, `FieldKind` undefined; outlet count is 1 per wire.

- [ ] **Step 4: Replace the view types**

In `crates/sway-editor/src/snapshot.rs`:

```rust
/// One inlet socket: a registered wire type this entity could consume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InletView {
    pub wire: &'static str,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub entity: Entity,
    pub id: NodeId,
    pub name: String,
    pub pos: Option<Point>,
    pub inlets: Vec<InletView>,
    /// 0 or 1. Architecture §2: an outlet is a *component*, and no node in the
    /// current set carries two distinct source component types (spec M6-6).
    pub outlets: u16,
}

/// What kind of editor a field wants. The widget layer switches on this; the
/// snapshot layer never builds a widget.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Float,
    Int,
    Bool,
    /// Every variant name, current one included.
    Enum(Vec<String>),
    Str,
    Vec3,
    /// Anything the walk could not classify — rendered read-only, and the
    /// signal that a type wants editor `TypeData`.
    Opaque,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorField {
    /// A struct field's reflected name, or a tuple-struct field's index as a
    /// string — which is why this is owned and `InspectorComponent::name` is not.
    pub name: String,
    pub value: String,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorComponent {
    /// `ComponentEntry::name` verbatim, so Task 8 can put it straight into an
    /// `EditorCommand::SetField` without leaking a `String`.
    pub name: &'static str,
    pub fields: Vec<InspectorField>,
}
```

`inspect`'s existing `name: entry.name.to_string()` becomes `name: entry.name`.
The existing test `a_component_the_entity_does_not_have_is_not_listed` compares
`view.components[0].name == "FloatOut"`, which still compiles against
`&'static str`.

- [ ] **Step 5: Rewrite the three capture functions**

```rust
/// Every entity the canvas draws: those carrying `EditorPos` (spec M6-4).
fn canvas_entities(world: &World) -> Vec<Entity> {
    let mut entities: Vec<Entity> = world
        .iter_entities()
        .filter(|entity| entity.contains::<EditorPos>())
        .map(|entity| entity.id())
        .collect();
    entities.sort();
    entities
}

/// The inlet sockets of one entity, in `WireRegistry` order — which is
/// registration order, fixed at startup, so a socket's ordinal is stable.
fn inlets_of(world: &World, registry: &WireRegistry, entity: Entity) -> Vec<InletView> {
    registry
        .entries
        .iter()
        .filter(|entry| (entry.has_target)(world, entity))
        .map(|entry| InletView {
            wire: entry.name,
            connected: (entry.read)(world, entity).is_some(),
        })
        .collect()
}

fn capture_nodes(world: &World) -> Vec<NodeView> {
    let Some(registry) = world.get_resource::<WireRegistry>() else {
        return Vec::new();
    };
    canvas_entities(world)
        .into_iter()
        .map(|entity| NodeView {
            entity,
            id: NodeId::of(entity),
            name: world
                .get::<Name>(entity)
                .map(|name| name.as_str().to_string())
                .unwrap_or_else(|| format!("Entity {}", entity.index())),
            pos: world
                .get::<EditorPos>(entity)
                .map(|pos| Point::new(pos.0.x as f64, pos.0.y as f64)),
            inlets: inlets_of(world, registry, entity),
            outlets: registry
                .entries
                .iter()
                .any(|entry| (entry.has_source)(world, entity)) as u16,
        })
        .collect()
}

fn capture_edges(world: &World) -> Vec<EdgeView> {
    let (Some(order), Some(registry)) = (
        world.get_resource::<GraphOrder>(),
        world.get_resource::<WireRegistry>(),
    ) else {
        return Vec::new();
    };
    order
        .steps
        .iter()
        .filter_map(|step| match *step {
            Step::Propagate { src, dst, wire, .. } => {
                // The inlet ordinal, so the edge lands on its own socket
                // rather than always on socket 0 (spec M6-6).
                let to_field = inlets_of(world, registry, dst)
                    .iter()
                    .position(|inlet| inlet.wire == wire)
                    .unwrap_or(0) as u16;
                Some(EdgeView {
                    from: NodeId::of(src),
                    from_field: 0,
                    from_index: 0,
                    to: NodeId::of(dst),
                    to_field,
                    to_index: 0,
                    wire,
                    activity: None,
                })
            }
            Step::Run { .. } => None,
        })
        .collect()
}
```

`graph_entities` is still used by `capture_tree` to decide `TreeGroup::Graph`;
leave it in place.

- [ ] **Step 6: Classify field kinds**

Replace `fields_of` and add a classifier:

```rust
fn fields_of(value: &dyn PartialReflect) -> Vec<InspectorField> {
    match value.reflect_ref() {
        ReflectRef::Struct(s) => (0..s.field_len())
            .map(|i| {
                let field = s.field_at(i).expect("index in range");
                InspectorField {
                    name: s.name_at(i).unwrap_or("?").to_string(),
                    value: format_value(field),
                    kind: kind_of(field),
                }
            })
            .collect(),
        ReflectRef::TupleStruct(t) => (0..t.field_len())
            .map(|i| {
                let field = t.field(i).expect("index in range");
                InspectorField {
                    name: i.to_string(),
                    value: format_value(field),
                    kind: kind_of(field),
                }
            })
            .collect(),
        ReflectRef::Enum(e) => vec![InspectorField {
            name: "variant".to_string(),
            value: e.variant_name().to_string(),
            kind: enum_kind(value),
        }],
        _ => vec![InspectorField {
            name: String::new(),
            value: format_value(value),
            kind: kind_of(value),
        }],
    }
}

fn kind_of(value: &dyn PartialReflect) -> FieldKind {
    if value.try_downcast_ref::<f32>().is_some() || value.try_downcast_ref::<f64>().is_some() {
        return FieldKind::Float;
    }
    if value.try_downcast_ref::<i64>().is_some()
        || value.try_downcast_ref::<i32>().is_some()
        || value.try_downcast_ref::<u32>().is_some()
        || value.try_downcast_ref::<usize>().is_some()
    {
        return FieldKind::Int;
    }
    if value.try_downcast_ref::<bool>().is_some() {
        return FieldKind::Bool;
    }
    if value.try_downcast_ref::<String>().is_some() {
        return FieldKind::Str;
    }
    if value.try_downcast_ref::<bevy_math::Vec3>().is_some() {
        return FieldKind::Vec3;
    }
    if matches!(value.reflect_ref(), ReflectRef::Enum(_)) {
        return enum_kind(value);
    }
    FieldKind::Opaque
}

/// Every variant name of an enum field, from its `TypeInfo` — the current
/// value only tells us one of them, and the editor needs the whole list.
fn enum_kind(value: &dyn PartialReflect) -> FieldKind {
    let Some(bevy_reflect::TypeInfo::Enum(info)) = value.get_represented_type_info() else {
        return FieldKind::Opaque;
    };
    FieldKind::Enum(
        info.iter()
            .map(|variant| variant.name().to_string())
            .collect(),
    )
}
```

- [ ] **Step 7: Fix the consumers**

Four call sites see the changed types. Each is mechanical, but all four must
change together or the crate does not compile.

**1. `inspector.rs`'s `lines`** iterates `component.fields` as `(name, value)`
tuples, and `component.name` is no longer a `String`:

```rust
        for component in &snap.inspector.components {
            lines.push((component.name.to_string(), true));
            for field in &component.fields {
                lines.push((format!("{}  {}", field.name, field.value), false));
            }
        }
```

(Task 8 replaces this method wholesale; it is fixed here only so this task's
commit compiles on its own, which is the point of committing per task.)

**2. `canvas.rs`'s `apply_snapshot`.** `NodeSlot.inlets` and
`NodeBox::set_sockets` both keep their `Vec<u16>` type — those are *per-field
slot counts*, for the variadic inlets that stay out of MVP, and are a different
thing from the new `Vec<InletView>`. Translate at the boundary, once, at the top
of the per-node loop:

```rust
        for (index, view) in snap.nodes.iter().enumerate() {
            // One socket per inlet: a wire inlet is scalar. `NodeSlot.inlets`
            // and `NodeBox` still speak in per-field slot counts, so this is
            // the one place the two representations meet.
            let inlet_counts: Vec<u16> = vec![1; view.inlets.len()];
```

then substitute `inlet_counts` for every former `view.inlets` / `view.inlets.clone()`
in that loop body — the `slot.inlets != view.inlets` comparison, the
`slot.inlets = …` assignment, the `NodeBox::set_sockets` call, the
`.with_sockets(…)` call, and the `NodeSlot { inlets: …, .. }` initialiser.

**3. `canvas.rs`'s test module.** Its `node()` helper builds `inlets: Vec::new()`,
which is still valid. But `a_node_box_lays_out_one_socket_per_slot` builds
`inlets: vec![2, 1]` — three slots across two fields — which a snapshot can no
longer express, because an inlet is now one wire and therefore one slot. Rewrite
it to say what is now true:

```rust
    #[test]
    fn a_node_box_lays_out_one_socket_per_inlet() {
        // An inlet is one registered wire type, so it is always exactly one
        // socket. Variadic (`Vec`) inlets stay out of MVP; `NodeBox` still
        // carries the per-field count that would express them.
        use crate::snapshot::InletView;
        let view = NodeView {
            inlets: vec![
                InletView { wire: "amount", connected: true },
                InletView { wire: "parent", connected: false },
            ],
            outlets: 1,
            ..node(0, "Recv", None)
        };
        let mut harness = harness_with(snapshot(vec![view], vec![]));
        let box_id = harness.root_widget().widget_id_of(NodeId(0)).unwrap();

        harness.edit_widget_with_id(box_id, |mut widget| {
            let node_box = widget.downcast::<NodeBox>();
            assert_eq!(node_box.widget.inlet_socket_count(), 2);
            assert_eq!(node_box.widget.outlet_socket_count(), 1);
        });
    }
```

**4. `lib.rs`'s test `one_node_snapshot`** builds a `NodeView` with
`inlets: Vec::new()` — unchanged, since `Vec::new()` still infers.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p sway-editor`
Expected: PASS. Then `cargo test --workspace` — expect PASS.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(editor): canvas draws EditorPos entities; sockets gain identity

The canvas drew only entities in GraphOrder, so a camera or light was
structurally invisible (M5's findings flagged it). It now draws every entity
with an EditorPos.

Outlets collapse from one-per-wire to one-per-node: since capture_edges
hardcodes from_field: 0, sockets past the first had never carried an edge.
Fixes the matching to_field defect, where an inbound edge drew into the top
inlet socket whatever wire it was.

Inspector fields gain a FieldKind so the widget layer can switch on it."
```

---

## Phase 3 — The signal sink and inspector editing

### Task 7: A real `RenderRootSignal` sink

**Files:**
- Modify: `crates/sway-editor/src/lib.rs`, `crates/sway-app/src/presenter.rs`, `crates/sway-app/src/shell.rs`

**Interfaces:**
- Produces: `EditorUi::new(size, scale_factor, commands: Sender<EditorCommand>)`; `EditorUi::take_cursor() -> Option<CursorIcon>`; layers serviced automatically.
- Consumes: `sway_graph::EditorCommand`.

Verified against `xilem @ c5950bc`: `RenderRootSignal::NewLayer(LayerType, NewWidget<dyn Widget>, Point)`,
`RemoveLayer(WidgetId)`, `RepositionLayer(WidgetId, Point)`; serviced by
`RenderRoot::add_layer(root, pos)`, `remove_layer(root_id)`,
`reposition_layer(root_id, new_pos)` — exactly as `masonry_winit`'s
`event_loop_runner.rs` does at lines 1122-1128.

- [ ] **Step 1: Write the failing tests**

`RenderRoot::has_widget(WidgetId) -> bool` is public and answers exactly the
question these tests ask: did the widget masonry asked for actually get into the
tree? (`LayerStack::layer_count` is `pub(crate)` in `masonry_core` and therefore
out of reach — no test seam is needed, because `has_widget` is a stronger
assertion anyway.)

The tests live in `lib.rs`'s own `mod tests`, so they can reach `EditorUi`'s
private `signals` field and private `drain_signals` method directly. That is the
point: the sink is fed the same way masonry feeds it, and nothing about the
production path is bent to be testable.

Add to `crates/sway-editor/src/lib.rs`'s test module:

```rust
    #[test]
    fn a_new_layer_signal_puts_the_widget_in_the_tree() {
        // Before M6 the sink was a no-op, so no popup, tooltip or Selector
        // dropdown could appear at all: ctx.create_layer only *emits*
        // NewLayer, and the layer does not exist until the host calls back
        // into RenderRoot.
        use masonry_core::core::{LayerType, NewWidget};
        use masonry::widgets::Label;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        ui.redraw();

        let popup = NewWidget::new(Label::new("popup"));
        let popup_id = popup.id();
        assert!(!ui.root.has_widget(popup_id), "not in the tree before the signal");

        ui.signals.borrow_mut().push(RenderRootSignal::NewLayer(
            LayerType::Other,
            popup.erased(),
            KurboPoint::new(10.0, 10.0),
        ));
        ui.drain_signals();

        assert!(ui.root.has_widget(popup_id), "the layer signal was serviced");
    }

    #[test]
    fn a_remove_layer_signal_takes_the_widget_back_out() {
        use masonry_core::core::{LayerType, NewWidget};
        use masonry::widgets::Label;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);
        ui.redraw();

        let popup = NewWidget::new(Label::new("popup"));
        let popup_id = popup.id();
        ui.signals.borrow_mut().push(RenderRootSignal::NewLayer(
            LayerType::Other,
            popup.erased(),
            KurboPoint::new(10.0, 10.0),
        ));
        ui.drain_signals();

        ui.signals
            .borrow_mut()
            .push(RenderRootSignal::RemoveLayer(popup_id));
        ui.drain_signals();

        assert!(!ui.root.has_widget(popup_id));
    }

    #[test]
    fn a_set_cursor_signal_is_handed_to_the_shell_once() {
        // Drag-to-connect (Task 15) wants cursor feedback, and the shell owns
        // the window. Reading it clears it, so the shell does not re-apply the
        // same icon every frame.
        use masonry_core::core::CursorIcon;

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx);

        ui.signals
            .borrow_mut()
            .push(RenderRootSignal::SetCursor(CursorIcon::Crosshair));
        ui.drain_signals();

        assert_eq!(ui.take_cursor(), Some(CursorIcon::Crosshair));
        assert_eq!(ui.take_cursor(), None, "reading the request clears it");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-editor layer_signal`
Expected: FAIL — `EditorUi::new` takes two arguments, and there is no `signals`
field, `drain_signals` or `take_cursor`.

- [ ] **Step 3: Add the sink**

**`graph_root` gains the sender too.** The inspector (Task 8) and the canvas
(Task 13) both need to send commands, and both are built inside `graph_root`.
Threading it through now means neither of those tasks has to reshape this
function later:

```rust
/// … (existing doc comment unchanged)
///
/// `commands` is handed to the two panes that write: the inspector edits
/// fields, the canvas creates, deletes, moves and rewires. The tree and the
/// transport bar are read-only and do not get it.
fn graph_root(commands: Sender<EditorCommand>) -> NewWidget<dyn Widget> {
```

Its two write-capable children become `Inspector::new(commands.clone())` and
`GraphCanvas::new(commands)`. Both constructors take the sender **as of this
task** — add the parameter to each now, store it in the struct, and leave it
unused until Tasks 8 and 13 read it. `Inspector`'s and `GraphCanvas`'s
`impl Default` both call the no-argument `new` and must be deleted; the only
callers are `graph_root` and tests, which now pass a sender.

In `crates/sway-editor/src/lib.rs`, extend the struct and constructor:

```rust
use std::cell::RefCell;
use std::rc::Rc;
use crossbeam_channel::Sender;
use masonry_core::core::CursorIcon;
use sway_graph::EditorCommand;

pub struct EditorUi {
    root: RenderRoot,
    reducer: WindowEventReducer,
    scale_factor: f64,
    last_anim_tick: Instant,
    node_ids: HashMap<Entity, NodeId>,
    /// Masonry emits signals while it holds `RenderRoot` borrowed, and
    /// servicing a layer signal needs `&mut RenderRoot` — so they are
    /// collected here and drained afterwards, exactly as `masonry_winit` does.
    signals: Rc<RefCell<Vec<RenderRootSignal>>>,
    /// The most recent cursor request, for the shell to apply to the window.
    cursor: Option<CursorIcon>,
    commands: Sender<EditorCommand>,
}
```

In `new`, replace the no-op sink:

```rust
    pub fn new(
        size: PhysicalSize<u32>,
        scale_factor: f64,
        commands: Sender<EditorCommand>,
    ) -> Self {
        let signals: Rc<RefCell<Vec<RenderRootSignal>>> = Rc::new(RefCell::new(Vec::new()));
        let sink_signals = signals.clone();
        let root = RenderRoot::new(
            graph_root(commands.clone()),
            move |signal: RenderRootSignal| sink_signals.borrow_mut().push(signal),
            RenderRootOptions {
                default_properties: Arc::new(masonry::theme::default_property_set()),
                use_system_fonts: true,
                size_policy: WindowSizePolicy::User,
                size,
                scale_factor,
                test_font: None,
            },
        );
        Self {
            root,
            reducer: WindowEventReducer::default(),
            scale_factor,
            last_anim_tick: Instant::now(),
            node_ids: HashMap::new(),
            signals,
            cursor: None,
            commands,
        }
    }
```

- [ ] **Step 4: Drain the signals**

```rust
    /// Services everything masonry asked the host for since the last call.
    ///
    /// Layers are the load-bearing case: `ctx.create_layer` only *emits*
    /// `NewLayer`, and a popup does not exist until the host calls back into
    /// `RenderRoot`. Signals this editor has no use for (IME, clipboard,
    /// window geometry, `Exit`) are dropped deliberately — the shell owns the
    /// window and this editor has one, fixed, non-closable pane layout.
    fn drain_signals(&mut self) {
        let drained: Vec<RenderRootSignal> = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in drained {
            match signal {
                RenderRootSignal::NewLayer(_layer_type, root, pos) => {
                    self.root.add_layer(root, pos);
                }
                RenderRootSignal::RemoveLayer(root_id) => {
                    self.root.remove_layer(root_id);
                }
                RenderRootSignal::RepositionLayer(root_id, pos) => {
                    self.root.reposition_layer(root_id, pos);
                }
                RenderRootSignal::SetCursor(icon) => self.cursor = Some(icon),
                _ => {}
            }
        }
    }

    /// The pending cursor request, if any. Cleared by reading it.
    pub fn take_cursor(&mut self) -> Option<CursorIcon> {
        self.cursor.take()
    }
```

Call `self.drain_signals()` at the end of `handle_winit_event`, and at the
start of `redraw` (before `sync_selection`).

- [ ] **Step 5: Update the callers**

`crates/sway-app/src/presenter.rs` — `EditorPresenter::new` gains a
`commands: Sender<EditorCommand>` parameter and forwards it to
`EditorUi::new`. Add a `pub fn take_cursor(&mut self) -> Option<CursorIcon>`
forwarding to the editor.

`crates/sway-app/src/shell.rs` — `ShellConfig` gains
`pub commands: Sender<EditorCommand>`; `resumed` passes it to
`EditorPresenter::new`. After `presenter.handle_winit_event(...)` in
`window_event`, apply any cursor request:

```rust
        if let Presenter::Editor(presenter) = &mut running.presenter
            && let Some(icon) = presenter.take_cursor()
        {
            running.window.set_cursor(icon);
        }
```

`crates/sway-app/src/main.rs` — build the channel before `shell::run` and put
the receiver in the app:

```rust
    let (editor_tx, editor_rx) = crossbeam_channel::unbounded();
```

Inside `build_app`, alongside the existing `Authoring` insert:

```rust
        if editor {
            app.insert_resource(sway_graph::Authoring)
                .insert_resource(sway_graph::EditorRx(editor_rx));
        }
```

`editor_rx` must be moved into the closure; it is already `move`.

Then pass `commands: editor_tx` in the `ShellConfig`.

Every existing test that constructs one of the three now-sender-taking types
needs a throwaway sender — `let (tx, _rx) = crossbeam_channel::unbounded();`:

- `lib.rs`: the four `EditorUi::new(size, 1.0)` call sites.
- `canvas.rs`: `harness_with`'s `GraphCanvas::new()`.
- `inspector.rs`: it has no test module yet; Task 8 adds one.

Add `crossbeam-channel.workspace = true` to `crates/sway-editor/Cargo.toml`.
This is `sway-editor`'s only new dependency in M6, and it pulls in no
transitive `wgpu` or `winit` — check with `cargo tree -p sway-editor --depth 1`
if in doubt, since the manifest is where that constraint is enforced.

- [ ] **Step 6: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Verify by eye that nothing regressed**

Run: `cargo run -p sway-app -- --editor --windowed`
Expected: the editor opens as before — two cubes bobbing, tree and canvas
populated. The sink changes no visible behaviour yet; this run confirms it
broke nothing.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(editor): service RenderRootSignal instead of dropping it

The sink was a no-op, documented as an M1b simplification. That is fatal to a
writing editor: ctx.create_layer only emits NewLayer, and a popup does not
exist until the host calls back into RenderRoot. Layers, SetCursor and the
command channel all land here."
```

---

### Task 8: Editable inspector fields

**Files:**
- Modify: `crates/sway-editor/src/inspector.rs`

**Interfaces:**
- Consumes: `InspectorField`, `FieldKind`, `EditorCommand::SetField`, `FieldValue`.
- Produces: an `Inspector` that emits `SetField` on commit.

- [ ] **Step 1: Write the failing test**

Add to `crates/sway-editor/src/inspector.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{FieldKind, InspectorComponent, InspectorField, InspectorView};
    use bevy_ecs::entity::Entity;
    use masonry::core::DefaultProperties;
    use masonry_testing::TestHarness;
    use sway_graph::{EditorCommand, FieldValue};

    /// `TestHarness::create` takes the default property set and a *prepared*
    /// widget (`NewWidget<W>`) — see `canvas.rs`'s own `harness_with` for the
    /// same shape. `harness.root_widget()` is a `WidgetRef<W>`, which derefs
    /// to `W`, so the widget's own methods are called on it directly.
    fn harness_with(
        kind: FieldKind,
        value: &str,
    ) -> (TestHarness<Inspector>, crossbeam_channel::Receiver<EditorCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Inspector::new(tx).prepare());
        let snap = snapshot_with(kind, value);
        harness.edit_root_widget(|mut inspector| {
            Inspector::apply_snapshot(&mut inspector, &snap);
        });
        (harness, rx)
    }

    fn snapshot_with(kind: FieldKind, value: &str) -> WorldSnapshot {
        WorldSnapshot {
            inspector: InspectorView {
                entity: Some(Entity::from_raw_u32(3).expect("valid entity id")),
                components: vec![InspectorComponent {
                    name: "Knobs",
                    fields: vec![InspectorField {
                        name: "gain".to_string(),
                        value: value.to_string(),
                        kind,
                    }],
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_float_field_gets_an_editable_input_not_a_label() {
        let (harness, _rx) = harness_with(FieldKind::Float, "0.500");
        assert_eq!(harness.root_widget().editable_row_count(), 1);
    }

    #[test]
    fn a_bool_field_gets_a_checkbox_and_an_enum_gets_a_selector() {
        let (bools, _rx) = harness_with(FieldKind::Bool, "true");
        assert_eq!(bools.root_widget().editable_row_count(), 1);

        let (enums, _rx) = harness_with(
            FieldKind::Enum(vec!["Sine".to_string(), "Saw".to_string()]),
            "Saw",
        );
        assert_eq!(enums.root_widget().editable_row_count(), 1);
    }

    #[test]
    fn an_opaque_field_stays_read_only() {
        // Which remains the signal that a type wants editor TypeData.
        let (harness, _rx) = harness_with(FieldKind::Opaque, "?");
        assert_eq!(harness.root_widget().editable_row_count(), 0);
    }

    #[test]
    fn committing_a_float_sends_exactly_one_set_field() {
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 1, "0.75");
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::SetField { component, field, value: FieldValue::Float(v), .. }
                    if *component == "Knobs" && field == "gain" && (*v - 0.75).abs() < f32::EPSILON
            ),
            "got {:?}",
            commands[0],
        );
    }

    #[test]
    fn an_unparseable_float_sends_nothing() {
        // The field simply snaps back on the next snapshot.
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 1, "not a number");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn committing_on_a_header_row_sends_nothing() {
        // Row 0 is the "Knobs" header, which has no field to write.
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 0, "0.75");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }
}
```

**Row indices count headers.** `apply_snapshot` pushes one header row per
component followed by one row per field, so the single `gain` field above is row
**1**, not row 0. `commit` addresses rows by that same index, because that is
what `on_action` has in hand when it matches a child's `WidgetId`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p sway-editor inspector::`
Expected: FAIL — `Inspector::new` takes no sender.

- [ ] **Step 3: Rewrite the Inspector**

Replace `Row` and the struct:

```rust
/// One rendered row. A header or a read-only value is a `Label`; an editable
/// field is the widget its `FieldKind` calls for.
enum RowKind {
    Header(WidgetPod<Label>),
    ReadOnly(WidgetPod<Label>),
    Text { label: WidgetPod<Label>, input: WidgetPod<TextInput> },
    Bool { label: WidgetPod<Label>, toggle: WidgetPod<Checkbox> },
    Enum { label: WidgetPod<Label>, selector: WidgetPod<Selector> },
}

struct Row {
    kind: RowKind,
    /// Which component and field this row edits. `None` for headers.
    target: Option<(String, String, FieldKind)>,
}

pub struct Inspector {
    rows: Vec<Row>,
    signature: Vec<String>,
    generation: u64,
    entity: Option<Entity>,
    commands: Sender<EditorCommand>,
}
```

`Inspector::new(commands: Sender<EditorCommand>)`.

Add the parse-and-send path:

```rust
    /// Rows that accept input. The rest are headers and unclassified values.
    pub fn editable_row_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| !matches!(row.kind, RowKind::Header(_) | RowKind::ReadOnly(_)))
            .count()
    }

    /// Parses `text` against the row's `FieldKind` and sends a `SetField`.
    /// A value that does not parse sends nothing — the field simply snaps back
    /// on the next snapshot.
    fn commit(&mut self, row_index: usize, text: &str) {
        let Some(row) = self.rows.get(row_index) else { return };
        let (Some(entity), Some((component, field, kind))) = (self.entity, row.target.clone())
        else {
            return; // a header row, or nothing selected
        };
        let value = match kind {
            FieldKind::Float => match text.trim().parse::<f32>() {
                Ok(v) => FieldValue::Float(v),
                Err(_) => return,
            },
            FieldKind::Int => match text.trim().parse::<i64>() {
                Ok(v) => FieldValue::Int(v),
                Err(_) => return,
            },
            FieldKind::Bool => FieldValue::Bool(text == "true"),
            FieldKind::Enum(_) => FieldValue::Enum(text.to_string()),
            FieldKind::Str => FieldValue::Str(text.to_string()),
            FieldKind::Vec3 => {
                let parts: Vec<f32> = text
                    .split(',')
                    .filter_map(|p| p.trim().parse::<f32>().ok())
                    .collect();
                if parts.len() != 3 {
                    return;
                }
                FieldValue::Vec3(bevy_math::Vec3::new(parts[0], parts[1], parts[2]))
            }
            FieldKind::Opaque => return,
        };

        // Send-failure is not an error worth reporting: the only way the
        // receiver is gone is that the app is shutting down.
        let _ = self.commands.send(EditorCommand::SetField {
            entity,
            component,
            field,
            value,
        });
    }

    /// Test seam for `commit`, which is otherwise only reachable through a
    /// real text-input action.
    pub fn commit_for_test(this: &mut WidgetMut<'_, Self>, row_index: usize, text: &str) {
        this.widget.commit(row_index, text);
    }
```

`row.target`'s first element is the `&'static str` Task 6 settled on, so it
drops straight into `SetField.component` with no allocation and no leak.

The new imports this file needs:

```rust
use crossbeam_channel::Sender;
use bevy_ecs::entity::Entity;
use masonry::core::{ActionCtx, ErasedAction, PropertiesMut, WidgetId};
use masonry::widgets::{
    Checkbox, CheckboxToggled, Label, SelectionChanged, Selector, TextAction, TextInput,
};
use sway_graph::{EditorCommand, FieldValue};

use crate::snapshot::{FieldKind, WorldSnapshot};
```

- [ ] **Step 4: Build the rows**

The three widget constructors, read off the pinned checkout rather than assumed
(`masonry/src/widgets/{checkbox,selector,text_input}.rs`):

- `Checkbox::new(checked: bool, text: impl Into<ArcStr>)` — **two** arguments.
- `Selector::new(options: Vec<String>)`, then `.with_selected_option(usize)`.
  It `debug_panic!`s on an empty option list, so an `Enum` kind with no variants
  must not reach it.
- `TextInput::new(text: &str)` — a borrow, not a `String`.

Rewrite `apply_snapshot`. The signature comparison keeps its job (a steady
selection must cost one comparison per frame — the existing test
`an_unchanged_selection_does_not_rebuild_the_inspector` pins that), but it now
has to include the kind: two snapshots whose text matches while a field's kind
changed are genuinely different rows.

```rust
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let signature = signature_of(snap);
        if signature == this.widget.signature {
            return;
        }

        for row in std::mem::take(&mut this.widget.rows) {
            match row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => this.ctx.remove_child(pod),
                RowKind::Text { label, input } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(input);
                }
                RowKind::Bool { label, toggle } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(toggle);
                }
                RowKind::Enum { label, selector } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(selector);
                }
            }
        }

        this.widget.entity = snap.inspector.entity;

        if snap.inspector.entity.is_none() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("nothing selected"))),
                target: None,
            });
        }
        for component in &snap.inspector.components {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new(component.name))),
                target: None,
            });
            for field in &component.fields {
                let label = WidgetPod::new(Label::new(field.name.clone()));
                let kind = match &field.kind {
                    FieldKind::Bool => RowKind::Bool {
                        label,
                        toggle: WidgetPod::new(Checkbox::new(field.value == "true", "")),
                    },
                    // An enum with no variants cannot happen (`enum_kind` reads
                    // them off `TypeInfo`), but `Selector::new` debug-panics on
                    // an empty list, so it is rendered read-only rather than
                    // trusted.
                    FieldKind::Enum(variants) if !variants.is_empty() => RowKind::Enum {
                        label,
                        selector: WidgetPod::new(
                            Selector::new(variants.clone()).with_selected_option(
                                variants.iter().position(|v| *v == field.value).unwrap_or(0),
                            ),
                        ),
                    },
                    FieldKind::Opaque | FieldKind::Enum(_) => {
                        RowKind::ReadOnly(WidgetPod::new(Label::new(format!(
                            "{}  {}",
                            field.name, field.value
                        ))))
                    }
                    // Float, Int, Str and Vec3 all commit as text; `commit`
                    // parses each against its own kind.
                    _ => RowKind::Text {
                        label,
                        input: WidgetPod::new(TextInput::new(&field.value)),
                    },
                };
                this.widget.rows.push(Row {
                    kind,
                    target: Some((component.name, field.name.clone(), field.kind.clone())),
                });
            }
        }
        if this.widget.rows.is_empty() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("no authored components"))),
                target: None,
            });
        }

        this.widget.signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }
```

```rust
/// What makes two snapshots the same set of rows. Includes the kind, because a
/// field whose text is unchanged but whose kind changed needs a new widget.
fn signature_of(snap: &WorldSnapshot) -> Vec<String> {
    let mut signature = Vec::new();
    for component in &snap.inspector.components {
        signature.push(component.name.to_string());
        for field in &component.fields {
            signature.push(format!("{}={}#{:?}", field.name, field.value, field.kind));
        }
    }
    signature
}
```

`register_children`, `children_ids` and `layout` must each visit every pod in
every row. `WidgetPod<Label>` and `WidgetPod<Checkbox>` are different types, so
a single "give me this row's pods" helper would need type erasure; matching in
each of the three methods is shorter and keeps the pods concrete:

```rust
impl Widget for Inspector {
    type Action = ();

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for row in &mut self.rows {
            match &mut row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => ctx.register_child(pod),
                RowKind::Text { label, input } => {
                    ctx.register_child(label);
                    ctx.register_child(input);
                }
                RowKind::Bool { label, toggle } => {
                    ctx.register_child(label);
                    ctx.register_child(toggle);
                }
                RowKind::Enum { label, selector } => {
                    ctx.register_child(label);
                    ctx.register_child(selector);
                }
            }
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        let mut ids = Vec::new();
        for row in &self.rows {
            match &row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => ids.push(pod.id()),
                RowKind::Text { label, input } => ids.extend([label.id(), input.id()]),
                RowKind::Bool { label, toggle } => ids.extend([label.id(), toggle.id()]),
                RowKind::Enum { label, selector } => ids.extend([label.id(), selector.id()]),
            }
        }
        ids.into_iter().collect()
    }
```

`layout` splits each field row into a label column and an editor column; a
header or read-only row spans the full width, exactly as today:

```rust
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        const LABEL_WIDTH: f64 = 90.0;
        for (index, row) in self.rows.iter_mut().enumerate() {
            let y = index as f64 * ROW_HEIGHT;
            match &mut row.kind {
                RowKind::Header(pod) => {
                    let row_size = Size::new((size.width - PADDING).max(0.0), ROW_HEIGHT);
                    ctx.run_layout(pod, row_size);
                    ctx.place_child(pod, Point::new(PADDING, y));
                }
                RowKind::ReadOnly(pod) => {
                    let row_size = Size::new((size.width - PADDING * 2.0).max(0.0), ROW_HEIGHT);
                    ctx.run_layout(pod, row_size);
                    ctx.place_child(pod, Point::new(PADDING * 2.0, y));
                }
                RowKind::Text { label, input } => {
                    place_field(ctx, label, input, size, y, LABEL_WIDTH);
                }
                RowKind::Bool { label, toggle } => {
                    place_field(ctx, label, toggle, size, y, LABEL_WIDTH);
                }
                RowKind::Enum { label, selector } => {
                    place_field(ctx, label, selector, size, y, LABEL_WIDTH);
                }
            }
        }
        ctx.set_clip_path(size.to_rect());
    }
```

```rust
/// Lays a label/editor pair out on one row: the label in a fixed left column,
/// the editor filling what is left. Generic over the editor's widget type,
/// which is the only thing that differs between the three editable kinds.
fn place_field<W: Widget + ?Sized>(
    ctx: &mut LayoutCtx<'_>,
    label: &mut WidgetPod<Label>,
    editor: &mut WidgetPod<W>,
    size: Size,
    y: f64,
    label_width: f64,
) {
    let x = PADDING * 2.0;
    ctx.run_layout(label, Size::new(label_width, ROW_HEIGHT));
    ctx.place_child(label, Point::new(x, y));

    let editor_x = x + label_width;
    let editor_width = (size.width - editor_x - PADDING).max(0.0);
    ctx.run_layout(editor, Size::new(editor_width, ROW_HEIGHT));
    ctx.place_child(editor, Point::new(editor_x, y));
}
```

`measure`, `paint`, `accessibility_role` and `accessibility` are unchanged;
`content_height` is still `rows.len() * ROW_HEIGHT`.

- [ ] **Step 5: Route the child actions**

Each editor widget submits its own action type, and `Inspector::on_action`
receives it with the emitting child's `WidgetId`. Find the row that owns that
id, turn the action into text, and commit.

`TextInput` itself emits nothing — its inner `TextArea<true>` does, and
`TextInput::area_pod()` is how you get that child's id. `TextArea` emits
`TextAction::Entered` on Enter and `TextAction::Changed` on every keystroke, so
only `Entered` commits: committing on `Changed` would send a `SetField` per
character, and `1` would be written while someone was typing `10`.

```rust
    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        let Some((index, text)) = self.resolve_action(action, source) else {
            return;
        };
        self.commit(index, &text);
        ctx.set_handled();
    }
```

```rust
    /// Which row an action came from, and the text it commits.
    ///
    /// `None` when the action is one this widget does not act on — notably
    /// `TextAction::Changed`, which fires per keystroke.
    fn resolve_action(&self, action: &ErasedAction, source: WidgetId) -> Option<(usize, String)> {
        for (index, row) in self.rows.iter().enumerate() {
            match &row.kind {
                RowKind::Text { input, .. } => {
                    // The action comes from the TextArea inside the TextInput,
                    // not from the TextInput itself.
                    if input.id() != source && self.text_area_id(index) != Some(source) {
                        continue;
                    }
                    return match action.downcast_ref::<TextAction>()? {
                        TextAction::Entered(text) => Some((index, text.clone())),
                        TextAction::Changed(_) => None,
                    };
                }
                RowKind::Bool { toggle, .. } if toggle.id() == source => {
                    let CheckboxToggled(checked) = action.downcast_ref::<CheckboxToggled>()?;
                    return Some((index, checked.to_string()));
                }
                RowKind::Enum { selector, .. } if selector.id() == source => {
                    let changed = action.downcast_ref::<SelectionChanged>()?;
                    return Some((index, changed.selected_content.clone()));
                }
                _ => {}
            }
        }
        None
    }
```

`text_area_id` reads the inner pod's id off the live `TextInput`:

```rust
    fn text_area_id(&self, row: usize) -> Option<WidgetId> {
        match &self.rows.get(row)?.kind {
            RowKind::Text { input, .. } => Some(input.widget().area_pod().id()),
            _ => None,
        }
    }
```

If `WidgetPod` at this rev exposes no `widget()` accessor, store the id instead:
give `RowKind::Text` a third field `input_area: WidgetId`, filled in Step 4 from
the `TextInput` before it is put in a pod:

```rust
                    _ => {
                        let text_input = TextInput::new(&field.value);
                        let input_area = text_input.area_pod().id();
                        RowKind::Text {
                            label,
                            input: WidgetPod::new(text_input),
                            input_area,
                        }
                    }
```

That form needs no accessor at all and is the one to prefer if either compiles.
`commit_for_test` bypasses this whole path, so the four commit tests pass either
way — it is the by-eye check in Step 7 that exercises it.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-editor`
Expected: PASS.

- [ ] **Step 7: Verify by eye**

Run: `cargo run -p sway-app -- --editor --windowed`
Select `lfoA` in the tree, change `beats` from `8.0` to `2.0`, press Enter.
Expected: the corresponding cube's bob visibly speeds up. This is M6's first
end-to-end write.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(editor): editable inspector fields

One widget per FieldKind; unclassified fields stay read-only, which remains
the signal that a type wants editor TypeData. A value that does not parse
sends nothing and snaps back on the next snapshot."
```

---

## Phase 4 — Files

### Task 9: `sway-document` claims `EditorPos` entities

**Files:**
- Create: `crates/sway-document/src/claim.rs`
- Modify: `crates/sway-document/src/lib.rs`, `crates/sway-document/src/asset.rs`

**Interfaces:**
- Produces: `claim_editor_entities(world: &mut World)`, added to `ProjectPlugin` in `PreUpdate` after the apply chain.

- [ ] **Step 1: Write the failing test**

`crates/sway-document/src/claim.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_math::Vec2;
    use sway_graph::EditorPos;

    fn claim_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy_asset::AssetPlugin::default())
            .add_plugins(crate::ProjectPlugin);
        sway_graph::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn an_editor_pos_entity_without_a_doc_id_is_claimed() {
        let mut app = claim_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        assert!(app.world().get::<DocId>(entity).is_some());
    }

    #[test]
    fn a_runtime_entity_without_an_editor_pos_is_not_claimed() {
        let mut app = claim_app();
        let entity = app.world_mut().spawn_empty().id();

        app.update();

        assert!(
            app.world().get::<DocId>(entity).is_none(),
            "emit.rs's guarantee that runtime-owned entities stay out of the \
             document depends on this",
        );
    }

    #[test]
    fn claimed_ids_do_not_collide() {
        let mut app = claim_app();
        let a = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let b = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        let id_a = app.world().get::<DocId>(a).cloned().unwrap();
        let id_b = app.world().get::<DocId>(b).cloned().unwrap();
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn a_claimed_id_does_not_collide_with_one_the_document_already_named() {
        let mut app = claim_app();
        app.world_mut().spawn((EditorPos(Vec2::ZERO), DocId("EditorPos".to_string())));
        let fresh = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        app.update();

        assert_ne!(
            app.world().get::<DocId>(fresh).cloned().unwrap().0,
            "EditorPos".to_string(),
        );
    }

    #[test]
    fn an_already_claimed_entity_keeps_its_id() {
        let mut app = claim_app();
        let entity = app
            .world_mut()
            .spawn((EditorPos(Vec2::ZERO), DocId("keepme".to_string())))
            .id();

        app.update();
        app.update();

        assert_eq!(app.world().get::<DocId>(entity).unwrap().0, "keepme");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p sway-document claim::`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement**

At the top of `crates/sway-document/src/claim.rs`:

```rust
//! Claiming editor-created entities for the document. Spec M6-3.
//!
//! `to_document` emits only entities carrying a `DocId`, and a
//! palette-created entity has none — but `DocId` is a document component and
//! the editor cannot write one. So the document layer notices and claims.
//!
//! `EditorPos` is the marker because it already means "authored on the
//! canvas": runtime-spawned entities never carry one, which is what keeps
//! `emit.rs`'s `an_entity_without_a_doc_id_is_not_in_the_document` true.

use std::collections::HashSet;

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use sway_graph::{ComponentDocRegistry, EditorPos};

use crate::diagnostics::DocId;

pub fn claim_editor_entities(world: &mut World) {
    let unclaimed: Vec<Entity> = world
        .iter_entities()
        .filter(|entity| entity.contains::<EditorPos>() && !entity.contains::<DocId>())
        .map(|entity| entity.id())
        .collect();
    if unclaimed.is_empty() {
        return;
    }

    let mut taken: HashSet<String> = world
        .iter_entities()
        .filter_map(|entity| entity.get::<DocId>().map(|id| id.0.clone()))
        .collect();

    for entity in unclaimed {
        let stem = stem_for(world, entity);
        let mut candidate = stem.clone();
        let mut n = 0u32;
        while taken.contains(&candidate) {
            n += 1;
            candidate = format!("{stem}.{n:03}");
        }
        taken.insert(candidate.clone());
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            entity_mut.insert(DocId(candidate));
        }
    }
}

/// The name of the first component this entity carries in
/// `ComponentDocRegistry` order — registration order, fixed at startup and
/// therefore deterministic.
fn stem_for(world: &World, entity: Entity) -> String {
    let Some(registry) = world.get_resource::<ComponentDocRegistry>() else {
        return "node".to_string();
    };
    let Ok(entity_ref) = world.get_entity(entity) else {
        return "node".to_string();
    };
    for entry in &registry.entries {
        let Some(component_id) = world.components().get_id(entry.type_id) else {
            continue;
        };
        if entity_ref.contains_id(component_id) {
            return entry.name.to_string();
        }
    }
    "node".to_string()
}
```

- [ ] **Step 4: Register it**

In `crates/sway-document/src/lib.rs`, add `pub mod claim;` and
`pub use claim::claim_editor_entities;`.

In `asset.rs`'s `ProjectPlugin::build`, extend the chain:

```rust
            .add_systems(
                PreUpdate,
                (
                    note_project_changes,
                    note_load_failures,
                    apply_pending_project,
                    crate::claim::claim_editor_entities,
                )
                    .chain(),
            );
```

Claiming runs *after* apply, so an entity the document just named keeps the
document's id rather than being handed a generated one.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-document`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(document): claim EditorPos entities the editor created

The editor cannot write a DocId — it does not link the document model — so
the document layer notices entities carrying EditorPos without one and
assigns a derived unique id. Runs after apply, so a document-named entity
keeps its own id."
```

---

### Task 10: Save, open, and reload suppression

**Files:**
- Create: `crates/sway-document/src/file.rs`
- Modify: `crates/sway-document/src/lib.rs`, `crates/sway-document/src/asset.rs`

**Interfaces:**
- Produces: `CurrentDocument { path: Option<PathBuf> }`, `save_to_path(world, &Path) -> Result<(), String>`, `open_from_path(world, &Path) -> Result<(), String>`, `LastApplied(Option<ProjectDoc>)`.

- [ ] **Step 1: Write the failing test**

`crates/sway-document/src/file.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_math::Vec2;
    use sway_graph::EditorPos;

    fn file_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy_asset::AssetPlugin::default())
            .add_plugins(crate::ProjectPlugin);
        sway_graph::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn save_then_open_reproduces_the_world() {
        let dir = std::env::temp_dir().join("sway-m6-save-open");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("round.sway.ron");

        let mut app = file_app();
        app.world_mut().spawn(EditorPos(Vec2::new(5.0, 6.0)));
        app.update();
        save_to_path(app.world_mut(), &path).expect("saves");

        let mut reopened = file_app();
        open_from_path(reopened.world_mut(), &path).expect("opens");

        let positions: Vec<Vec2> = reopened
            .world_mut()
            .query::<&EditorPos>()
            .iter(reopened.world())
            .map(|p| p.0)
            .collect();
        assert_eq!(positions, vec![Vec2::new(5.0, 6.0)]);
    }

    #[test]
    fn saving_records_the_path_for_a_later_plain_save() {
        let dir = std::env::temp_dir().join("sway-m6-save-path");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.sway.ron");

        let mut app = file_app();
        save_to_path(app.world_mut(), &path).expect("saves");

        assert_eq!(app.world().resource::<CurrentDocument>().path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn a_document_equal_to_the_last_applied_one_is_skipped() {
        // Suppresses the reload a Save of a watched file triggers.
        let mut app = file_app();
        app.world_mut().spawn(EditorPos(Vec2::ZERO));
        app.update();
        let doc = crate::to_document(app.world_mut());
        app.world_mut().insert_resource(LastApplied(Some(doc.clone())));

        assert!(should_skip(app.world(), &doc));
    }

    #[test]
    fn a_changed_document_is_not_skipped() {
        let mut app = file_app();
        app.world_mut().spawn(EditorPos(Vec2::ZERO));
        app.update();
        let doc = crate::to_document(app.world_mut());
        app.world_mut().insert_resource(LastApplied(Some(doc)));

        app.world_mut().spawn(EditorPos(Vec2::ONE));
        app.update();
        let changed = crate::to_document(app.world_mut());

        assert!(!should_skip(app.world(), &changed));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p sway-document file::`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Open and save, by path. Spec M6-8.
//!
//! Deliberately not through the `AssetServer`: asset paths resolve against
//! the `assets/` root, so a dialog-picked absolute path cannot round-trip
//! through it.

use std::path::{Path, PathBuf};

use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;

use crate::apply::apply;
use crate::doc::{ProjectDoc, parse};
use crate::emit::{to_document, to_ron};

/// The file the editor is currently editing. `None` until the first Save As.
#[derive(Resource, Default)]
pub struct CurrentDocument {
    pub path: Option<PathBuf>,
}

/// The most recently applied document, for suppressing the reload a Save of a
/// watched file triggers.
#[derive(Resource, Default)]
pub struct LastApplied(pub Option<ProjectDoc>);

/// Whether an incoming document is the one already in the world.
pub fn should_skip(world: &World, incoming: &ProjectDoc) -> bool {
    world
        .get_resource::<LastApplied>()
        .and_then(|last| last.0.as_ref())
        .is_some_and(|last| last == incoming)
}

pub fn save_to_path(world: &mut World, path: &Path) -> Result<(), String> {
    let doc = to_document(world);
    let text = to_ron(&doc).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    world.insert_resource(CurrentDocument { path: Some(path.to_path_buf()) });
    world.insert_resource(LastApplied(Some(doc)));
    Ok(())
}

pub fn open_from_path(world: &mut World, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = parse(&text).map_err(|e| e.to_string())?;
    let diagnostics = apply(world, &doc);
    world.insert_resource(diagnostics);
    world.insert_resource(CurrentDocument { path: Some(path.to_path_buf()) });
    world.insert_resource(LastApplied(Some(doc)));
    // The watcher on any previously-loaded asset path stops mattering.
    if let Some(mut handle) = world.get_resource_mut::<crate::asset::ProjectHandle>() {
        handle.0 = None;
    }
    Ok(())
}
```

- [ ] **Step 4: Apply the suppression in the asset path**

In `asset.rs`'s `apply_pending_project`, before applying:

```rust
    if crate::file::should_skip(world, &doc) {
        return;
    }
```

and after a successful apply, record it:

```rust
    world.insert_resource(crate::file::LastApplied(Some(doc.clone())));
```

Register both resources in `ProjectPlugin::build`:

```rust
            .init_resource::<crate::file::CurrentDocument>()
            .init_resource::<crate::file::LastApplied>()
```

Add `pub mod file;` and
`pub use file::{CurrentDocument, LastApplied, open_from_path, save_to_path};`
to `lib.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(document): open and save by path, with reload suppression

std::fs rather than the AssetServer, whose paths resolve against assets/ and
so cannot carry a dialog-picked absolute path. An incoming document equal to
the last applied one is skipped, which is what stops a Save of a watched file
from reloading itself."
```

---

### Task 11: File dialogs and the toolbar

**Files:**
- Modify: `crates/sway-app/Cargo.toml`, `crates/sway-app/src/presenter.rs`, `crates/sway-app/src/shell.rs`, `crates/sway-editor/src/transport_bar.rs`, `crates/sway-editor/src/lib.rs`

**Interfaces:**
- Produces: `sway_editor::FileRequest { Open, Save, SaveAs }`; `TransportBar::take_file_requests() -> Vec<FileRequest>`; `EditorUi::take_file_requests() -> Vec<FileRequest>`.
- Consumes: `sway_document::{CurrentDocument, open_from_path, save_to_path}` from Task 10.

**Recorded deviation from the spec.** M6-8 sketches
`FileCommand { Open(PathBuf), Save, SaveAs(PathBuf) }`. A path-carrying variant
is unbuildable by the widget that emits it: only `sway-app` owns `rfd`, so the
toolbar cannot know a path at the moment it is clicked. The enum therefore
carries no paths and is named `FileRequest` to say so — a request the shell
turns into a dialog and then into a `sway_document` call. Nothing downstream
wanted the path-carrying form: the shell was always going to be both the
producer and the consumer of it. Record this in the commit message.

- [ ] **Step 1: Verify `rfd`'s async future is pollable without an executor**

Spec "Verify before implementing" item 3, and the load-bearing assumption of
Step 5. `rfd`'s blocking form spins a nested `NSApplication` modal on the very
thread winit's event loop is running on; the async form is only usable here if
its future can be polled straight from `redraw` with no executor underneath.

Add to `crates/sway-app/Cargo.toml`:

```toml
rfd = "0.15"
```

Create `crates/sway-app/tests/rfd_pollable.rs`:

```rust
//! Spec "Verify before implementing" item 3: `rfd::AsyncFileDialog`'s future
//! must be pollable from the shell's redraw loop, which has no executor under
//! it. If this ever fails, `shell.rs`'s `Dialog` has to become a thread plus a
//! channel instead.
//!
//! `#[ignore]` because it opens a real file dialog on some platforms and would
//! block CI. Run it by hand once, when adding `rfd` or bumping it:
//! `cargo test -p sway-app --test rfd_pollable -- --ignored`

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[test]
#[ignore = "opens a real file dialog; run by hand when adding or bumping rfd"]
fn an_async_file_dialog_future_polls_pending_without_an_executor() {
    let mut future = pin!(rfd::AsyncFileDialog::new().pick_file());
    let mut cx = Context::from_waker(Waker::noop());

    // One poll, no executor, no runtime. Pending is the pass: the dialog is
    // open and nobody has picked anything yet.
    assert!(matches!(future.as_mut().poll(&mut cx), Poll::Pending));
}
```

Run: `cargo test -p sway-app --test rfd_pollable -- --ignored`
Expected: PASS (a file dialog appears; dismiss it).

**If it panics or hangs instead**, `rfd`'s async form needs an executor on this
platform. Fall back to running the blocking `rfd::FileDialog` on a spawned
thread that sends its result back over a `std::sync::mpsc` channel, and have
`Dialog::poll` do a `try_recv` instead of a future poll — the rest of Step 5 is
unchanged, because `Dialog` is the only thing that knows which it is. Record the
deviation in the commit message.

- [ ] **Step 2: Write the failing test for the toolbar**

Add to `crates/sway-editor/src/transport_bar.rs`'s existing test module:

```rust
    #[test]
    fn the_save_button_emits_a_save_request() {
        use crate::FileRequest;
        let mut harness = harness_with(snapshot(false, 120.0, "001.1.1", true));
        let save_id = harness.root_widget().save_button_id();

        harness.mouse_click_on(save_id, Some(masonry::core::PointerButton::Primary));

        harness.edit_root_widget(|mut bar| {
            assert_eq!(
                TransportBar::take_file_requests(&mut bar),
                vec![FileRequest::Save],
            );
        });
    }

    #[test]
    fn taking_the_requests_drains_them() {
        use crate::FileRequest;
        let mut harness = harness_with(snapshot(false, 120.0, "001.1.1", true));
        let open_id = harness.root_widget().open_button_id();

        harness.mouse_click_on(open_id, Some(masonry::core::PointerButton::Primary));

        harness.edit_root_widget(|mut bar| {
            assert_eq!(
                TransportBar::take_file_requests(&mut bar),
                vec![FileRequest::Open],
            );
            assert!(
                TransportBar::take_file_requests(&mut bar).is_empty(),
                "the shell must not act on the same request twice",
            );
        });
    }
```

`mouse_click_on(WidgetId, Option<PointerButton>)` is `TestHarness`'s own helper
for pressing a specific widget — it moves the pointer there first, so it works
regardless of where the buttons land in the strip's layout.

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p sway-editor transport_bar::`
Expected: FAIL — no `FileRequest`, no `save_button_id`, no `take_file_requests`.

- [ ] **Step 4: Add `FileRequest` and the three buttons**

In `crates/sway-editor/src/lib.rs`:

```rust
/// A file operation the shell performs, asked for by the toolbar.
///
/// Lives here rather than in `sway-document` because it is a UI intent: the
/// editor asks for a file to be opened without knowing what parsing one means.
/// It carries no path — see the deviation note on this task: only `sway-app`
/// owns `rfd`, so a path does not exist until the shell has run a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRequest {
    Open,
    Save,
    SaveAs,
}
```

In `transport_bar.rs`, give `TransportBar` three `Button` children and an
outbox. The buttons are built once in `new` rather than rebuilt in
`apply_snapshot`, which is what keeps the existing "an unchanged snapshot
rebuilds nothing" guarantee true — `apply_snapshot` only ever touches `labels`:

```rust
pub struct TransportBar {
    labels: Vec<WidgetPod<Label>>,
    fields: Vec<String>,
    generation: u64,
    playing: bool,
    /// Open / Save / Save As, in that order. Built once; never rebuilt by a
    /// snapshot.
    buttons: [WidgetPod<Button>; 3],
    /// What the toolbar has asked for since the shell last drained it.
    requests: Vec<FileRequest>,
}

impl TransportBar {
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            fields: Vec::new(),
            generation: 0,
            playing: false,
            buttons: [
                WidgetPod::new(Button::with_text("Open")),
                WidgetPod::new(Button::with_text("Save")),
                WidgetPod::new(Button::with_text("Save As")),
            ],
            requests: Vec::new(),
        }
    }

    pub fn open_button_id(&self) -> WidgetId {
        self.buttons[0].id()
    }

    pub fn save_button_id(&self) -> WidgetId {
        self.buttons[1].id()
    }

    pub fn save_as_button_id(&self) -> WidgetId {
        self.buttons[2].id()
    }
}
```

`Button::with_text(impl Into<Arc<str>>)` and its action
`ButtonPress { button: Option<PointerButton> }` are both from
`masonry::widgets`; a button submits `ButtonPress` on release and on an
accessibility click.

```rust
// --- MARK: WIDGETMUT
impl TransportBar {
    /// Drains what the toolbar has asked for. Called once per frame by the
    /// shell, through `EditorUi::take_file_requests`.
    pub fn take_file_requests(this: &mut WidgetMut<'_, Self>) -> Vec<FileRequest> {
        std::mem::take(&mut this.widget.requests)
    }
}
```

The `Widget` impl gains an action handler and includes the buttons in the three
child-visiting methods:

```rust
    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        if action.downcast_ref::<ButtonPress>().is_none() {
            return;
        }
        let request = match self.buttons.iter().position(|b| b.id() == source) {
            Some(0) => FileRequest::Open,
            Some(1) => FileRequest::Save,
            Some(2) => FileRequest::SaveAs,
            _ => return,
        };
        self.requests.push(request);
        ctx.set_handled();
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for label in &mut self.labels {
            ctx.register_child(label);
        }
        for button in &mut self.buttons {
            ctx.register_child(button);
        }
    }

    fn children_ids(&self) -> ChildrenIds {
        self.labels
            .iter()
            .map(|label| label.id())
            .chain(self.buttons.iter().map(|button| button.id()))
            .collect()
    }
```

`layout` places the buttons after the readout fields, at the same pitch:

```rust
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, label) in self.labels.iter_mut().enumerate() {
            let x = PADDING + index as f64 * FIELD_WIDTH;
            ctx.run_layout(label, Size::new(FIELD_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(label, Point::new(x, 0.0));
        }
        let buttons_start = PADDING + self.labels.len() as f64 * FIELD_WIDTH;
        for (index, button) in self.buttons.iter_mut().enumerate() {
            let x = buttons_start + index as f64 * BUTTON_WIDTH;
            ctx.run_layout(button, Size::new(BUTTON_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(button, Point::new(x, 0.0));
        }
        ctx.set_clip_path(size.to_rect());
    }
```

with, beside the existing `FIELD_WIDTH`:

```rust
/// Fixed column width per file button.
const BUTTON_WIDTH: f64 = 72.0;
```

and `measure`'s `MaxContent` horizontal arm widened to match:

```rust
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(
                PADDING
                    + self.labels.len() as f64 * FIELD_WIDTH
                    + self.buttons.len() as f64 * BUTTON_WIDTH,
            ),
```

New imports for this file: `masonry::core::{ActionCtx, ErasedAction, PropertiesMut, WidgetId}`,
`masonry::widgets::{Button, ButtonPress}`, and `crate::FileRequest`.

Finally, in `lib.rs`, forward from the tagged transport bar:

```rust
    /// What the toolbar has asked the shell to do since the last call.
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        self.root.edit_widget_with_tag(TRANSPORT_BAR_TAG, |mut bar| {
            TransportBar::take_file_requests(&mut bar)
        })
    }
```

- [ ] **Step 5: Drive the dialogs from the shell**

In `presenter.rs`, forward from `EditorPresenter`:

```rust
    /// What the toolbar has asked for. Drained by the shell each redraw.
    pub fn take_file_requests(&mut self) -> Vec<sway_editor::FileRequest> {
        self.editor.take_file_requests()
    }
```

Add `sway-document.workspace = true` to `crates/sway-app/Cargo.toml` if Task 1
did not already (it did — this is a check, not a change).

In `shell.rs`, one small state machine, so the event loop does not grow a
second one:

```rust
/// A file dialog in flight.
///
/// `rfd`'s async form returns a future the shell polls once per redraw; the
/// blocking form would spin a nested `NSApplication` modal on the thread
/// winit's event loop already owns (M6-8). Exactly one dialog is ever open:
/// a second request while one is pending is dropped, which is also what a
/// modal dialog would do.
struct Dialog {
    kind: DialogKind,
    future: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    Open,
    Save,
}

impl Dialog {
    fn open() -> Self {
        Self {
            kind: DialogKind::Open,
            future: Box::pin(
                rfd::AsyncFileDialog::new()
                    .add_filter("sway project", &["ron"])
                    .pick_file(),
            ),
        }
    }

    fn save() -> Self {
        Self {
            kind: DialogKind::Save,
            future: Box::pin(
                rfd::AsyncFileDialog::new()
                    .add_filter("sway project", &["ron"])
                    .set_file_name("untitled.sway.ron")
                    .save_file(),
            ),
        }
    }

    /// One poll. `None` means still open; `Some(None)` means cancelled.
    fn poll(&mut self) -> Poll<Option<PathBuf>> {
        let mut cx = Context::from_waker(Waker::noop());
        self.future
            .as_mut()
            .poll(&mut cx)
            .map(|handle| handle.map(|h| h.path().to_path_buf()))
    }
}
```

`Running` gains `pending_dialog: Option<Dialog>`, initialised to `None` in
`resumed`. Both new steps go in `Running::redraw`, after the `present` call and
before `request_redraw`:

```rust
    fn redraw(&mut self) {
        match &mut self.presenter { /* unchanged */ }

        // The toolbar's requests, then one poll of whatever dialog is open.
        // Both only exist on the editor path; the show path has no toolbar.
        if let Presenter::Editor(presenter) = &mut self.presenter {
            let requests = presenter.take_file_requests();
            for request in requests {
                if self.pending_dialog.is_some() {
                    // A modal dialog is already up; ignore the rest.
                    break;
                }
                match request {
                    FileRequest::Save => match self.current_path() {
                        Some(path) => self.save(&path),
                        // Never saved: Save means Save As.
                        None => self.pending_dialog = Some(Dialog::save()),
                    },
                    FileRequest::SaveAs => self.pending_dialog = Some(Dialog::save()),
                    FileRequest::Open => self.pending_dialog = Some(Dialog::open()),
                }
            }
        }
        self.poll_dialog();

        self.window.request_redraw();
    }
```

```rust
impl Running {
    /// The file the document currently lives in, if it has ever been saved.
    fn current_path(&self) -> Option<PathBuf> {
        self.app
            .world()
            .get_resource::<sway_document::CurrentDocument>()
            .and_then(|current| current.path.clone())
    }

    /// Advances the open dialog, if any, and applies its result.
    ///
    /// A failure here is reported and dropped: a bad path or an unparseable
    /// file must not take the editor down mid-session (global constraint —
    /// panics are startup-only).
    fn poll_dialog(&mut self) {
        let Some(dialog) = &mut self.pending_dialog else {
            return;
        };
        let Poll::Ready(picked) = dialog.poll() else {
            return;
        };
        let kind = dialog.kind;
        self.pending_dialog = None;

        // `None` is a cancelled dialog, which is not an error.
        let Some(path) = picked else {
            return;
        };
        match kind {
            DialogKind::Open => {
                if let Err(error) = sway_document::open_from_path(self.app.world_mut(), &path) {
                    eprintln!("open failed: {error}");
                }
            }
            DialogKind::Save => self.save(&path),
        }
    }

    fn save(&mut self, path: &Path) {
        if let Err(error) = sway_document::save_to_path(self.app.world_mut(), path) {
            eprintln!("save failed: {error}");
        }
    }
}
```

New imports for `shell.rs`: `std::future::Future`, `std::path::{Path, PathBuf}`,
`std::pin::Pin`, `std::task::{Context, Poll, Waker}`, and
`sway_editor::FileRequest`.

- [ ] **Step 6: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Verify by eye**

Run: `cargo run -p sway-app -- --editor --windowed`
Edit `lfoA.beats`, press Save As, choose a path, quit, then relaunch and Open
that file. Expected: the edited value is what loads.

Then press Save (not Save As) and confirm no dialog appears the second time —
`CurrentDocument.path` is set, so it writes straight through. Confirm too that
saving does **not** reload the file underneath you: that is Task 10's
suppression, and this is the first time a real watched file exercises it.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(app): Open, Save and Save As

Masonry supplies no file dialog and no signal variant for one, so sway-app
does, via rfd's async form polled from the redraw loop — the blocking form
spins a nested NSApplication modal on the thread winit's event loop owns.

Deviation from M6-8: the enum is FileRequest and carries no path. A
path-carrying variant is unbuildable by the widget that emits it, since only
sway-app owns rfd; the shell was always going to be both producer and consumer
of the path-carrying form."
```

---

## Phase 5 — The palette

### Task 12: The palette layer widget

**Files:**
- Create: `crates/sway-editor/src/palette.rs`
- Modify: `crates/sway-editor/src/lib.rs`, `crates/sway-editor/src/snapshot.rs`

**Interfaces:**
- Produces: `Palette::new(names: Vec<&'static str>)`, implementing `masonry_core::core::Layer`; `Palette::visible() -> Vec<&'static str>`; `PaletteAction::Picked(&'static str)`.
- Consumes: `WorldSnapshot.palette: Vec<&'static str>` — the registry's names, captured once per frame.

Modelled on `masonry/src/layers/selector_menu.rs` (`impl Layer`,
dismiss-on-click-outside via `capture_pointer_event`, `as_layer` returning
`Some(self)`), which is the only layer in the pinned checkout and therefore the
reference for what a well-formed one looks like.

**Rows are `Button`s, not `SelectorItem`s.** `SelectorItem`'s action type is
`NoAction`, so `SelectorMenu` has to work out which item was clicked by matching
`ctx.target()` inside its own `on_pointer_event` — its source comments call that
fragile ("we might want to find a more robust system"). `Button` submits
`ButtonPress`, and `Widget::on_action` is handed the emitting child's `WidgetId`
directly. Same look, no target matching.

- [ ] **Step 1: Capture the registry names in the snapshot**

In `snapshot.rs`, add `pub palette: Vec<&'static str>` to `WorldSnapshot` and
populate it in `capture`:

```rust
/// Every authorable component name, for the palette. Registration order, which
/// is fixed at startup.
fn capture_palette(world: &World) -> Vec<&'static str> {
    world
        .get_resource::<ComponentDocRegistry>()
        .map(|registry| registry.entries.iter().map(|entry| entry.name).collect())
        .unwrap_or_default()
}
```

`WorldSnapshot` derives `Default`, and `Vec` is `Default`, so nothing else needs
touching.

- [ ] **Step 2: Write the failing tests**

`crates/sway-editor/src/palette.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use masonry::core::{DefaultProperties, PointerButton};
    use masonry_testing::TestHarness;

    fn names() -> Vec<&'static str> {
        vec!["Lfo", "Math", "MeshAsset", "PbrMaterial", "Remap"]
    }

    #[test]
    fn an_empty_filter_lists_everything() {
        let palette = Palette::new(names());
        assert_eq!(palette.visible(), names());
    }

    #[test]
    fn the_filter_is_a_case_insensitive_substring_match() {
        // The rule, stated once: lowercase both sides and ask for `contains`.
        // "ma" is inside "Math" and is *not* inside "MeshAsset" — the letters
        // are there but not adjacent, and this is not a fuzzy matcher.
        let mut palette = Palette::new(names());
        palette.set_filter("ma");
        assert_eq!(palette.visible(), vec!["Math"]);

        // Case-insensitive in both directions.
        palette.set_filter("MESH");
        assert_eq!(palette.visible(), vec!["MeshAsset"]);
    }

    #[test]
    fn the_filter_matches_anywhere_in_the_name_not_just_the_start() {
        let mut palette = Palette::new(names());
        palette.set_filter("material");
        assert_eq!(palette.visible(), vec!["PbrMaterial"]);
    }

    #[test]
    fn a_filter_matching_nothing_lists_nothing() {
        let mut palette = Palette::new(names());
        palette.set_filter("zzz");
        assert!(palette.visible().is_empty());
    }

    #[test]
    fn picking_a_row_emits_that_components_name() {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Palette::new(names()).prepare());
        let row_id = harness.root_widget().row_id(0).expect("five rows are listed");

        harness.mouse_click_on(row_id, Some(PointerButton::Primary));

        assert_eq!(
            harness.pop_action::<PaletteAction>().map(|(action, _)| action),
            Some(PaletteAction::Picked("Lfo")),
        );
    }

    #[test]
    fn picking_addresses_the_filtered_row_not_the_underlying_one() {
        // The defect this guards against: indexing the pick into `names`
        // rather than into `visible()`, so filtering to "Remap" and clicking
        // the only row would create an `Lfo`.
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Palette::new(names()).prepare());
        harness.edit_root_widget(|mut palette| {
            Palette::apply_filter(&mut palette, "remap");
        });
        let row_id = harness.root_widget().row_id(0).expect("one row survives the filter");

        harness.mouse_click_on(row_id, Some(PointerButton::Primary));

        assert_eq!(
            harness.pop_action::<PaletteAction>().map(|(action, _)| action),
            Some(PaletteAction::Picked("Remap")),
        );
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p sway-editor palette::`
Expected: FAIL — the module does not exist.

- [ ] **Step 4: Implement `Palette`**

```rust
//! The component palette: a filterable list of every authorable component,
//! opened by right-clicking the graph canvas. Spec M6, "Palette".
//!
//! A masonry `Layer`, modelled on `masonry::layers::SelectorMenu`: it dismisses
//! itself on a press outside its own border box, which is the behaviour every
//! popup in the pinned checkout has and the one users expect.
//!
//! Knows nothing about the world. It is handed a list of names by
//! `WorldSnapshot.palette` and reports the one that was picked; `GraphCanvas`
//! turns that into an `EditorCommand::Create` (Task 13).

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, EventCtx, Layer, LayoutCtx, MeasureCtx,
    PaintCtx, PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget,
    WidgetId, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::{Button, ButtonPress, TextAction, TextInput};
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

/// Height of the filter box and of each listed row, in logical pixels.
const ROW_HEIGHT: f64 = 22.0;
/// The palette's fixed width.
const WIDTH: f64 = 200.0;
/// At most this many rows are listed; the filter is how you reach the rest.
/// Without a cap, a registry of forty components would open a popup taller
/// than the window.
const MAX_ROWS: usize = 12;

/// A component type was picked from the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    Picked(&'static str),
}

pub struct Palette {
    /// Every authorable name, unfiltered, in registry order.
    names: Vec<&'static str>,
    filter: String,
    input: WidgetPod<TextInput>,
    /// The `TextArea` inside `input` — the child that actually submits the
    /// text actions, per `TextInput`'s own docs.
    input_area: WidgetId,
    /// One button per currently visible name, paired with the name it picks.
    /// Paired rather than re-derived, so a click can never be resolved against
    /// a filter that changed in between.
    rows: Vec<(&'static str, WidgetPod<Button>)>,
}

// --- MARK: BUILDERS
impl Palette {
    pub fn new(names: Vec<&'static str>) -> Self {
        let input = TextInput::new("").with_placeholder("filter…");
        let input_area = input.area_pod().id();
        let mut palette = Self {
            names,
            filter: String::new(),
            input: WidgetPod::new(input),
            input_area,
            rows: Vec::new(),
        };
        palette.rebuild_rows();
        palette
    }
}

// --- MARK: METHODS
impl Palette {
    /// The names matching the current filter, in registry order, capped at
    /// [`MAX_ROWS`].
    ///
    /// Case-insensitive substring, not fuzzy: `"ma"` finds `Math` and not
    /// `MeshAsset`. A fuzzy matcher would be nicer and is not what this is.
    pub fn visible(&self) -> Vec<&'static str> {
        let needle = self.filter.trim().to_lowercase();
        self.names
            .iter()
            .copied()
            .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
            .take(MAX_ROWS)
            .collect()
    }

    /// Sets the filter and rebuilds the row list. Pure state; the widget-tree
    /// side of the same change is [`apply_filter`](Self::apply_filter).
    pub fn set_filter(&mut self, filter: &str) {
        if self.filter == filter {
            return;
        }
        filter.clone_into(&mut self.filter);
        self.rebuild_rows();
    }

    /// The `WidgetId` of the `idx`th visible row, for tests and for the
    /// canvas's own assertions.
    pub fn row_id(&self, idx: usize) -> Option<WidgetId> {
        self.rows.get(idx).map(|(_, pod)| pod.id())
    }

    fn rebuild_rows(&mut self) {
        self.rows = self
            .visible()
            .into_iter()
            .map(|name| (name, WidgetPod::new(Button::with_text(name))))
            .collect();
    }

    fn content_height(&self) -> f64 {
        (self.rows.len() + 1) as f64 * ROW_HEIGHT
    }
}

// --- MARK: WIDGETMUT
impl Palette {
    /// Sets the filter from outside the widget, telling masonry the child set
    /// changed. `set_filter` alone cannot do that — it has no context.
    pub fn apply_filter(this: &mut WidgetMut<'_, Self>, filter: &str) {
        if this.widget.filter == filter {
            return;
        }
        for (_, pod) in std::mem::take(&mut this.widget.rows) {
            this.ctx.remove_child(pod);
        }
        filter.clone_into(&mut this.widget.filter);
        this.widget.rebuild_rows();
        this.ctx.children_changed();
        this.ctx.request_layout();
    }
}

// --- MARK: IMPL WIDGET
impl Widget for Palette {
    type Action = PaletteAction;

    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        // A row was clicked: report the name paired with *that* pod, so the
        // answer cannot drift from what the user saw.
        if action.downcast_ref::<ButtonPress>().is_some()
            && let Some((name, _)) = self.rows.iter().find(|(_, pod)| pod.id() == source)
        {
            ctx.submit_action::<Self::Action>(PaletteAction::Picked(name));
            ctx.set_handled();
            return;
        }

        // The filter box changed. `Changed` (per keystroke) is the right
        // signal here, unlike in the inspector: filtering is free and
        // incremental, and waiting for Enter would make the box feel dead.
        if source == self.input_area
            && let Some(TextAction::Changed(text)) = action.downcast_ref::<TextAction>()
        {
            let text = text.clone();
            let id = ctx.widget_id();
            ctx.mutate_later(id, move |mut palette| {
                let mut palette = palette.downcast::<Self>();
                Self::apply_filter(&mut palette, &text);
            });
            ctx.set_handled();
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.input);
        for (_, pod) in &mut self.rows {
            ctx.register_child(pod);
        }
    }

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
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(WIDTH),
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(self.content_height()),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let row_size = Size::new(size.width, ROW_HEIGHT);
        ctx.run_layout(&mut self.input, row_size);
        ctx.place_child(&mut self.input, Point::ZERO);
        for (index, (_, pod)) in self.rows.iter_mut().enumerate() {
            ctx.run_layout(pod, row_size);
            ctx.place_child(pod, Point::new(0.0, (index + 1) as f64 * ROW_HEIGHT));
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        // An opaque backing, or the canvas and its edges show through the
        // gaps between the children.
        painter.fill_rect(
            Rect::new(0.0, 0.0, WIDTH, self.content_height()),
            Color::from_rgb8(44, 46, 54),
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::ListBox
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        std::iter::once(self.input.id())
            .chain(self.rows.iter().map(|(_, pod)| pod.id()))
            .collect()
    }

    /// Required for `ctx.create_layer` to accept this widget at all — it
    /// `debug_panic!`s when `as_layer` returns `None`.
    fn as_layer(&mut self) -> Option<&mut dyn Layer> {
        Some(self)
    }
}

// --- MARK: IMPL LAYER
impl Layer for Palette {
    /// Dismisses on a press outside the palette, exactly as `SelectorMenu`
    /// does. `capture_pointer_event` sees *every* pointer event in the window,
    /// including ones that never reach this widget's own hit box, which is why
    /// this is the layer hook rather than `on_pointer_event`.
    fn capture_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        let dismiss = match event {
            PointerEvent::Down(PointerButtonEvent { state, .. }) => {
                !ctx.border_box().contains(ctx.local_position(state.position))
            }
            PointerEvent::Cancel(..) => true,
            _ => false,
        };
        if dismiss {
            ctx.remove_layer(ctx.widget_id());
        }
    }
}
```

Add `pub mod palette;` to `crates/sway-editor/src/lib.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-editor palette::`
Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(editor): the component palette layer

A filterable list of ComponentDocRegistry's names, dismissing on click-outside
the way masonry's own SelectorMenu does. Rows are Buttons rather than
SelectorItems, so a pick is resolved from the action's source WidgetId instead
of by matching ctx.target() — the fragility SelectorMenu's own comments flag.

Each row is paired with the name it picks, so a click can never resolve against
a filter that changed in between."
```

---

### Task 13: Create and delete from the canvas

**Files:**
- Modify: `crates/sway-editor/src/canvas.rs`, `crates/sway-editor/src/node_box.rs`

**Interfaces:**
- Consumes: `Palette`, `PaletteAction`, `EditorCommand::{Create, Delete, MoveNode}`, `WorldSnapshot.palette`.
- Produces: right-click opens the palette; a pick sends `Create` at the pointer's canvas position; Delete/Backspace on a selected node sends `Delete`; `NodeBoxAction::DragEnded`, which is what carries a drag into `EditorPos`.

**Two coordinate spaces, kept straight.** `GraphCanvas` carries no transform of
its own but sits inside two `Split`s, so its *local* space is offset from window
space. Child node transforms are `translate(pan) * scale(zoom) * translate(pos)`
and are relative to the canvas's local space. Therefore:

- **canvas space → local:** `local = pos * zoom + pan` (this is `to_visual`).
- **local → canvas space:** `pos = (local - pan) / zoom`.
- **local → window:** `ctx.to_window(local)`, which is what `create_layer` wants.
- `ctx.local_position(state.position)` is how a pointer event gets into local
  space. The existing pan code uses `window_point` *deltas*, which are equal in
  both spaces because no ancestor scales or rotates — a delta is not a position,
  and only positions need converting.

**Name clash.** `canvas.rs` already imports `masonry_core::kurbo::Vec2`.
`EditorCommand` speaks `bevy_math::Vec2`. Import the second one aliased —
`use bevy_math::Vec2 as WorldVec2;` — rather than qualifying at each use site.

- [ ] **Step 1: Write the failing tests**

Add to `crates/sway-editor/src/canvas.rs`'s test module. Its existing `node()`
helper and `snapshot()` builder are reused; `harness_with` now needs to hand
back the receiver:

```rust
    fn harness_and_rx(
        snap: WorldSnapshot,
    ) -> (TestHarness<GraphCanvas>, crossbeam_channel::Receiver<EditorCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), GraphCanvas::new(tx).prepare());
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::apply_snapshot(&mut canvas, &snap);
        });
        (harness, rx)
    }

    #[test]
    fn picking_from_the_palette_creates_at_the_canvas_position() {
        let (mut harness, rx) = harness_and_rx(snapshot(vec![], vec![]));

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::palette_picked_for_test(&mut canvas, "Lfo", Point::new(120.0, 60.0));
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::Create { component: "Lfo", pos }
                    if *pos == WorldVec2::new(120.0, 60.0)
            ),
            "got {:?}",
            commands[0],
        );
    }

    #[test]
    fn a_right_click_opens_the_palette_at_the_pointer() {
        let mut snap = snapshot(vec![], vec![]);
        snap.palette = vec!["Lfo", "Remap"];
        let (mut harness, _rx) = harness_and_rx(snap);

        harness.mouse_move(Point::new(200.0, 150.0));
        harness.mouse_button_press(Some(PointerButton::Secondary));

        // The harness services NewLayer/RemoveLayer signals itself (see
        // `masonry_testing::TestHarness::process_signals`), so a layer that was
        // asked for is a layer that exists.
        assert!(
            harness.root_widget().palette_layer_id().is_some(),
            "a secondary press opens the palette",
        );
    }

    #[test]
    fn the_delete_key_deletes_the_selected_node() {
        let entity = Entity::from_raw_u32(4).expect("valid entity id");
        let (mut harness, rx) = harness_and_rx(snapshot(
            vec![NodeView { entity, ..node(4, "a", Some(Point::new(10.0, 10.0))) }],
            vec![],
        ));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::set_selected(&mut canvas, Some(NodeId(4)));
            GraphCanvas::delete_selected_for_test(&mut canvas);
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Delete { entity }],
        );
    }

    #[test]
    fn deleting_with_nothing_selected_sends_nothing() {
        let (mut harness, rx) = harness_and_rx(snapshot(vec![], vec![]));
        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::delete_selected_for_test(&mut canvas);
        });
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn ending_a_drag_reports_the_nodes_new_canvas_position() {
        // MoveNode is what carries a drag into EditorPos and therefore into
        // the saved document. Before M6 a dragged position lived only in the
        // widget and was lost on exit.
        let entity = Entity::from_raw_u32(4).expect("valid entity id");
        let (mut harness, rx) = harness_and_rx(snapshot(
            vec![NodeView { entity, ..node(4, "a", Some(Point::new(100.0, 100.0))) }],
            vec![],
        ));

        harness.mouse_move(Point::new(150.0, 130.0));
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_move(Point::new(200.0, 180.0));
        harness.mouse_button_release(Some(PointerButton::Primary));

        let moved = harness.root_widget().position_of(NodeId(4)).unwrap();
        let commands: Vec<_> = rx.try_iter().collect();
        assert!(
            commands.iter().any(|c| matches!(
                c,
                EditorCommand::MoveNode { entity: e, pos }
                    if *e == entity
                        && *pos == WorldVec2::new(moved.x as f32, moved.y as f32)
            )),
            "the released position must be the one sent; got {commands:?}",
        );
    }
```

The test module's imports grow by `crate::snapshot::NodeView`,
`sway_graph::EditorCommand`, and `bevy_math::Vec2 as WorldVec2`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-editor canvas::`
Expected: FAIL — no `palette_picked_for_test`, `palette_layer_id`,
`delete_selected_for_test`, and no `MoveNode` is ever sent.

- [ ] **Step 3: Report the end of a drag from `NodeBox`**

`NodeBox` captures the pointer on its own `Down`, so the matching `Up` is
delivered to *it*, not to `GraphCanvas` — the canvas cannot see a drag end
without being told. Add a variant to `NodeBoxAction` in `node_box.rs`:

```rust
pub enum NodeBoxAction {
    /// This node was pressed: the canvas should select it.
    Selected,
    /// The pointer moved by this delta while dragging the node.
    DraggedBy(masonry_core::kurbo::Vec2),
    /// The drag finished. The canvas writes the node's settled position back
    /// to the world; a press with no movement reports this too, and the
    /// world-side equal-value guard makes that a no-op.
    DragEnded,
}
```

and submit it where the gesture ends:

```rust
            PointerEvent::Up(..) => {
                if matches!(self.gesture, Gesture::Dragging { .. }) {
                    ctx.submit_action::<Self::Action>(NodeBoxAction::DragEnded);
                }
                self.gesture = Gesture::None;
                ctx.set_handled();
            }
```

`Cancel` deliberately does not report: a cancelled gesture should leave the
world alone.

- [ ] **Step 4: Implement the canvas side**

`GraphCanvas` gains four fields:

```rust
pub struct GraphCanvas {
    // … existing fields unchanged …
    /// Where edits go. The canvas produces data; `sway-graph` applies it.
    commands: Sender<EditorCommand>,
    /// Every authorable component name, from the last snapshot — what the
    /// palette is built from.
    palette: Vec<&'static str>,
    /// The open palette layer, if any, and the canvas-space position it was
    /// opened at (which is where a pick creates the node).
    palette_layer: Option<(WidgetId, Point)>,
}
```

`new` takes the sender (already added in Task 7) and initialises the rest to
empty/`None`. `apply_snapshot` records the names, near the top:

```rust
        this.widget.palette.clone_from(&snap.palette);
```

The pointer handler gains a `Secondary` arm. It must come **before** the
existing catch-all `PointerEvent::Down(..)` arm, which treats any other press as
a background click:

```rust
            PointerEvent::Down(PointerButtonEvent {
                button: Some(PointerButton::Secondary),
                state,
                ..
            }) => {
                let local = ctx.local_position(state.position);
                let canvas_pos = self.to_canvas(local);
                let palette = NewWidget::new(Palette::new(self.palette.clone()));
                self.palette_layer = Some((palette.id(), canvas_pos));
                ctx.create_layer(LayerType::Other, palette, ctx.to_window(local));
                ctx.set_handled();
            }
```

`create_layer` only *emits* `NewLayer`; the layer exists once the host services
it, which is what Task 7 built and what `TestHarness` does for itself.

`on_action` grows two arms. It currently returns early when the source is not
one of its node boxes, so the palette arm has to be handled before that lookup:

```rust
    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        if let Some(PaletteAction::Picked(component)) = action.downcast_ref::<PaletteAction>() {
            // The position the palette was *opened* at, not where the pick
            // landed: the node belongs where the user pointed, not where the
            // list happened to place that row.
            if let Some((layer_id, pos)) = self.palette_layer.take() {
                let _ = self.commands.send(EditorCommand::Create {
                    component,
                    pos: WorldVec2::new(pos.x as f32, pos.y as f32),
                });
                ctx.remove_layer(layer_id);
            }
            ctx.set_handled();
            return;
        }
        let _ = source; // … existing NodeBox lookup and match follow …
```

and the node-box match gains the drag-end arm:

```rust
            NodeBoxAction::DragEnded => {
                if let Some(slot) = self.slots.get(&id) {
                    let _ = self.commands.send(EditorCommand::MoveNode {
                        entity: slot.entity,
                        pos: WorldVec2::new(slot.pos.x as f32, slot.pos.y as f32),
                    });
                }
            }
```

Deletion is a text event, since it is a key:

```rust
    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        let TextEvent::Keyboard(key_event) = event else {
            return;
        };
        if !key_event.state.is_down() {
            return;
        }
        if matches!(
            key_event.key,
            Key::Named(NamedKey::Delete) | Key::Named(NamedKey::Backspace)
        ) {
            self.delete_selected();
            ctx.set_handled();
        }
    }
```

with the helpers and the two test seams:

```rust
impl GraphCanvas {
    /// Maps a point in this widget's local space to canvas space — the
    /// inverse of `to_visual`.
    fn to_canvas(&self, local: Point) -> Point {
        ((local.to_vec2() - self.pan) / self.zoom).to_point()
    }

    /// Sends `Delete` for the selected node, if there is one. The canvas does
    /// not remove the node itself: the world is the truth, and the next
    /// snapshot is what takes the box away.
    fn delete_selected(&mut self) {
        let Some(entity) = self.selected.and_then(|id| self.entity_of(id)) else {
            return;
        };
        let _ = self.commands.send(EditorCommand::Delete { entity });
    }

    /// The open palette layer's id, for tests.
    pub fn palette_layer_id(&self) -> Option<WidgetId> {
        self.palette_layer.map(|(id, _)| id)
    }

    /// Test seam for the palette pick, which otherwise needs a live layer.
    pub fn palette_picked_for_test(
        this: &mut WidgetMut<'_, Self>,
        component: &'static str,
        pos: Point,
    ) {
        let _ = this.commands_send(EditorCommand::Create {
            component,
            pos: WorldVec2::new(pos.x as f32, pos.y as f32),
        });
    }

    /// Test seam for the Delete key.
    pub fn delete_selected_for_test(this: &mut WidgetMut<'_, Self>) {
        this.widget.delete_selected();
    }
}
```

`commands_send` is not a real method — write `this.widget.commands.send(…)`
directly; the field is private to the module and the test seam lives beside it.

New imports for `canvas.rs`: `bevy_math::Vec2 as WorldVec2`,
`crossbeam_channel::Sender`, `masonry::core::{LayerType, NewWidget, TextEvent}`,
`masonry::core::keyboard::{Key, NamedKey}`, `sway_graph::EditorCommand`, and
`crate::palette::{Palette, PaletteAction}`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Verify by eye**

Run: `cargo run -p sway-app -- --editor --windowed`
Right-click the canvas, type `lfo` in the filter, pick `Lfo`. Expected: a new
node box appears under the pointer and a new row appears in the tree. Drag it
somewhere, select it, press Delete. Expected: both disappear.

Then Save, quit, relaunch and Open: the node must come back where you dragged
it. That is `MoveNode` → `EditorPos` → the document, end to end.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(editor): create and delete nodes from the canvas

Right-click opens the palette at the pointer; a pick creates the node at the
position the palette was opened at, so EditorPos is set by the gesture rather
than by a fallback grid.

NodeBox gains DragEnded: it captures the pointer, so the canvas never saw the
Up that ends a drag. Dragging a node now writes back through MoveNode and
therefore into the saved document, which it never did before."
```

---

## Phase 6 — Drag-to-connect

### Task 14: Socket hit-testing and the edge drag

**Files:**
- Modify: `crates/sway-editor/src/canvas.rs`, `crates/sway-editor/src/node_box.rs`

**Interfaces:**
- Produces: `SocketKind { Outlet, Inlet(u16) }`; `SocketRef { node: NodeId, kind: SocketKind }`; `GraphCanvas::socket_at(Point) -> Option<SocketRef>` (canvas space); `NodeBoxAction::{SocketPressed, ConnectDragged, ConnectReleased}`; a rubber-band edge painted while dragging.

**Where the press is detected, and why it is not on the canvas.** Masonry
hit-tests children before parents — deepest hit wins — and `NodeBox` marks every
primary `Down` handled. A press on a socket therefore reaches the `NodeBox` and
never reaches `GraphCanvas` at all, so `GraphCanvas` cannot start an edge drag
from its own `on_pointer_event` no matter what its geometry says. The press-side
hit test lives in `NodeBox`, which already owns its socket geometry and already
reports gestures upward (`Selected`, `DraggedBy`) under the same rule.

The *release* is different: it lands over whichever node the pointer ended up
on, which is not the node that captured the pointer. That one the canvas
resolves globally, which is what `socket_at` is for.

**Positions are reported in the pressed box's own local space.** `NodeBox`'s
module doc explains why deltas are reported in window space; a *position* is
different. `ctx.local_position` inside `NodeBox` already divides out pan and
zoom (they are in that widget's own transform), and the canvas knows where that
box sits, so `canvas_point = slot.pos + reported.to_vec2()` — no window-to-local
conversion anywhere, and no second copy of the pan/zoom mapping.

- [ ] **Step 1: Write the failing tests**

Add to `crates/sway-editor/src/canvas.rs`'s test module:

```rust
    /// A node at a known canvas position with two inlets and one outlet, so
    /// every socket has a distinct, computable position.
    fn socket_node(pos: Point) -> NodeView {
        use crate::snapshot::InletView;
        NodeView {
            inlets: vec![
                InletView { wire: "amount", connected: false },
                InletView { wire: "parent", connected: false },
            ],
            outlets: 1,
            ..node(0, "n", Some(pos))
        }
    }

    #[test]
    fn a_point_on_the_outlet_socket_hits_it() {
        let origin = Point::new(40.0, 25.0);
        let (harness, _rx) = harness_and_rx(snapshot(vec![socket_node(origin)], vec![]));

        // `outlet_socket_local(inlet_field_count, outlets, field)` is the same
        // math `paint` uses, so the probe cannot drift from what is drawn.
        let local = node_box::outlet_socket_local(2, 1, 2);
        let probe = origin + local.to_vec2();

        assert_eq!(
            harness.root_widget().socket_at(probe),
            Some(SocketRef { node: NodeId(0), kind: SocketKind::Outlet }),
        );
    }

    #[test]
    fn each_inlet_socket_reports_its_own_ordinal() {
        // Two inlets must not both resolve to ordinal 0 — the same class of
        // defect as the hardcoded to_field M6-6 fixes.
        let origin = Point::new(40.0, 25.0);
        let (harness, _rx) = harness_and_rx(snapshot(vec![socket_node(origin)], vec![]));

        let first = origin + node_box::inlet_socket_local(&[1, 1], 0, 0).to_vec2();
        let second = origin + node_box::inlet_socket_local(&[1, 1], 1, 0).to_vec2();
        assert_ne!(first, second, "the two sockets must be at different heights");

        assert_eq!(
            harness.root_widget().socket_at(first),
            Some(SocketRef { node: NodeId(0), kind: SocketKind::Inlet(0) }),
        );
        assert_eq!(
            harness.root_widget().socket_at(second),
            Some(SocketRef { node: NodeId(0), kind: SocketKind::Inlet(1) }),
        );
    }

    #[test]
    fn a_point_between_sockets_hits_nothing() {
        let origin = Point::new(40.0, 25.0);
        let (harness, _rx) = harness_and_rx(snapshot(vec![socket_node(origin)], vec![]));

        // The middle of the box: no socket is anywhere near it.
        let middle = origin + kurbo::Vec2::new(node_box::SIZE.width / 2.0, node_box::SIZE.height / 2.0);
        assert_eq!(harness.root_widget().socket_at(middle), None);
    }

    #[test]
    fn a_point_far_from_every_node_hits_nothing() {
        let (harness, _rx) = harness_and_rx(snapshot(vec![socket_node(Point::ZERO)], vec![]));
        assert_eq!(harness.root_widget().socket_at(Point::new(-500.0, -500.0)), None);
    }

    #[test]
    fn pressing_an_outlet_starts_a_drag_and_releasing_clears_it() {
        let origin = Point::new(40.0, 25.0);
        let (mut harness, _rx) = harness_and_rx(snapshot(vec![socket_node(origin)], vec![]));

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::socket_pressed_for_test(
                &mut canvas,
                SocketRef { node: NodeId(0), kind: SocketKind::Outlet },
            );
        });
        assert!(harness.root_widget().edge_drag_origin().is_some());

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::connect_released_for_test(&mut canvas, Point::new(-500.0, -500.0));
        });
        assert!(
            harness.root_widget().edge_drag_origin().is_none(),
            "releasing over empty canvas cancels the drag",
        );
    }
```

`node_box::SIZE`, `inlet_socket_local` and `outlet_socket_local` are already
`pub(crate)`, so the test module reaches them through `crate::node_box`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-editor canvas::`
Expected: FAIL — `socket_at`, `SocketRef`, `SocketKind` undefined.

- [ ] **Step 3: Add the types and the hit test**

In `canvas.rs`:

```rust
/// Which socket on a node. An outlet needs no ordinal — there is at most one
/// (M6-6) — while an inlet's ordinal is its wire's position in the node's
/// inlet list, which is `WireRegistry` order and fixed at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketKind {
    Outlet,
    Inlet(u16),
}

/// One socket, addressed across the whole canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketRef {
    pub node: NodeId,
    pub kind: SocketKind,
}

/// An edge drag in progress.
struct EdgeDrag {
    from: SocketRef,
    /// Where the pointer is now, in canvas space. Painted as the loose end of
    /// the rubber band.
    cursor: Point,
}

/// How close a probe must be to a socket to count as hitting it, in
/// canvas-space pixels. Deliberately larger than the 4px dot `NodeBox` draws:
/// an exact-radius target is unhittable in practice.
const SOCKET_HIT_RADIUS: f64 = node_box::SOCKET_RADIUS * 2.5;
```

`SOCKET_RADIUS` is currently private to `node_box.rs`; make it `pub(crate)` so
both modules agree on one number.

```rust
impl GraphCanvas {
    /// The socket at a canvas-space point, if any.
    ///
    /// Uses the same `inlet_socket_local`/`outlet_socket_local` math `paint`
    /// does, against the same mirrored `NodeSlot` counts, so what is hittable
    /// is exactly what is drawn.
    pub fn socket_at(&self, point: Point) -> Option<SocketRef> {
        for id in &self.nodes {
            let Some(slot) = self.slots.get(id) else {
                continue;
            };
            let local = point - slot.pos.to_vec2();

            if slot.outlets > 0 {
                let outlet = node_box::outlet_socket_local(
                    slot.inlets.len() as u16,
                    slot.outlets,
                    slot.inlets.len() as u16,
                );
                if outlet.distance(local) <= SOCKET_HIT_RADIUS {
                    return Some(SocketRef { node: *id, kind: SocketKind::Outlet });
                }
            }

            for ordinal in 0..slot.inlets.len() as u16 {
                let inlet = node_box::inlet_socket_local(&slot.inlets, ordinal, 0);
                if inlet.distance(local) <= SOCKET_HIT_RADIUS {
                    return Some(SocketRef { node: *id, kind: SocketKind::Inlet(ordinal) });
                }
            }
        }
        None
    }

    /// The socket an edge drag started from, if one is in progress.
    pub fn edge_drag_origin(&self) -> Option<SocketRef> {
        self.drag.as_ref().map(|drag| drag.from)
    }
}
```

`GraphCanvas` gains `drag: Option<EdgeDrag>`, initialised to `None` in `new`.
Note `outlet_socket_local`'s `field` argument is the node's *flat* ordinal with
inlets first, which is why the third argument above is `slot.inlets.len()`.

- [ ] **Step 4: Report socket gestures from `NodeBox`**

Three more variants on `NodeBoxAction`, and a second `Gesture`:

```rust
pub enum NodeBoxAction {
    Selected,
    DraggedBy(masonry_core::kurbo::Vec2),
    DragEnded,
    /// A press landed on one of this box's sockets. Positions in the two
    /// variants below are in this box's own local space; the canvas adds the
    /// box's canvas position to get canvas space (see the task preamble).
    SocketPressed(SocketKind),
    /// The pointer moved while dragging from a socket.
    ConnectDragged(Point),
    /// The socket drag ended here.
    ConnectReleased(Point),
}
```

```rust
enum Gesture {
    None,
    Dragging { last_window: Point },
    /// Dragging an edge out of one of this box's sockets.
    Connecting,
}
```

The `Down` arm hit-tests its own sockets before falling through to the existing
move-the-node gesture:

```rust
            PointerEvent::Down(PointerButtonEvent { button, state, .. }) => {
                if *button != Some(PointerButton::Primary) {
                    return;
                }
                ctx.capture_pointer();
                let local = ctx.local_position(state.position);
                if let Some(kind) = self.socket_at_local(local) {
                    self.gesture = Gesture::Connecting;
                    ctx.submit_action::<Self::Action>(NodeBoxAction::SocketPressed(kind));
                } else {
                    self.gesture = Gesture::Dragging { last_window: window_point(state) };
                    ctx.submit_action::<Self::Action>(NodeBoxAction::Selected);
                }
                ctx.set_handled();
            }
```

`Move` and `Up` gain a `Connecting` case each:

```rust
            PointerEvent::Move(PointerUpdate { current, .. }) if ctx.is_active() => {
                match &mut self.gesture {
                    Gesture::Dragging { last_window } => {
                        let window = window_point(current);
                        let delta = window - *last_window;
                        *last_window = window;
                        ctx.submit_action::<Self::Action>(NodeBoxAction::DraggedBy(delta));
                    }
                    Gesture::Connecting => {
                        let local = ctx.local_position(current.position);
                        ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectDragged(local));
                    }
                    Gesture::None => {}
                }
                ctx.set_handled();
            }
            PointerEvent::Up(PointerButtonEvent { state, .. }) => {
                match self.gesture {
                    Gesture::Dragging { .. } => {
                        ctx.submit_action::<Self::Action>(NodeBoxAction::DragEnded);
                    }
                    Gesture::Connecting => {
                        let local = ctx.local_position(state.position);
                        ctx.submit_action::<Self::Action>(NodeBoxAction::ConnectReleased(local));
                    }
                    Gesture::None => {}
                }
                self.gesture = Gesture::None;
                ctx.set_handled();
            }
```

and the box's own socket hit test, the mirror of the canvas's:

```rust
impl NodeBox {
    /// Which of this box's sockets a local-space point is on, if any. Same
    /// radius and same geometry the canvas uses.
    fn socket_at_local(&self, local: Point) -> Option<SocketKind> {
        let inlet_fields = self.inlets.len() as u16;
        if self.outlets > 0 {
            let outlet = outlet_socket_local(inlet_fields, self.outlets, inlet_fields);
            if outlet.distance(local) <= SOCKET_RADIUS * 2.5 {
                return Some(SocketKind::Outlet);
            }
        }
        for ordinal in 0..inlet_fields {
            if inlet_socket_local(&self.inlets, ordinal, 0).distance(local) <= SOCKET_RADIUS * 2.5 {
                return Some(SocketKind::Inlet(ordinal));
            }
        }
        None
    }
}
```

`node_box.rs` imports `crate::canvas::SocketKind`. The `2.5` factor is written
out here rather than importing `canvas::SOCKET_HIT_RADIUS`, to keep the
dependency one-way (`node_box` → `canvas` for the type only); if that ever
drifts, move the constant into `node_box.rs` beside `SOCKET_RADIUS` and have
`canvas.rs` import it instead.

Delete the note in `NodeBoxAction`'s doc comment that says drag-to-connect is
deliberately absent and arrives at M7 — it arrives here.

- [ ] **Step 5: Track the drag on the canvas and paint the rubber band**

`GraphCanvas::on_action`'s node-box match gains three arms, delegating to plain
methods so Task 15 and the tests share one path:

```rust
            NodeBoxAction::SocketPressed(kind) => self.socket_pressed(ctx, id, kind),
            NodeBoxAction::ConnectDragged(local) => {
                if let Some(slot) = self.slots.get(&id) {
                    let cursor = slot.pos + local.to_vec2();
                    if let Some(drag) = &mut self.drag {
                        drag.cursor = cursor;
                    }
                }
                ctx.request_paint_only();
            }
            NodeBoxAction::ConnectReleased(local) => {
                let point = self.slots.get(&id).map(|slot| slot.pos + local.to_vec2());
                if let Some(point) = point {
                    self.connect_released(point);
                }
                ctx.request_paint_only();
            }
```

```rust
impl GraphCanvas {
    /// A socket was pressed. An outlet starts an edge drag; an inlet is
    /// Task 15's disconnect gesture, and does nothing yet.
    fn socket_pressed(&mut self, _ctx: &mut ActionCtx<'_>, node: NodeId, kind: SocketKind) {
        if kind != SocketKind::Outlet {
            return;
        }
        let cursor = self.slots.get(&node).map(|slot| slot.pos).unwrap_or_default();
        self.drag = Some(EdgeDrag { from: SocketRef { node, kind }, cursor });
    }

    /// The edge drag ended at this canvas-space point. Task 15 turns a landing
    /// on a legal inlet into a `Connect`; for now every release just cancels.
    fn connect_released(&mut self, _point: Point) {
        self.drag = None;
    }

    /// Test seam for a socket press.
    pub fn socket_pressed_for_test(this: &mut WidgetMut<'_, Self>, socket: SocketRef) {
        let cursor = this
            .widget
            .slots
            .get(&socket.node)
            .map(|slot| slot.pos)
            .unwrap_or_default();
        if socket.kind == SocketKind::Outlet {
            this.widget.drag = Some(EdgeDrag { from: socket, cursor });
        }
    }

    /// Test seam for the release.
    pub fn connect_released_for_test(this: &mut WidgetMut<'_, Self>, point: Point) {
        this.widget.connect_released(point);
    }
}
```

`paint` draws the rubber band after the settled edges, so it sits on top of
them and under the node boxes:

```rust
        if let Some(drag) = &self.drag
            && let Some(slot) = self.slots.get(&drag.from.node)
        {
            let from_local = node_box::outlet_socket_local(
                slot.inlets.len() as u16,
                slot.outlets,
                slot.inlets.len() as u16,
            );
            let from = self.to_visual(slot.pos + from_local.to_vec2());
            let to = self.to_visual(drag.cursor);
            self.paint_edge(painter, from, to, Color::from_rgb8(220, 220, 230), 2.0);
        }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-editor canvas::`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(editor): socket hit-testing and the rubber-band edge drag

The press-side hit test lives in NodeBox, not GraphCanvas: masonry hit-tests
children before parents and NodeBox marks every primary Down handled, so a
press on a socket never reaches the canvas. The release is resolved
canvas-side, because it lands over a different node than the one that captured
the pointer — which is exactly what socket_at is for."
```

---

### Task 15: Legality highlighting, Connect and Disconnect

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`, `crates/sway-editor/src/canvas.rs`

**Interfaces:**
- Consumes: `SocketRef`, `SocketKind`, `EdgeDrag`, `EditorCommand::{Connect, Disconnect}`.
- Produces: `InletView.accepts_from: Vec<Entity>`.

Legality needs `has_source(src)` and `has_target(dst)`, which are world-side
predicates. Rather than call them from a widget, the snapshot carries the answer
per inlet:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct InletView {
    pub wire: &'static str,
    pub connected: bool,
    /// Entities this inlet could legally accept a wire from, this frame.
    pub accepts_from: Vec<Entity>,
}
```

`InletView` loses `Copy` (it now owns a `Vec`), so the `Copy` in Task 6's derive
must go; `NodeView`'s own derive is already `Clone, Debug, PartialEq` and needs
no change.

**Every existing `InletView` literal gains the third field.** Adding a field to
a public struct breaks every construction site, and there are three by now:
Task 6's snapshot tests, Task 6 Step 7's rewritten
`a_node_box_lays_out_one_socket_per_inlet`, and Task 14's `socket_node` helper.
All three want `accepts_from: Vec::new()` — none of them is testing legality.
Do this first, before Step 1's new tests, or the crate will not compile far
enough to run them.

- [ ] **Step 1: Write the failing tests**

In `snapshot.rs`'s test module:

```rust
    #[test]
    fn an_inlet_accepts_only_entities_with_the_wires_source_component() {
        let mut app = app();
        let emit = spawn_emit(app.world_mut(), 1, None);
        let recv = spawn_recv(app.world_mut(), 2, None);
        let unrelated = spawn_spatial(app.world_mut(), 3, None);
        recompile(&mut app);

        let snapshot = capture(app.world());
        let node = snapshot.nodes.iter().find(|n| n.entity == recv).unwrap();
        let amount = node.inlets.iter().find(|i| i.wire == "amount").unwrap();

        assert!(amount.accepts_from.contains(&emit), "Emit carries FloatOut");
        assert!(
            !amount.accepts_from.contains(&unrelated),
            "a Transform-only entity cannot source an amount wire",
        );
    }

    #[test]
    fn an_inlet_never_offers_the_node_itself() {
        // Bevy drops a self-relationship anyway (tests/relationship_semantics.rs),
        // but the editor must not paint one as legal in the first place.
        let mut app = app();
        let both = spawn_double_target(app.world_mut(), None);
        // Give it a source too, so it would otherwise qualify for its own inlet.
        app.world_mut().entity_mut(both).insert(bevy_transform::components::Transform::default());
        recompile(&mut app);

        let snapshot = capture(app.world());
        let node = snapshot.nodes.iter().find(|n| n.entity == both).unwrap();
        let parent = node.inlets.iter().find(|i| i.wire == "parent").unwrap();

        assert!(!parent.accepts_from.contains(&both));
    }
```

In `canvas.rs`'s test module:

```rust
    /// A source node and a target node whose `amount` inlet accepts it.
    fn connectable() -> (WorldSnapshot, Entity, Entity) {
        use crate::snapshot::InletView;
        let src = Entity::from_raw_u32(1).expect("valid entity id");
        let dst = Entity::from_raw_u32(2).expect("valid entity id");
        let source = NodeView {
            entity: src,
            outlets: 1,
            ..node(1, "src", Some(Point::new(0.0, 0.0)))
        };
        let target = NodeView {
            entity: dst,
            inlets: vec![InletView {
                wire: "amount",
                connected: false,
                accepts_from: vec![src],
            }],
            outlets: 0,
            ..node(2, "dst", Some(Point::new(400.0, 0.0)))
        };
        (snapshot(vec![source, target], vec![]), src, dst)
    }

    /// The canvas-space position of the `dst` node's first inlet socket.
    fn first_inlet_point(origin: Point) -> Point {
        origin + node_box::inlet_socket_local(&[1], 0, 0).to_vec2()
    }

    #[test]
    fn releasing_on_a_legal_inlet_sends_connect() {
        let (snap, src, dst) = connectable();
        let (mut harness, rx) = harness_and_rx(snap);

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::socket_pressed_for_test(
                &mut canvas,
                SocketRef { node: NodeId(1), kind: SocketKind::Outlet },
            );
            GraphCanvas::connect_released_for_test(
                &mut canvas,
                first_inlet_point(Point::new(400.0, 0.0)),
            );
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Connect { wire: "amount", src, dst }],
        );
    }

    #[test]
    fn releasing_on_an_illegal_inlet_sends_nothing() {
        let (mut snap, _src, _dst) = connectable();
        snap.nodes[1].inlets[0].accepts_from.clear();
        let (mut harness, rx) = harness_and_rx(snap);

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::socket_pressed_for_test(
                &mut canvas,
                SocketRef { node: NodeId(1), kind: SocketKind::Outlet },
            );
            GraphCanvas::connect_released_for_test(
                &mut canvas,
                first_inlet_point(Point::new(400.0, 0.0)),
            );
        });

        assert_eq!(rx.try_iter().count(), 0);
        assert!(
            harness.root_widget().edge_drag_origin().is_none(),
            "an illegal release still ends the drag",
        );
    }

    #[test]
    fn pressing_a_connected_inlet_sends_disconnect() {
        let (mut snap, _src, dst) = connectable();
        snap.nodes[1].inlets[0].connected = true;
        let (mut harness, rx) = harness_and_rx(snap);

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::socket_pressed_for_test(
                &mut canvas,
                SocketRef { node: NodeId(2), kind: SocketKind::Inlet(0) },
            );
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Disconnect { wire: "amount", dst }],
        );
    }

    #[test]
    fn pressing_an_unconnected_inlet_sends_nothing() {
        let (snap, _src, _dst) = connectable();
        let (mut harness, rx) = harness_and_rx(snap);

        harness.edit_root_widget(|mut canvas| {
            GraphCanvas::socket_pressed_for_test(
                &mut canvas,
                SocketRef { node: NodeId(2), kind: SocketKind::Inlet(0) },
            );
        });

        assert_eq!(rx.try_iter().count(), 0);
    }
```

`socket_pressed_for_test` must now route through the real `socket_pressed`
rather than setting `drag` by hand — rewrite it as:

```rust
    pub fn socket_pressed_for_test(this: &mut WidgetMut<'_, Self>, socket: SocketRef) {
        this.widget.socket_pressed(socket.node, socket.kind);
    }
```

and drop the `_ctx` parameter from `socket_pressed` (nothing in it needed a
context; `on_action` calls `ctx.request_paint_only()` around it instead).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p sway-editor`
Expected: FAIL — `InletView` has no `accepts_from`, and no command is sent.

- [ ] **Step 3: Populate `accepts_from`**

`inlets_of` gains the canvas entity list, computed once by the caller and passed
in — one pass over the node set, not one per inlet:

```rust
fn inlets_of(
    world: &World,
    registry: &WireRegistry,
    canvas: &[Entity],
    entity: Entity,
) -> Vec<InletView> {
    registry
        .entries
        .iter()
        .filter(|entry| (entry.has_target)(world, entity))
        .map(|entry| InletView {
            wire: entry.name,
            connected: (entry.read)(world, entity).is_some(),
            // Excluding `entity` itself keeps a self-edge from ever being
            // offered; Bevy would drop it anyway (tests/relationship_semantics.rs),
            // but the editor should not paint it as legal.
            accepts_from: canvas
                .iter()
                .copied()
                .filter(|candidate| *candidate != entity && (entry.has_source)(world, *candidate))
                .collect(),
        })
        .collect()
}
```

`capture_nodes` computes `canvas_entities(world)` once and passes it to both
its own loop and to `inlets_of`. `capture_edges` also calls `inlets_of` (for the
`to_field` ordinal, Task 6); it only needs the wire names, so pass an empty
slice there rather than recomputing the candidate list per edge:

```rust
                let to_field = inlets_of(world, registry, &[], dst)
                    .iter()
                    .position(|inlet| inlet.wire == wire)
                    .unwrap_or(0) as u16;
```

- [ ] **Step 4: Emit `Connect` and `Disconnect`**

```rust
    /// A socket was pressed. An outlet starts an edge drag; a *connected*
    /// inlet is the disconnect gesture.
    fn socket_pressed(&mut self, node: NodeId, kind: SocketKind) {
        match kind {
            SocketKind::Outlet => {
                let cursor = self.slots.get(&node).map(|slot| slot.pos).unwrap_or_default();
                self.drag = Some(EdgeDrag { from: SocketRef { node, kind }, cursor });
            }
            SocketKind::Inlet(ordinal) => {
                let Some(slot) = self.slots.get(&node) else { return };
                let Some(inlet) = slot.inlet_views.get(ordinal as usize) else { return };
                if !inlet.connected {
                    return;
                }
                let _ = self.commands.send(EditorCommand::Disconnect {
                    wire: inlet.wire,
                    dst: slot.entity,
                });
            }
        }
    }

    /// The edge drag ended at this canvas-space point. A landing on an inlet
    /// that accepts the drag's origin connects; anything else cancels.
    fn connect_released(&mut self, point: Point) {
        let Some(drag) = self.drag.take() else { return };
        let Some(src) = self.entity_of(drag.from.node) else { return };
        let Some(SocketRef { node, kind: SocketKind::Inlet(ordinal) }) = self.socket_at(point)
        else {
            return; // released over empty canvas or over an outlet
        };
        let Some(slot) = self.slots.get(&node) else { return };
        let Some(inlet) = slot.inlet_views.get(ordinal as usize) else { return };
        // Legality is re-checked here even though `paint` only highlighted
        // legal inlets: the highlight is a hint, this is the rule.
        if !inlet.accepts_from.contains(&src) {
            return;
        }
        let _ = self.commands.send(EditorCommand::Connect {
            wire: inlet.wire,
            src,
            dst: slot.entity,
        });
    }
```

This needs the inlet *views*, not just the per-field slot counts, so `NodeSlot`
gains one field beside the `inlets: Vec<u16>` it already mirrors:

```rust
    /// The snapshot's inlet views for this node — wire names, connectedness
    /// and legality. Mirrored for the same reason `inlets` is: `paint` and
    /// `on_action` cannot read a live child's state.
    inlet_views: Vec<InletView>,
```

kept in sync in `apply_snapshot` alongside `inlets`, in both the create and the
update branch. Because `InletView` is no longer `Copy`, the update branch's
existing guard becomes:

```rust
                    if slot.inlet_views != view.inlets || slot.outlets != view.outlets {
                        slot.inlets = inlet_counts.clone();
                        slot.inlet_views = view.inlets.clone();
                        slot.outlets = view.outlets;
                        let mut child = this.ctx.get_mut(&mut slot.pod);
                        NodeBox::set_sockets(&mut child, inlet_counts, view.outlets);
                    }
```

- [ ] **Step 5: Highlight the legal targets while dragging**

`NodeBox` paints its own sockets, and it does not know about legality, so the
highlight is painted by the canvas *over* the boxes. That means `post_paint`,
not `paint` — `paint` runs before children (which is what puts edges behind the
boxes) and a highlight drawn there would be covered by the box it marks:

```rust
    /// Draws the legality overlay while an edge drag is in progress. Runs
    /// after children, so the marks sit on top of the node boxes; the edges in
    /// `paint` deliberately sit underneath them.
    fn post_paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let Some(drag) = &self.drag else { return };
        let Some(src) = self.entity_of(drag.from.node) else { return };

        for id in &self.nodes {
            let Some(slot) = self.slots.get(id) else { continue };
            for (ordinal, inlet) in slot.inlet_views.iter().enumerate() {
                let legal = inlet.accepts_from.contains(&src);
                let local = node_box::inlet_socket_local(&slot.inlets, ordinal as u16, 0);
                let centre = self.to_visual(slot.pos + local.to_vec2());
                let colour = if legal {
                    Color::from_rgb8(120, 220, 140)
                } else {
                    Color::from_rgb8(70, 72, 80)
                };
                painter
                    .fill(Circle::new(centre, node_box::SOCKET_RADIUS * 1.6 * self.zoom), colour)
                    .draw();
            }
        }
    }
```

`canvas.rs` imports `masonry_core::kurbo::Circle` for this. `post_paint`'s
signature is identical to `paint`'s at this rev (verified against
`masonry_core/src/core/widget.rs:404`, where it has an empty default body), so
it is written above exactly as `paint` already is in this file.

- [ ] **Step 6: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Verify by eye**

Run: `cargo run -p sway-app -- --editor --windowed`
Press on a node's right-hand dot and drag. Expected: a line follows the pointer,
and every inlet that could accept it lights green while the rest dim. Release on
a green one: the edge appears. Press a connected inlet dot: the edge goes away.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(editor): drag-to-connect with registry-driven legality

An inlet carries the entities it could accept, computed world-side from
has_source/has_target — the widget layer highlights rather than deciding, and
re-checks on release because the highlight is a hint and the rule is the rule.
Self-edges are never offered.

The highlight is painted in post_paint rather than paint: paint runs before
children, which is what puts edges behind the node boxes, so a highlight drawn
there would be covered by the box it marks."
```

---

### Task 16: Exit criterion and findings

**Files:**
- Create: `docs/superpowers/reports/2026-08-10-m6-editor-write-half-findings.md`
- Modify: `docs/architecture.md`, `docs/superpowers/specs/2026-07-25-sway-design.md`, `docs/superpowers/specs/2026-08-09-mvp-roadmap-design.md`

- [ ] **Step 1: Run the whole suite**

Run: `cargo test --workspace`
Expected: PASS. Record the exact count for the findings report.

- [ ] **Step 2: Walk the exit criterion by eye**

Run: `cargo run -p sway-app -- --editor --windowed`

In one session, without touching RON:
1. Right-click the canvas; create an `Lfo`.
2. Create a `Vec3`.
3. Drag from the `Lfo`'s outlet to the `Vec3`'s `vec3.y` inlet.
4. Drag from the `Vec3`'s outlet to a cube's `translation` inlet.
5. Edit the `Lfo`'s `beats` in the inspector; confirm the cube's motion changes.
6. Save As to a new path.
7. Quit, relaunch, Open that path.
8. Confirm the graph, the wiring and the edited value all came back.

Record what actually happened, including anything that did not work.

- [ ] **Step 3: Amend the two documents M6-5 invalidates**

In `docs/architecture.md` §7, replace the "The editor therefore treats
wire-driven fields as read-only…" passage with a statement of what M6 actually
does: every field is editable, a save records the instantaneous driven value,
and the first tick after load overwrites it. Update §10's "Out of MVP" entry
for restore-authored-value-on-disconnect, which currently justifies itself by
pointing at the read-only rule.

In `2026-08-09-mvp-roadmap-design.md`, mark D2 as superseded, pointing at
M6-5. In `2026-07-25-sway-design.md`, update the M6 line and the "Restore
authored value on disconnect — superseded by D2" bullet.

- [ ] **Step 4: Write the findings report**

Follow `2026-08-10-m5-minimal-scene-slice-findings.md`'s shape: Question,
Answer, What was built (one bullet per task with its commit), the surprises,
what M7 inherits, and what is not answered. M7 specifically inherits the
driven-axis question M6-5 leaves open for the gizmo.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "docs: M6 findings — the editor write half"
```

---

## Self-Review

**Spec coverage.** M6-1 → Tasks 2-5. M6-2 → Task 1. M6-3 → Task 9. M6-4 → Task 6.
M6-5 → no code by definition; the doc amendments are Task 16 Step 3. M6-6 →
Task 6 (sockets, ordinals) and Task 15 (legality). M6-7 → Task 7. M6-8 → Tasks
10-11. Palette → Tasks 12-13. Delete → Task 3. Inspector editing → Tasks 6, 8.
Drag-to-connect → Tasks 14-15. All four "Verify before implementing" items are
first steps of the tasks that depend on them (1 → Task 3, 2 → Task 7 preamble,
3 → Task 11, 4 → Task 3's `AppTypeRegistry` clone).

**Type consistency.** `InspectorComponent.name` is `&'static str` and
`InspectorField.name` is `String` throughout, settled once in Task 6 and used
unchanged in Task 8 — the component name is `ComponentEntry::name`, a field name
can be a tuple index. `EditorCommand::SetField` matches: `component: &'static str`,
`field: String`. `InletView` is produced in Task 6 with two fields and gains
`accepts_from` in Task 15, which is also where it loses `Copy`; every task that
builds one after that point builds all three fields. `NodeSlot.inlets` stays
`Vec<u16>` (per-field slot counts, for the variadic inlets that stay out of MVP)
and is a different thing from `NodeView.inlets: Vec<InletView>` — Task 6 Step 7
converts between them in the one place they meet, and Task 15 adds
`NodeSlot.inlet_views` for what the counts cannot carry.

**Deviations from the spec, each recorded in its own task and commit message:**
- **M6-8's `FileCommand`** becomes `FileRequest` and carries no path (Task 11).
  A path-carrying variant is unbuildable by the widget that emits it: only
  `sway-app` owns `rfd`.
- **M6-6's socket hit-testing** is split. The spec implies the canvas resolves
  sockets; masonry hit-tests children before parents and `NodeBox` marks every
  primary `Down` handled, so the *press* is detected in `NodeBox` and only the
  *release* is resolved canvas-side (Task 14). The behaviour is what the spec
  describes; the location is what masonry's dispatch allows.

**Verified against the pinned checkout** (`xilem @ c5950bc`) rather than
assumed, because a plan that guesses an API teaches the wrong thing:
`TestHarness::create(DefaultProperties, NewWidget<W>)` and `root_widget()`
returning a `WidgetRef` that derefs to the widget; `pop_action::<T>()` (there is
no `take_actions`); the harness servicing `NewLayer`/`RemoveLayer` itself;
`RenderRoot::{add_layer, remove_layer, reposition_layer, has_widget}`;
`Checkbox::new(bool, impl Into<ArcStr>)`; `Selector::new(Vec<String>)` plus
`with_selected_option`; `TextInput::new(&str)` and `area_pod()`;
`TextArea`'s `TextAction::{Entered, Changed}`; `Button::with_text` and
`ButtonPress`; `Widget::post_paint`'s signature; `LayerStack::layer_count` being
`pub(crate)` and therefore unusable as a test seam.

**One remaining conditional**, because it is a genuine unknown rather than an
unwritten step: Task 3's `Delete` has two shapes depending on what Step 2's
characterization test finds out about Bevy's producer-side wire cleanup. Both
are written out in full, and the test is committed either way.
