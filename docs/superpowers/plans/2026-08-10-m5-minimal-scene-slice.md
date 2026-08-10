# M5 — Minimal Scene Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The demo document authors its own camera, light, material and PBR cubes, with no Rust-side scene setup left anywhere.

**Architecture:** New scene components (`MeshAsset`, `PbrMaterial`, `SceneCamera`) and value nodes (`Vec3`, `Math`, `Remap`) live in `sway-nodes`, each next to the wires that target it. Every one is an ordinary Bevy component; a `Changed<T>` plain system turns it into the Bevy render component it stands for, and `#[require]` supplies its companions. A material is its own entity whose handle reaches meshes through a `MaterialFrom` wire, so sharing is visible topology.

**Tech Stack:** Rust 2024, Bevy 0.19 (pinned `=0.19.0`), wgpu `=29.0.4`, RON 0.12.

## Global Constraints

- **Bevy is pinned at `=0.19.0`** and wgpu at `=29.0.4`. Do not bump either; the bevy↔vello coupling depends on the exact tuple (architecture §5).
- **A wire must never write an equal value.** `get_mut` marks `Changed` unconditionally, and `Changed<T>` is the whole dirty story downstream. Every `propagate` uses `Mut::map_unchanged(..).set_if_neq(..)`, and every wire gets the change-detection test from Task 2 (architecture §7).
- **`sway-graph` must not depend on `bevy_render`, MIDI types, or the document format** (architecture §5). Task 1 is the only `sway-graph` change in this plan and adds no dependency.
- **Registered short names are document keys.** They are not reflect type paths, and they must be unique; `register_authorable` panics at startup on a duplicate.
- **Behaviour vs. plain system** (architecture §2): a component whose output depends on a *wired inlet in the same tick* is a behaviour, placed in the graph order. Everything else — including every `Changed<T>` system in this plan — is an ordinary Bevy system.
- **Rendering is verified by eye**, with the single readback exception in Task 9 (spec M5-4).
- **Commit at the end of every task.** Every task must leave `cargo test --workspace` green and the app runnable.

**Reference documents:**
- Spec: `docs/superpowers/specs/2026-08-10-m5-minimal-scene-slice-design.md`
- Roadmap and decisions D1–D6: `docs/superpowers/specs/2026-08-09-mvp-roadmap-design.md`
- Architecture: `docs/architecture.md`

## Facts already verified

These were checked empirically against this workspace while the plan was written. Do not re-litigate them; if one turns out false, stop and report.

1. `#[require(MeshMaterial3d<StandardMaterial>)]` compiles — `#[require]` accepts a generic type argument — and materialises `MeshMaterial3d(Handle::default())`.
2. `Mesh3d` and `MeshMaterial3d<M>` are both `Default` and `PartialEq`. `Mesh3d` requires only `Transform`, **not** `Visibility`.
3. `Camera3d` drags in `Camera`, `Projection`, `Transform` and `Visibility`. `DirectionalLight` and `PointLight` each drag in `Transform`, and both carry `#[reflect(Component, Default)]`, which is what `register_authorable` demands.
4. `ComponentInfo::required_components().iter_ids()` is public and reports the **transitive** required set (`Mesh3d` reports `Transform`).
5. A bare `World` reports `Changed<T>` correctly across `clear_trackers()`, in both directions — an equal write leaves it clear, a real write sets it.
6. `assets/cube.gltf` as generated in Task 5 loads through `AssetServer` as `"cube.gltf#Mesh0/Primitive0"`, yielding 24 vertices and 36 indices. Under `cargo test`, cargo sets `CARGO_MANIFEST_DIR` in the test process, so Bevy's asset root resolves to `crates/sway-app/assets`.
7. `App::new()` + `(TaskPoolPlugin::default(), AssetPlugin::default())` + `init_asset::<T>()` is enough to run a `Changed<T>` system that calls `AssetServer::load` — no device, no render plugins. Tasks 5 and 6 both use it.
8. A `Transform` serialises to RON as `(translation:(0.0,1.5,5.0),rotation:(-0.14521316,-0.0,-0.0,0.98940045),scale:(1.0,1.0,1.0))` — the quaternion is a 4-tuple, not named fields.

## File structure

**`sway-graph`** (one file):

| File | Responsibility |
|---|---|
| `src/project/apply.rs` (modify) | Pass 3 stops removing `#[require]`-supplied companions |

**`sway-nodes`** — after this milestone:

| File | Responsibility |
|---|---|
| `src/field_wire.rs` (new) | The `field_wire!` macro; nothing else |
| `src/wire_testing.rs` (new, `#[cfg(test)]`) | `assert_writes_only_on_change` |
| `src/value.rs` (new) | `Vec3Value`, `Math`, `Remap` + their inlet wires |
| `src/spatial.rs` (rewritten) | `TranslationFrom`, `RotationFrom`, `ScaleFrom` |
| `src/mesh_asset.rs` (new) | `MeshAsset` + its load system |
| `src/pbr_material.rs` (new) | `PbrMaterial`, `MaterialOut`, `MaterialFrom` + its sync system |
| `src/scene.rs` (rewritten) | `SceneCamera`; light registration |
| `src/osc.rs` (modify) | `Lfo` gains `#[require]`; its tests move off `TranslationYFrom` |
| `src/math.rs` (modify) | Pure `math_value` / `remap_value` stay; `switch_value` deleted |
| `src/mesh.rs`, `src/material.rs` | **deleted** |

**`sway-app`** — after this milestone:

| File | Responsibility |
|---|---|
| `assets/cube.gltf` (new) | The one mesh asset |
| `assets/demo.sway.ron` (rewritten) | The whole scene, as a document |
| `src/main.rs` (modify) | No scene setup; `load_project` gated on `--demo` |
| `tests/demo_document.rs` (rewritten) | World shape of the new document |
| `tests/demo_renders.rs` (new) | The readback test |
| `src/scene.rs`, `src/demo_assets.rs`, `src/lib.rs` | **deleted** |

---

### Task 1: `apply` keeps `#[require]` companions

Roadmap D4 says a palette click spawns one component and Bevy materialises the rest. That breaks on load today: `apply_components` removes every registered-authorable component the document did not name, and its own comment says this deliberately includes "components the entity only acquired implicitly (Bevy required-components)". So a document naming `Lfo` but not `FloatOut` would load an LFO with no outlet.

The rule becomes: a component **required, transitively, by a component this document named on this entity** is exempt from removal. Everything else still goes.

**Files:**
- Modify: `crates/sway-graph/src/project/apply.rs:83-206` (pass 3) and its test module
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: no new public API. Behaviour change every later task depends on — a document may omit `FloatOut`, `Transform`, `Mesh3d`, `MaterialOut` and any other `#[require]`-supplied companion.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/sway-graph/src/project/apply.rs`, after the `Osc` definition:

```rust
    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Outlet(f32);

    /// Stands in for `Lfo`, which requires `FloatOut` (roadmap D4).
    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    #[require(Outlet)]
    struct Emitter;

    fn require_app() -> App {
        let mut app = App::new();
        register_authorable::<Emitter>(&mut app, "Emitter");
        register_authorable::<Outlet>(&mut app, "Outlet");
        register_authorable::<Osc>(&mut app, "Osc");
        app
    }

    #[test]
    fn a_required_companion_survives_a_document_that_does_not_name_it() {
        // D4: the palette spawns one component and Bevy materialises the rest,
        // so a document naming `Emitter` alone must still load an entity with
        // its outlet attached. Without the exemption the removal pass strips
        // `Outlet` right back off again and the node has no output.
        let mut app = require_app();
        let text = r#"Project(version: 1, entities: [
            Entity(id: "a", components: { "Emitter": () })
        ])"#;

        apply(app.world_mut(), &doc(text));
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert!(app.world().get::<Outlet>(entity).is_some(), "first load");

        // A reload is the sharper case: now the component is already present
        // and unnamed, which is exactly what the removal pass looks for.
        apply(app.world_mut(), &doc(text));
        assert!(app.world().get::<Outlet>(entity).is_some(), "after reload");
    }

    #[test]
    fn a_component_no_named_component_requires_is_still_removed() {
        // The exemption must be narrow: only what a *named* component pulls in.
        let mut app = require_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Emitter": (), "Osc": (hz: 3.0) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Emitter": () })
            ])"#),
        );

        assert!(app.world().get::<Outlet>(entity).is_some(), "required, kept");
        assert!(app.world().get::<Osc>(entity).is_none(), "not required, dropped");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph a_required_companion_survives -- --nocapture`
Expected: FAIL — `assertion failed: app.world().get::<Outlet>(entity).is_some()` at the "first load" assertion.

- [ ] **Step 3: Implement the exemption**

In `crates/sway-graph/src/project/apply.rs`, add the import at the top of the file, beside the other `bevy_ecs` imports:

```rust
use bevy_ecs::component::ComponentId;
```

Then in `apply_components`, between the insert loop's closing brace (line 178) and the removal loop's comment block (line 180), insert:

```rust
    // A component the document did not name is removed below — but a
    // `#[require]` companion was never the document's to name. `Lfo` carries
    // `FloatOut` because `Lfo` requires it (roadmap D4), and a document that
    // names only `Lfo` must still load a node with an outlet. So anything
    // required, transitively, by a component this document named on this
    // entity is exempt. `ComponentInfo::required_components()` reports the
    // transitive set, so a `MeshAsset` that requires `Mesh3d` exempts
    // `Transform` too.
    let mut required_by_named: Vec<ComponentId> = Vec::new();
    for type_id in &written {
        let Some(component_id) = world.components().get_id(*type_id) else {
            continue;
        };
        let Some(info) = world.components().get_info(component_id) else {
            continue;
        };
        required_by_named.extend(info.required_components().iter_ids());
    }
```

And inside the removal loop, immediately after the `if written.contains(&entry.type_id) { continue; }` guard:

```rust
        if world
            .components()
            .get_id(entry.type_id)
            .is_some_and(|id| required_by_named.contains(&id))
        {
            continue;
        }
```

Finally, extend the removal loop's existing doc comment (lines 180-184) so it no longer claims required components are removed:

```rust
    // Anything registered-authorable, present, absent from the document, and
    // not required by something the document did name is removed — including
    // components the entity acquired from a runtime system. `Transform` is the
    // sharpest case: a doc-owned entity that picks one up outside the document,
    // and whose named components do not require one, loses it on the next
    // reload. Spec §4.1; intended.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, all tests, including the pre-existing `a_component_dropped_from_the_document_is_removed`.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/project/apply.rs
git commit -m "fix(document): keep #[require] companions a document does not name

D4 has the palette spawn one component and Bevy materialise the rest, but
apply's removal pass stripped exactly those companions back off on load.
Exempt anything transitively required by a component the document named."
```

---

### Task 2: The wire macro, the change-detection helper, the `Vec3` node

Three things arrive together because none of them is testable alone: the macro has no users without a wire, the wires have no producer without the node, and the helper has nothing to check without both.

**Files:**
- Create: `crates/sway-nodes/src/field_wire.rs`
- Create: `crates/sway-nodes/src/wire_testing.rs`
- Create: `crates/sway-nodes/src/value.rs`
- Modify: `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/src/osc.rs`

**Interfaces:**
- Consumes: `sway_graph::{Wire, propagate_of, register_wire, register_behaviour, register_authorable, TickCtx, EditorPos}`; `crate::outputs::{FloatOut, Vec3Out}`.
- Produces:
  - `field_wire!(WireName / DrivesName, Source => Target, "doc-name", |t| &mut t.field, |s| value)` — declares a `pub struct WireName(pub Entity)`, a `pub struct DrivesName(Vec<Entity>)`, and `impl Wire for WireName`.
  - `crate::wire_testing::assert_writes_only_on_change::<W>(source: W::Source, different: W::Source, target: W::Target)`.
  - `pub struct Vec3Value { pub x: f32, pub y: f32, pub z: f32 }`, doc name `"Vec3"`, requires `Vec3Out` + `EditorPos`.
  - `pub fn vec3_behaviour(&mut World, Entity, &TickCtx)`.
  - Wires `Vec3XFrom` / `Vec3YFrom` / `Vec3ZFrom` with doc names `"vec3.x"`, `"vec3.y"`, `"vec3.z"`.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-nodes/src/value.rs` with only its test module for now:

```rust
//! Value nodes: literals and arithmetic that produce an outlet.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::outputs::{FloatOut, Vec3Out};
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::WiresPlugin;

    fn slice_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy::time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(WiresPlugin)
            .add_plugins(crate::WireNodesPlugin);
        app.update(); // frame 0 starts with an empty fixed-time accumulator
        app
    }

    #[test]
    fn a_vec3_node_publishes_its_three_fields() {
        let mut app = slice_app();
        let node = app
            .world_mut()
            .spawn(Vec3Value { x: 1.0, y: 2.0, z: 3.0 })
            .id();

        app.update();

        assert_eq!(
            app.world().get::<Vec3Out>(node).map(|o| o.0),
            Some(Vec3::new(1.0, 2.0, 3.0)),
            "#[require] must have supplied Vec3Out, and the behaviour filled it"
        );
    }

    #[test]
    fn a_float_reaches_a_vec3_axis_in_one_tick() {
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(0.75)).id();
        let node = app.world_mut().spawn(Vec3Value::default()).id();
        app.world_mut().entity_mut(node).insert(Vec3YFrom(float));

        app.update();

        assert_eq!(
            app.world().get::<Vec3Out>(node).map(|o| o.0),
            Some(Vec3::new(0.0, 0.75, 0.0)),
            "the inlet must land before the behaviour runs, in ONE tick"
        );
    }

    #[test]
    fn the_vec3_inlets_never_write_an_equal_value() {
        assert_writes_only_on_change::<Vec3XFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
        assert_writes_only_on_change::<Vec3YFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
        assert_writes_only_on_change::<Vec3ZFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib value`
Expected: FAIL to compile — `cannot find type Vec3Value`, `cannot find module wire_testing`.

- [ ] **Step 3: Write the macro**

Create `crates/sway-nodes/src/field_wire.rs`:

```rust
//! One macro for the twenty-odd near-identical value wires.
//!
//! A wire is a `Relationship` on the consumer, a `RelationshipTarget` on the
//! producer, and a `propagate` that writes one field. All three are mechanical;
//! the only per-wire facts are the two types, the document name, which field of
//! the target to write, and how to get the value out of the source.
//!
//! `set_if_neq` is not optional. `get_mut` marks `Changed` unconditionally, and
//! `Changed<T>` is the whole dirty story downstream (architecture §7), so a wire
//! that writes an equal value silently recooks everything it feeds.

/// ```ignore
/// field_wire!(
///     /// Doc comment for the wire type.
///     TranslationFrom / DrivesTranslation,
///     Vec3Out => Transform,
///     "translation",
///     |t| &mut t.translation,
///     |s| s.0
/// );
/// ```
macro_rules! field_wire {
    (
        $(#[$attr:meta])*
        $wire:ident / $drives:ident,
        $src:ty => $dst:ty,
        $name:literal,
        |$t:ident| $field:expr,
        |$s:ident| $value:expr
    ) => {
        $(#[$attr])*
        #[derive(bevy::prelude::Component)]
        #[relationship(relationship_target = $drives)]
        pub struct $wire(#[entities] pub bevy::prelude::Entity);

        #[derive(bevy::prelude::Component)]
        #[relationship_target(relationship = $wire)]
        pub struct $drives(Vec<bevy::prelude::Entity>);

        impl sway_graph::Wire for $wire {
            type Source = $src;
            type Target = $dst;
            const NAME: &'static str = $name;

            fn propagate(src: &$src, dst: bevy_ecs::change_detection::Mut<$dst>) {
                let $s = src;
                let value = $value;
                let mut field = dst.map_unchanged(|$t| $field);
                bevy_ecs::change_detection::DetectChangesMut::set_if_neq(&mut field, value);
            }
        }
    };
}

pub(crate) use field_wire;
```

- [ ] **Step 4: Write the change-detection helper**

Create `crates/sway-nodes/src/wire_testing.rs`:

```rust
//! The per-wire change-detection check architecture §9 requires.
//!
//! Both halves matter. The first proves the wire does not dirty its target when
//! nothing changed. The second proves the harness can see a write at all —
//! without it, a wire that never writes anything would pass the first half.

#![cfg(test)]

use bevy::prelude::*;
use sway_graph::{Wire, propagate_of};

fn changed_count<T: Component>(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), Changed<T>>();
    query.iter(world).count()
}

/// Propagates `source` twice and asserts the second write left `Changed` clear,
/// then propagates `different` and asserts that one did not.
pub(crate) fn assert_writes_only_on_change<W: Wire>(
    source: W::Source,
    different: W::Source,
    target: W::Target,
) {
    let mut world = World::new();
    let src = world.spawn(source).id();
    let dst = world.spawn(target).id();

    propagate_of::<W>(&mut world, src, dst);
    world.clear_trackers();
    propagate_of::<W>(&mut world, src, dst);
    assert_eq!(
        changed_count::<W::Target>(&mut world),
        0,
        "wire \"{}\" wrote an equal value; use map_unchanged(..).set_if_neq(..)",
        W::NAME
    );

    let other = world.spawn(different).id();
    world.clear_trackers();
    propagate_of::<W>(&mut world, other, dst);
    assert_eq!(
        changed_count::<W::Target>(&mut world),
        1,
        "wire \"{}\" did not write a genuinely different value — the check above \
         proves nothing",
        W::NAME
    );
}
```

- [ ] **Step 5: Write the `Vec3` node and its inlets**

Prepend to `crates/sway-nodes/src/value.rs`, above the test module:

```rust
use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::{EditorPos, TickCtx};

use crate::field_wire::field_wire;
use crate::outputs::{FloatOut, Vec3Out};

/// A vector literal whose components are driveable (roadmap D5). Transform,
/// colour and tint inlets take `Vec3`, so something has to produce one; this
/// reads as a value in the graph rather than as a `Compose` operator, which is
/// how both TouchDesigner and Houdini present it.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Vec3Out, EditorPos)]
pub struct Vec3Value {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// A behaviour, not a plain system: each axis may be wire-driven, so this has
/// to run between those propagations and the propagation of its own outlet.
pub fn vec3_behaviour(world: &mut World, entity: Entity, _ctx: &TickCtx) {
    let Some(value) = world.get::<Vec3Value>(entity).copied() else {
        return;
    };
    if let Some(mut out) = world.get_mut::<Vec3Out>(entity) {
        out.set_if_neq(Vec3Out(Vec3::new(value.x, value.y, value.z)));
    }
}

field_wire!(
    /// Drives `Vec3.x`.
    Vec3XFrom / DrivesVec3X,
    FloatOut => Vec3Value,
    "vec3.x",
    |t| &mut t.x,
    |s| s.0
);

field_wire!(
    /// Drives `Vec3.y`.
    Vec3YFrom / DrivesVec3Y,
    FloatOut => Vec3Value,
    "vec3.y",
    |t| &mut t.y,
    |s| s.0
);

field_wire!(
    /// Drives `Vec3.z`.
    Vec3ZFrom / DrivesVec3Z,
    FloatOut => Vec3Value,
    "vec3.z",
    |t| &mut t.z,
    |s| s.0
);
```

- [ ] **Step 6: Give `Lfo` its `#[require]` companions and register everything**

In `crates/sway-nodes/src/osc.rs`, add `EditorPos` to the imports and the attribute to `Lfo`:

```rust
use sway_graph::{EditorPos, TickCtx, Transport, TransportTime, Wire};
```

```rust
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(FloatOut, EditorPos)]
pub struct Lfo {
```

In `crates/sway-nodes/src/lib.rs`, add the modules (alphabetical among the existing `mod` lines) and register the new node:

```rust
mod field_wire;
mod value;
#[cfg(test)]
mod wire_testing;
```

```rust
pub use value::*;
```

and inside `WireNodesPlugin::build`, after the `Lfo` registrations:

```rust
        sway_graph::register_behaviour::<Vec3Value>(app, vec3_behaviour);
        sway_graph::register_wire::<Vec3XFrom>(app);
        sway_graph::register_wire::<Vec3YFrom>(app);
        sway_graph::register_wire::<Vec3ZFrom>(app);
```

```rust
        sway_graph::register_authorable::<Vec3Value>(app, "Vec3");
```

Update the expected list in `the_plugin_registers_every_authorable_component`:

```rust
        assert_eq!(
            names,
            vec!["EditorPos", "FloatOut", "Lfo", "Transform", "Vec3", "Vec3Out"]
        );
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes`
Expected: PASS. If `a_float_reaches_a_vec3_axis_in_one_tick` fails with `Some(Vec3::ZERO)`, the behaviour ran before the inlet propagated — check that `register_behaviour::<Vec3Value>` was called, since the order emits inbound propagations before an entity's behaviours.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-nodes/src/field_wire.rs crates/sway-nodes/src/wire_testing.rs \
        crates/sway-nodes/src/value.rs crates/sway-nodes/src/osc.rs crates/sway-nodes/src/lib.rs
git commit -m "feat(nodes): the Vec3 value node, on a field_wire macro

D5 makes transform and colour inlets Vec3, so Vec3Out needs a producer.
Adds the macro every remaining wire is generated from, and the per-wire
change-detection check architecture §9 asks for."
```

---

### Task 3: Transform wires replace `TranslationYFrom`

D5: `TranslationFrom`, `RotationFrom` (euler, degrees) and `ScaleFrom` take `Vec3Out`; there are no per-axis transform wires. The demo document and the tests that assert on `"translation.y"` migrate in the same task, so the tree stays green.

**Files:**
- Rewrite: `crates/sway-nodes/src/spatial.rs`
- Modify: `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/src/osc.rs` (tests)
- Modify: `crates/sway-app/assets/demo.sway.ron`, `crates/sway-app/tests/demo_document.rs`

**Interfaces:**
- Consumes: `field_wire!`, `assert_writes_only_on_change`, `Vec3Value`, `Vec3Out` from Task 2.
- Produces: `TranslationFrom` (`"translation"`), `RotationFrom` (`"rotation"`), `ScaleFrom` (`"scale"`), each `Vec3Out => Transform`. `TranslationYFrom` and `DrivesTranslationY` no longer exist.

- [ ] **Step 1: Write the failing test**

Replace the whole of `crates/sway-nodes/src/spatial.rs` with its test module first:

```rust
//! Wires into the scene transform. Roadmap D5: these take `Vec3`, not floats.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::outputs::Vec3Out;
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::propagate_of;

    #[test]
    fn translation_and_scale_write_the_whole_vector() {
        let mut world = World::new();
        let src = world.spawn(Vec3Out(Vec3::new(1.0, 2.0, 3.0))).id();
        let dst = world.spawn(Transform::default()).id();

        propagate_of::<TranslationFrom>(&mut world, src, dst);
        propagate_of::<ScaleFrom>(&mut world, src, dst);

        let transform = world.get::<Transform>(dst).copied().expect("present");
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn rotation_reads_euler_degrees() {
        // Degrees because that is what an author types. The wire converts, so
        // nothing downstream ever sees a degree.
        let mut world = World::new();
        let src = world.spawn(Vec3Out(Vec3::new(0.0, 90.0, 0.0))).id();
        let dst = world.spawn(Transform::default()).id();

        propagate_of::<RotationFrom>(&mut world, src, dst);

        let rotation = world.get::<Transform>(dst).expect("present").rotation;
        let turned = rotation * Vec3::Z;
        assert!(
            (turned - Vec3::X).length() < 1e-5,
            "90 degrees about Y must take +Z to +X, got {turned:?}"
        );
    }

    #[test]
    fn the_transform_wires_never_write_an_equal_value() {
        assert_writes_only_on_change::<TranslationFrom>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        assert_writes_only_on_change::<ScaleFrom>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        // The quaternion is compared, not the euler triple it came from.
        assert_writes_only_on_change::<RotationFrom>(
            Vec3Out(Vec3::new(0.0, 90.0, 0.0)),
            Vec3Out(Vec3::new(0.0, 45.0, 0.0)),
            Transform::default(),
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib spatial`
Expected: FAIL to compile — `cannot find type TranslationFrom`.

- [ ] **Step 3: Write the three wires**

Prepend to `crates/sway-nodes/src/spatial.rs`:

```rust
use bevy::prelude::*;

use crate::field_wire::field_wire;
use crate::outputs::Vec3Out;

field_wire!(
    /// Drives `Transform.translation` whole. There is no per-axis wire: an
    /// offset that used to live in the authored `Transform` now lives in the
    /// `Vec3` node feeding this (roadmap D5).
    TranslationFrom / DrivesTranslation,
    Vec3Out => Transform,
    "translation",
    |t| &mut t.translation,
    |s| s.0
);

field_wire!(
    /// Drives `Transform.rotation` from euler angles in **degrees**, XYZ order.
    /// The quaternion is built before the comparison, so an unchanged triple
    /// leaves `Transform` clean.
    RotationFrom / DrivesRotation,
    Vec3Out => Transform,
    "rotation",
    |t| &mut t.rotation,
    |s| Quat::from_euler(
        EulerRot::XYZ,
        s.0.x.to_radians(),
        s.0.y.to_radians(),
        s.0.z.to_radians()
    )
);

field_wire!(
    /// Drives `Transform.scale` whole.
    ScaleFrom / DrivesScale,
    Vec3Out => Transform,
    "scale",
    |t| &mut t.scale,
    |s| s.0
);
```

- [ ] **Step 4: Register them and drop `TranslationYFrom`**

In `crates/sway-nodes/src/lib.rs`, replace `sway_graph::register_wire::<TranslationYFrom>(app);` with:

```rust
        sway_graph::register_wire::<TranslationFrom>(app);
        sway_graph::register_wire::<RotationFrom>(app);
        sway_graph::register_wire::<ScaleFrom>(app);
```

- [ ] **Step 5: Move `osc.rs`'s chain tests onto the new wires**

In `crates/sway-nodes/src/osc.rs`'s test module, replace the `use crate::spatial::TranslationYFrom;` line with:

```rust
    use crate::spatial::TranslationFrom;
    use crate::value::{Vec3Value, Vec3YFrom};
```

and rewrite the two tests that used it:

```rust
    #[test]
    fn a_modulated_lfo_reaches_a_transform_in_one_tick() {
        // The chain the design turns on, now one node longer (D5):
        //   Lfo A -> Lfo B.amplitude -> B computes -> Vec3.y -> Transform
        // A's output is 1.0 * 0.5 = 0.5, which becomes B's amplitude, so B's
        // output is 0.5, which becomes the vector's y. If the order were wrong,
        // B would still hold its authored amplitude of 0.0 and y would be 0.0.
        let mut app = slice_app();
        let a = app.world_mut().spawn(lfo(0.25, 0.5)).id();
        let b = app.world_mut().spawn(lfo(0.25, 0.0)).id();
        let vector = app.world_mut().spawn(Vec3Value::default()).id();
        let mesh = app.world_mut().spawn(Transform::default()).id();
        app.world_mut().entity_mut(b).insert(AmplitudeFrom(a));
        app.world_mut().entity_mut(vector).insert(Vec3YFrom(b));
        app.world_mut().entity_mut(mesh).insert(TranslationFrom(vector));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(a).map(|o| o.0), Some(0.5));
        assert_eq!(app.world().get::<Lfo>(b).map(|l| l.amplitude), Some(0.5));
        assert_eq!(
            app.world().get::<Transform>(mesh).map(|t| t.translation.y),
            Some(0.5),
            "the whole chain must land in ONE tick"
        );
    }

    #[test]
    fn one_producer_fans_out_to_two_consumers() {
        let mut app = slice_app();
        let a = app.world_mut().spawn(lfo(0.25, 0.5)).id();
        let vector = app.world_mut().spawn(Vec3Value::default()).id();
        app.world_mut().entity_mut(vector).insert(Vec3YFrom(a));
        let x = app.world_mut().spawn(Transform::default()).id();
        let y = app.world_mut().spawn(Transform::default()).id();
        app.world_mut().entity_mut(x).insert(TranslationFrom(vector));
        app.world_mut().entity_mut(y).insert(TranslationFrom(vector));

        app.update();

        assert_eq!(app.world().get::<Transform>(x).map(|t| t.translation.y), Some(0.5));
        assert_eq!(app.world().get::<Transform>(y).map(|t| t.translation.y), Some(0.5));
    }
```

Note the `lfo(..)` helper now spawns a bare `Lfo`, because `#[require(FloatOut)]` supplies the outlet — drop the `FloatOut::default()` from those spawn tuples as shown.

- [ ] **Step 6: Migrate the demo document to the new wire**

In `crates/sway-app/assets/demo.sway.ron`, replace the header comment's first two lines and the two cube entities, inserting the two `Vec3` nodes. The cubes' x offsets move into the vector nodes, because `translation` is now written whole:

```ron
//   Lfo A ──amplitude──▶ Lfo B ──vec3.y──▶ vec3B ──translation──▶ cube B
//         └──vec3.y──▶ vec3A ──translation──▶ cube A
```

```ron
        Entity(
            id: "vec3A",
            components: {
                "Vec3": (x: -0.8, y: 0.0, z: 0.0),
                "EditorPos": ((-40.0, 40.0)),
            },
            wires: { "vec3.y": "lfoA" },
        ),
        Entity(
            id: "vec3B",
            components: {
                "Vec3": (x: 0.8, y: 0.0, z: 0.0),
                "EditorPos": ((-40.0, 120.0)),
            },
            wires: { "vec3.y": "lfoB" },
        ),
        Entity(
            id: "cubeA",
            components: {
                "Transform": (),
                "DemoCube": (),
                "EditorPos": ((180.0, 40.0)),
            },
            wires: { "translation": "vec3A", "parent": "group" },
        ),
        Entity(
            id: "cubeB",
            components: {
                "Transform": (),
                "DemoCube": (),
                "EditorPos": ((180.0, 120.0)),
            },
            wires: { "translation": "vec3B", "parent": "group" },
        ),
```

- [ ] **Step 7: Update the demo document test**

In `crates/sway-app/tests/demo_document.rs`: change the import to `use sway_nodes::{AmplitudeFrom, TranslationFrom, Vec3YFrom};`, update the module-doc diagram to match the document's, extend the expected id list to `["cubeA", "cubeB", "group", "lfoA", "lfoB", "vec3A", "vec3B"]`, and replace the two `TranslationYFrom` assertions with:

```rust
    let vec3_a = entity_named(world, "vec3A");
    let vec3_b = entity_named(world, "vec3B");
    assert_eq!(world.get::<Vec3YFrom>(vec3_a).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_b).map(|w| w.0), Some(lfo_b));
    assert_eq!(world.get::<TranslationFrom>(cube_a).map(|w| w.0), Some(vec3_a));
    assert_eq!(world.get::<TranslationFrom>(cube_b).map(|w| w.0), Some(vec3_b));
```

- [ ] **Step 8: Run the whole workspace**

Run: `cargo test --workspace`
Expected: PASS. `demo_document_loads_and_reconciles_cleanly` failing with an `UnknownWire` diagnostic means the document still names `"translation.y"` somewhere.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-nodes/src/spatial.rs crates/sway-nodes/src/lib.rs crates/sway-nodes/src/osc.rs \
        crates/sway-app/assets/demo.sway.ron crates/sway-app/tests/demo_document.rs
git commit -m "feat(nodes): Vec3 transform wires replace TranslationYFrom

D5: transform inlets take Vec3, not per-axis floats. The demo's cube
offsets move into the Vec3 nodes, since translation is now written whole."
```

---

### Task 4: `Math` and `Remap` nodes

Both wrap pure functions that already exist and are already tested; this is the wrapping only. `switch_value` goes, per the roadmap's deleted list — nothing wants a bool outlet.

**Files:**
- Modify: `crates/sway-nodes/src/value.rs`, `crates/sway-nodes/src/math.rs`, `crates/sway-nodes/src/lib.rs`

**Interfaces:**
- Consumes: `field_wire!`, `assert_writes_only_on_change`, `FloatOut`, `math_value`, `remap_value`, `MathOp`.
- Produces: `Math { op: MathOp, a: f32, b: f32 }` (doc name `"Math"`), `Remap { input, in_min, in_max, out_min, out_max, clamp }` (doc name `"Remap"`), behaviours `math_behaviour` / `remap_behaviour`, wires `MathAFrom` (`"math.a"`), `MathBFrom` (`"math.b"`), `RemapInputFrom` (`"remap.input"`).

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/sway-nodes/src/value.rs`:

```rust
    #[test]
    fn math_computes_from_its_authored_and_driven_inlets() {
        // "LFO x 2" is one Math with b left unwired — the reason there is no
        // Const node.
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(3.0)).id();
        let node = app
            .world_mut()
            .spawn(Math { op: MathOp::Mul, a: 0.0, b: 2.0 })
            .id();
        app.world_mut().entity_mut(node).insert(MathAFrom(float));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(node).map(|o| o.0), Some(6.0));
    }

    #[test]
    fn remap_rescales_its_driven_input() {
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(0.5)).id();
        let node = app
            .world_mut()
            .spawn(Remap {
                input: 0.0,
                in_min: 0.0,
                in_max: 1.0,
                out_min: 0.0,
                out_max: 10.0,
                clamp: true,
            })
            .id();
        app.world_mut().entity_mut(node).insert(RemapInputFrom(float));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(node).map(|o| o.0), Some(5.0));
    }

    #[test]
    fn the_math_and_remap_inlets_never_write_an_equal_value() {
        assert_writes_only_on_change::<MathAFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Math::default(),
        );
        assert_writes_only_on_change::<MathBFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Math::default(),
        );
        assert_writes_only_on_change::<RemapInputFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Remap::default(),
        );
    }
```

and extend the test module's imports with `use crate::math::MathOp;`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib value`
Expected: FAIL to compile — `cannot find type Math`.

- [ ] **Step 3: Write both nodes**

Append to `crates/sway-nodes/src/value.rs`, above the test module:

```rust
use crate::math::{MathOp, math_value, remap_value};

/// Binary arithmetic. `b` is an authored field a wire may override, which is
/// why there is no `Const` node: "LFO x 2" is one `Math` with `b: 2.0` unwired.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(FloatOut, EditorPos)]
pub struct Math {
    pub op: MathOp,
    pub a: f32,
    pub b: f32,
}

pub fn math_behaviour(world: &mut World, entity: Entity, _ctx: &TickCtx) {
    let Some(node) = world.get::<Math>(entity).copied() else {
        return;
    };
    let value = math_value(node.op, node.a, node.b);
    if let Some(mut out) = world.get_mut::<FloatOut>(entity) {
        out.set_if_neq(FloatOut(value));
    }
}

field_wire!(
    /// Drives `Math.a`.
    MathAFrom / DrivesMathA,
    FloatOut => Math,
    "math.a",
    |t| &mut t.a,
    |s| s.0
);

field_wire!(
    /// Drives `Math.b`.
    MathBFrom / DrivesMathB,
    FloatOut => Math,
    "math.b",
    |t| &mut t.b,
    |s| s.0
);

/// Rescales `input` from one range to another. `input` is a field rather than
/// an implicit inlet so that `RemapInputFrom` has something to write, exactly
/// as `Math.a` does.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(FloatOut, EditorPos)]
pub struct Remap {
    pub input: f32,
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub clamp: bool,
}

impl Default for Remap {
    fn default() -> Self {
        Self {
            input: 0.0,
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            clamp: false,
        }
    }
}

pub fn remap_behaviour(world: &mut World, entity: Entity, _ctx: &TickCtx) {
    let Some(node) = world.get::<Remap>(entity).copied() else {
        return;
    };
    let value = remap_value(
        node.input,
        node.in_min,
        node.in_max,
        node.out_min,
        node.out_max,
        node.clamp,
    );
    if let Some(mut out) = world.get_mut::<FloatOut>(entity) {
        out.set_if_neq(FloatOut(value));
    }
}

field_wire!(
    /// Drives `Remap.input`.
    RemapInputFrom / DrivesRemapInput,
    FloatOut => Remap,
    "remap.input",
    |t| &mut t.input,
    |s| s.0
);
```

- [ ] **Step 4: Register them, delete `switch_value`**

In `crates/sway-nodes/src/lib.rs`, inside `WireNodesPlugin::build`:

```rust
        sway_graph::register_behaviour::<Math>(app, math_behaviour);
        sway_graph::register_behaviour::<Remap>(app, remap_behaviour);
        sway_graph::register_wire::<MathAFrom>(app);
        sway_graph::register_wire::<MathBFrom>(app);
        sway_graph::register_wire::<RemapInputFrom>(app);
```

```rust
        app.register_type::<MathOp>();
        sway_graph::register_authorable::<Math>(app, "Math");
        sway_graph::register_authorable::<Remap>(app, "Remap");
```

and extend the expected list in `the_plugin_registers_every_authorable_component` to:

```rust
            vec!["EditorPos", "FloatOut", "Lfo", "Math", "Remap", "Transform", "Vec3", "Vec3Out"]
```

In `crates/sway-nodes/src/math.rs`, delete the `switch_value` function (lines 46-48). It has no callers — the roadmap cut it because it needs a bool outlet and nothing produces one.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-nodes`
Expected: PASS, including the untouched `tests/traces.rs` golden traces, which still call `math_value` and `remap_value` directly.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes/src/value.rs crates/sway-nodes/src/math.rs crates/sway-nodes/src/lib.rs
git commit -m "feat(nodes): Math and Remap wrap their existing pure logic

Without them Lfo is the only value source M6's palette has to list. The
arithmetic and its tests already existed; this is the component around it.
Deletes switch_value, which needs a bool outlet nothing produces."
```

---

### Task 5: `MeshAsset` and the cube asset

**Files:**
- Create: `crates/sway-nodes/src/mesh_asset.rs`
- Create: `crates/sway-app/assets/cube.gltf`
- Delete: `crates/sway-nodes/src/mesh.rs`
- Modify: `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/Cargo.toml`

**Interfaces:**
- Consumes: `sway_graph::{EditorPos, register_authorable}`.
- Produces: `pub struct MeshAsset { pub path: String }` (doc name `"MeshAsset"`), requiring `Transform`, `Visibility`, `Mesh3d`, `MeshMaterial3d<StandardMaterial>` and `EditorPos`; `pub fn load_mesh_assets(..)` registered in `Update`. The asset is addressed as `"cube.gltf#Mesh0/Primitive0"`.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-nodes/src/mesh_asset.rs` with its test module only:

```rust
//! `MeshAsset` — a mesh that comes from a file.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;

    /// `AssetPlugin` plus the one asset type, which is all the load system
    /// needs — no device, no renderer. The path never resolves to a real file
    /// here; `AssetServer::load` hands back its handle immediately either way,
    /// and that handle is what this system's contract is about.
    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.add_systems(Update, load_mesh_assets);
        app
    }

    #[test]
    fn a_path_becomes_a_mesh_handle() {
        let mut app = asset_app();
        let entity = app
            .world_mut()
            .spawn(MeshAsset { path: "cube.gltf#Mesh0/Primitive0".into() })
            .id();

        app.update();

        let handle = app.world().get::<Mesh3d>(entity).expect("#[require] supplies Mesh3d");
        assert_ne!(handle.0, Handle::default(), "the load system replaced the default handle");
    }

    #[test]
    fn an_empty_path_leaves_the_handle_alone() {
        // What a palette click produces before anyone types a path. It must not
        // ask the asset server to load "", which logs an error every frame.
        let mut app = asset_app();
        let entity = app.world_mut().spawn(MeshAsset::default()).id();

        app.update();

        assert_eq!(
            app.world().get::<Mesh3d>(entity).map(|m| m.0.clone()),
            Some(Handle::default())
        );
    }

    #[test]
    fn require_supplies_everything_the_renderer_needs() {
        // Mesh3d requires Transform but NOT Visibility, which is why Visibility
        // is on MeshAsset's own require list. Without it nothing draws.
        let mut app = asset_app();
        let entity = app.world_mut().spawn(MeshAsset::default()).id();

        assert!(app.world().get::<Transform>(entity).is_some());
        assert!(app.world().get::<Visibility>(entity).is_some());
        assert!(app.world().get::<Mesh3d>(entity).is_some());
        assert!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(entity).is_some(),
            "the material wire needs a target component to write into"
        );
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib mesh_asset`
Expected: FAIL to compile — `cannot find type MeshAsset`.

- [ ] **Step 3: Write the component and its system**

Prepend to `crates/sway-nodes/src/mesh_asset.rs`:

```rust
use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::EditorPos;

/// A mesh named by path. The sub-asset label is part of the path —
/// `"cube.gltf#Mesh0/Primitive0"` — because a glTF file holds many meshes.
///
/// `Mesh3d` and `MeshMaterial3d` are required rather than inserted by the
/// system below so that a `MaterialFrom` wire always has a target to write
/// into, even before anything has loaded.
#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Transform, Visibility, Mesh3d, MeshMaterial3d<StandardMaterial>, EditorPos)]
pub struct MeshAsset {
    pub path: String,
}

/// An ordinary `Changed<T>` system — the second row of the behaviour table
/// (architecture §2): it consumes nothing the graph produces within a tick.
pub fn load_mesh_assets(
    asset_server: Res<AssetServer>,
    mut meshes: Query<(&MeshAsset, &mut Mesh3d), Changed<MeshAsset>>,
) {
    for (asset, mut mesh) in &mut meshes {
        if asset.path.is_empty() {
            continue;
        }
        mesh.set_if_neq(Mesh3d(asset_server.load(asset.path.clone())));
    }
}
```

- [ ] **Step 4: Register it, and retire `mesh.rs`**

In `crates/sway-nodes/src/lib.rs`: add `mod mesh_asset;` and `pub use mesh_asset::*;`, delete `mod mesh;` and `pub use mesh::*;`, and inside `WireNodesPlugin::build`:

```rust
        sway_graph::register_authorable::<MeshAsset>(app, "MeshAsset");
        app.add_systems(
            bevy_app::Update,
            load_mesh_assets.run_if(bevy_ecs::prelude::resource_exists::<AssetServer>),
        );
```

The `run_if` matters: `sway-nodes`' own test apps have no `AssetPlugin`, and an unguarded system would panic on the missing resource the first time one of them called `update()`.

Extend the authorable list in the plugin test to include `"MeshAsset"` (alphabetically: after `"Math"`).

Then:

```bash
git rm crates/sway-nodes/src/mesh.rs
```

`geometry_to_mesh` was its only content, and D1 takes procedural geometry out of the MVP. It was also `sway-nodes`' only use of `sway-geo`, so remove that dependency line from `crates/sway-nodes/Cargo.toml`:

```toml
sway-graph.workspace = true
bevy.workspace = true
```

(delete `sway-geo.workspace = true`). `sway-geo` stays a workspace member, dormant, reachable through `sway-app`.

- [ ] **Step 5: Generate the cube asset**

Write this generator to your scratch directory (**not** into the repo) and run it from `crates/sway-app/assets/`:

```python
import base64, json, struct

# 24 vertices: 6 faces x 4 corners, each face with its own normal, so the cube
# has hard edges rather than the smeared shading per-vertex normals would give.
faces = [
    ((0,0,1),  [(-1,-1, 1), ( 1,-1, 1), ( 1, 1, 1), (-1, 1, 1)]),
    ((0,0,-1), [( 1,-1,-1), (-1,-1,-1), (-1, 1,-1), ( 1, 1,-1)]),
    ((1,0,0),  [( 1,-1, 1), ( 1,-1,-1), ( 1, 1,-1), ( 1, 1, 1)]),
    ((-1,0,0), [(-1,-1,-1), (-1,-1, 1), (-1, 1, 1), (-1, 1,-1)]),
    ((0,1,0),  [(-1, 1, 1), ( 1, 1, 1), ( 1, 1,-1), (-1, 1,-1)]),
    ((0,-1,0), [(-1,-1,-1), ( 1,-1,-1), ( 1,-1, 1), (-1,-1, 1)]),
]
HALF = 0.5  # a 1x1x1 cube
positions, normals, indices = [], [], []
for n, corners in faces:
    base = len(positions)
    for c in corners:
        positions.append(tuple(v * HALF for v in c))
        normals.append(n)
    indices += [base, base+1, base+2, base, base+2, base+3]

pos_bytes = b"".join(struct.pack("<fff", *p) for p in positions)
nrm_bytes = b"".join(struct.pack("<fff", *n) for n in normals)
idx_bytes = b"".join(struct.pack("<H", i) for i in indices)
# Each accessor's byte offset must be a multiple of its component size (4 for
# float, 2 for u16); these three blocks are 288/288/72 bytes, so they already are.
buf = pos_bytes + nrm_bytes + idx_bytes
uri = "data:application/octet-stream;base64," + base64.b64encode(buf).decode()

gltf = {
  "asset": {"version": "2.0", "generator": "sway M5 cube"},
  "scene": 0,
  "scenes": [{"nodes": [0]}],
  "nodes": [{"mesh": 0, "name": "Cube"}],
  "meshes": [{"name": "Cube", "primitives": [
      {"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "mode": 4}]}],
  "buffers": [{"byteLength": len(buf), "uri": uri}],
  "bufferViews": [
      {"buffer": 0, "byteOffset": 0, "byteLength": len(pos_bytes), "target": 34962},
      {"buffer": 0, "byteOffset": len(pos_bytes), "byteLength": len(nrm_bytes), "target": 34962},
      {"buffer": 0, "byteOffset": len(pos_bytes)+len(nrm_bytes), "byteLength": len(idx_bytes), "target": 34963},
  ],
  "accessors": [
      {"bufferView": 0, "componentType": 5126, "count": len(positions), "type": "VEC3",
       "min": [-HALF,-HALF,-HALF], "max": [HALF,HALF,HALF]},
      {"bufferView": 1, "componentType": 5126, "count": len(normals), "type": "VEC3"},
      {"bufferView": 2, "componentType": 5123, "count": len(indices), "type": "SCALAR"},
  ],
}
open("cube.gltf", "w").write(json.dumps(gltf, indent=2) + "\n")
print("vertices", len(positions), "indices", len(indices), "buffer bytes", len(buf))
```

Expected output: `vertices 24 indices 36 buffer bytes 648`, and a 2231-byte `crates/sway-app/assets/cube.gltf`. The generator is a one-shot; only its output is checked in.

- [ ] **Step 6: Prove the asset loads**

Create `crates/sway-app/tests/cube_asset.rs`:

```rust
//! The one check that `assets/cube.gltf` is a file Bevy can actually read.
//!
//! A world-shape test cannot reach this: a wrong sub-asset label, a malformed
//! buffer, or an asset root that differs under `cargo test` all leave a
//! perfectly-shaped world and an empty screen. Needs a real device only because
//! `Assets<Mesh>` comes from the render plugins.

use bevy::prelude::*;

#[test]
fn the_cube_asset_loads_as_a_mesh() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(16, 16);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size);
    app.finish();
    app.cleanup();

    let handle: Handle<Mesh> = app
        .world()
        .resource::<AssetServer>()
        .load("cube.gltf#Mesh0/Primitive0");

    // Asset loading is asynchronous, so this polls rather than assuming a
    // frame count.
    let mut loaded = None;
    for updates in 1..=300 {
        app.update();
        if let Some(mesh) = app.world().resource::<Assets<Mesh>>().get(&handle) {
            loaded = Some((updates, mesh.count_vertices(), mesh.indices().map(|i| i.len())));
            break;
        }
    }

    let state = app.world().resource::<AssetServer>().load_state(&handle);
    let (updates, vertices, indices) =
        loaded.unwrap_or_else(|| panic!("cube.gltf never loaded; load state = {state:?}"));
    eprintln!("cube.gltf loaded after {updates} update(s)");
    assert_eq!(vertices, 24, "six faces of four corners, hard edges");
    assert_eq!(indices, Some(36), "two triangles per face");
}
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p sway-nodes --lib mesh_asset && cargo test -p sway-app --test cube_asset`
Expected: PASS both. A `load state = Failed(..)` panic naming the path means the asset root did not resolve — check that `cube.gltf` is in `crates/sway-app/assets/`, not the workspace root.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-nodes/src/mesh_asset.rs crates/sway-nodes/src/lib.rs \
        crates/sway-nodes/Cargo.toml crates/sway-app/assets/cube.gltf \
        crates/sway-app/tests/cube_asset.rs
git rm --cached crates/sway-nodes/src/mesh.rs 2>/dev/null; true
git commit -m "feat(nodes): MeshAsset loads a mesh from a path

Adds the one mesh asset the MVP scene needs. Deletes mesh.rs's
geometry_to_mesh with it — D1 puts procedural geometry past the MVP, and
that function was sway-nodes' only use of sway-geo."
```

---

### Task 6: `PbrMaterial`, `MaterialOut` and the `MaterialFrom` wire

Spec M5-3: a material is its own node, and sharing one across two meshes is a visible fan-out.

**Files:**
- Create: `crates/sway-nodes/src/pbr_material.rs`
- Delete: `crates/sway-nodes/src/material.rs`
- Modify: `crates/sway-nodes/src/lib.rs`

**Interfaces:**
- Consumes: `field_wire!`, `assert_writes_only_on_change`, `sway_graph::EditorPos`.
- Produces: `pub struct PbrMaterial { pub base_color: Vec3, pub emissive: Vec3, pub metallic: f32, pub roughness: f32 }` (doc name `"PbrMaterial"`, requires `MaterialOut` + `EditorPos`); `pub struct MaterialOut(pub Handle<StandardMaterial>)` — produced, never authorable; `pub fn sync_pbr_materials(..)` in `Update`; wire `MaterialFrom` (`"material"`), `MaterialOut => MeshMaterial3d<StandardMaterial>`.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-nodes/src/pbr_material.rs` with its test module only:

```rust
//! `PbrMaterial` — a material as its own node.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::propagate_of;

    fn material_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<StandardMaterial>();
        app.add_systems(Update, sync_pbr_materials);
        app
    }

    #[test]
    fn a_material_node_publishes_a_handle_to_its_own_asset() {
        let mut app = material_app();
        let node = app
            .world_mut()
            .spawn(PbrMaterial {
                base_color: Vec3::new(0.6, 0.7, 0.9),
                metallic: 0.25,
                ..default()
            })
            .id();

        app.update();

        let handle = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_ne!(handle, Handle::default(), "an asset was created");
        let material = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .expect("the handle resolves");
        assert_eq!(material.metallic, 0.25);
    }

    #[test]
    fn editing_a_material_mutates_the_asset_in_place() {
        // In place, not replaced: every mesh already holding this handle must
        // see the edit, which is the whole reason a material is its own node.
        let mut app = material_app();
        let node = app.world_mut().spawn(PbrMaterial::default()).id();
        app.update();
        let before = app.world().get::<MaterialOut>(node).expect("required").0.clone();

        app.world_mut().get_mut::<PbrMaterial>(node).expect("present").metallic = 1.0;
        app.update();

        let after = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_eq!(before, after, "the handle must not change under an edit");
        assert_eq!(
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(&after)
                .map(|m| m.metallic),
            Some(1.0)
        );
    }

    #[test]
    fn the_material_wire_hands_the_same_handle_to_two_meshes() {
        let mut app = material_app();
        let node = app.world_mut().spawn(PbrMaterial::default()).id();
        app.update();

        let a = app
            .world_mut()
            .spawn(MeshMaterial3d::<StandardMaterial>::default())
            .id();
        let b = app
            .world_mut()
            .spawn(MeshMaterial3d::<StandardMaterial>::default())
            .id();
        propagate_of::<MaterialFrom>(app.world_mut(), node, a);
        propagate_of::<MaterialFrom>(app.world_mut(), node, b);

        let expected = app.world().get::<MaterialOut>(node).expect("required").0.clone();
        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(a).map(|m| m.0.clone()),
            Some(expected.clone())
        );
        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(b).map(|m| m.0.clone()),
            Some(expected)
        );
    }

    #[test]
    fn the_material_wire_never_writes_an_equal_value() {
        let mut assets = Assets::<StandardMaterial>::default();
        let one = assets.add(StandardMaterial::default());
        let two = assets.add(StandardMaterial::default());
        assert_writes_only_on_change::<MaterialFrom>(
            MaterialOut(one),
            MaterialOut(two),
            MeshMaterial3d::<StandardMaterial>::default(),
        );
    }

    #[test]
    fn material_parameters_reach_the_standard_material() {
        // Carried over from the deleted material.rs.
        let material = PbrMaterial {
            base_color: Vec3::ONE,
            emissive: Vec3::ZERO,
            metallic: 0.25,
            roughness: 0.75,
        }
        .to_standard_material();
        assert_eq!(material.base_color, Color::srgb(1.0, 1.0, 1.0));
        assert_eq!(material.metallic, 0.25);
        assert_eq!(material.perceptual_roughness, 0.75);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib pbr_material`
Expected: FAIL to compile — `cannot find type PbrMaterial`.

- [ ] **Step 3: Write the component, the outlet, the system and the wire**

Prepend to `crates/sway-nodes/src/pbr_material.rs`:

```rust
use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::EditorPos;

use crate::field_wire::field_wire;

/// A PBR material as a node. Colours are `Vec3` rather than `Color` because
/// roadmap D5 makes every colour inlet a `Vec3` wire, and the field a wire
/// writes has to be the type the wire carries. They are read as sRGB — what an
/// author types — and converted on the way to the asset.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(MaterialOut, EditorPos)]
pub struct PbrMaterial {
    pub base_color: Vec3,
    pub emissive: Vec3,
    pub metallic: f32,
    pub roughness: f32,
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color: Vec3::splat(0.8),
            emissive: Vec3::ZERO,
            metallic: 0.0,
            roughness: 0.5,
        }
    }
}

impl PbrMaterial {
    pub fn to_standard_material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: Color::srgb(self.base_color.x, self.base_color.y, self.base_color.z),
            emissive: LinearRgba::rgb(self.emissive.x, self.emissive.y, self.emissive.z),
            metallic: self.metallic,
            perceptual_roughness: self.roughness,
            ..default()
        }
    }
}

/// The outlet, in the sense of architecture §2: an entity is a material
/// producer because it has one of these. Not authorable — a handle has no
/// business round-tripping through a document.
#[derive(Component, Default, Debug, Clone, PartialEq)]
pub struct MaterialOut(pub Handle<StandardMaterial>);

/// An ordinary `Changed<T>` system. The comparison the "never write an equal
/// value" rule asks for happens upstream, on `PbrMaterial` itself: this body
/// only runs when that component actually changed, so the asset write is
/// already guarded.
pub fn sync_pbr_materials(
    mut assets: ResMut<Assets<StandardMaterial>>,
    mut nodes: Query<(&PbrMaterial, &mut MaterialOut), Changed<PbrMaterial>>,
) {
    for (node, mut out) in &mut nodes {
        let desired = node.to_standard_material();
        match assets.get_mut(&out.0) {
            // Mutating in place is what makes sharing work: every mesh already
            // holding this handle picks the edit up.
            Some(existing) => *existing = desired,
            None => out.set_if_neq(MaterialOut(assets.add(desired))),
        }
    }
}

field_wire!(
    /// Hands a material node's asset to a mesh. Sourced from `MaterialOut`
    /// rather than from `MeshMaterial3d` so the editor's legality rule stays
    /// exact — every mesh carries a `MeshMaterial3d`, and sourcing from that
    /// would make every mesh look like a legal material producer.
    MaterialFrom / DrivesMaterial,
    MaterialOut => MeshMaterial3d<StandardMaterial>,
    "material",
    |t| &mut t.0,
    |s| s.0.clone()
);
```

- [ ] **Step 4: Register it, and retire `material.rs`**

In `crates/sway-nodes/src/lib.rs`: replace `mod material;` with `mod pbr_material;` and `pub use material::*;` with `pub use pbr_material::*;`, then inside `WireNodesPlugin::build`:

```rust
        sway_graph::register_wire::<MaterialFrom>(app);
        sway_graph::register_authorable::<PbrMaterial>(app, "PbrMaterial");
        app.add_systems(
            bevy_app::Update,
            sync_pbr_materials
                .run_if(bevy_ecs::prelude::resource_exists::<Assets<StandardMaterial>>),
        );
```

Extend the authorable list in the plugin test with `"PbrMaterial"`.

```bash
git rm crates/sway-nodes/src/material.rs
```

Its `standard_material` free function is now `PbrMaterial::to_standard_material`, and its test moved with it in Step 1.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-nodes`
Expected: PASS. If `editing_a_material_mutates_the_asset_in_place` reports a changed handle, `sync_pbr_materials` took the `None` arm on the second pass — check it calls `assets.get_mut(&out.0)`, not `assets.get(..)` on a fresh handle.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes/src/pbr_material.rs crates/sway-nodes/src/lib.rs
git commit -m "feat(nodes): PbrMaterial as its own node, wired into meshes

Architecture §6: materials are wired, not assigned, so sharing one across
two meshes is a visible fan-out rather than duplicated fields. Absorbs
material.rs's standard_material as PbrMaterial::to_standard_material."
```

---

### Task 7: `SceneCamera` and the lights

**Files:**
- Rewrite: `crates/sway-nodes/src/scene.rs`
- Modify: `crates/sway-nodes/src/lib.rs`

**Interfaces:**
- Consumes: `sway_graph::{EditorPos, register_authorable}`.
- Produces: `pub struct SceneCamera` (marker; doc name `"SceneCamera"`, requires `Camera3d` + `EditorPos`). `DirectionalLight` and `PointLight` become authorable under their own names.

- [ ] **Step 1: Write the failing test**

Replace the whole of `crates/sway-nodes/src/scene.rs` with its test module only:

```rust
//! The camera and the lights, as nodes.

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

    #[test]
    fn a_scene_camera_brings_a_working_camera_with_it() {
        // The render target is not set here: headless::retarget_cameras points
        // every camera at the viewport texture each Update, which is the whole
        // of "SceneCamera produces Camera3d + RenderTarget".
        let mut world = World::new();
        let entity = world.spawn(SceneCamera).id();

        assert!(world.get::<Camera3d>(entity).is_some());
        assert!(world.get::<Camera>(entity).is_some());
        assert!(world.get::<Projection>(entity).is_some());
        assert!(world.get::<Transform>(entity).is_some(), "authored by the document");
    }

    #[test]
    fn the_camera_and_both_lights_are_authorable() {
        let mut app = App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(crate::WireNodesPlugin);

        let registry = app.world().resource::<sway_graph::ComponentDocRegistry>();
        for name in ["SceneCamera", "DirectionalLight", "PointLight"] {
            assert!(registry.by_name(name).is_some(), "{name} must be authorable");
        }
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --lib scene`
Expected: FAIL to compile — `cannot find type SceneCamera`.

- [ ] **Step 3: Write the marker**

Prepend to `crates/sway-nodes/src/scene.rs`:

```rust
use bevy::prelude::*;
use sway_graph::EditorPos;

/// The scene's camera, as opposed to M7's editor camera. A bare marker: the
/// render target is set by `sway_runtime::headless::retarget_cameras`, and
/// field of view and clear colour stay at Bevy's defaults until something asks
/// otherwise. What this component carries is identity — which of the cameras in
/// the world is the one the show looks through.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Camera3d, EditorPos)]
pub struct SceneCamera;
```

The old file's `transform` and `rgb` helpers are gone: D1 leaves them with no future caller, and the roadmap lists them as deleted.

- [ ] **Step 4: Register all three**

In `crates/sway-nodes/src/lib.rs`, inside `WireNodesPlugin::build`:

```rust
        sway_graph::register_authorable::<SceneCamera>(app, "SceneCamera");
        // Bevy's own types, registered directly: both already carry
        // #[reflect(Component, Default)] and both already require Transform.
        // #[require(EditorPos)] cannot be added to a foreign type, so a light
        // with no authored EditorPos lands on the canvas's fallback grid.
        sway_graph::register_authorable::<DirectionalLight>(app, "DirectionalLight");
        sway_graph::register_authorable::<PointLight>(app, "PointLight");
```

The final expected list in `the_plugin_registers_every_authorable_component`:

```rust
        assert_eq!(
            names,
            vec![
                "DirectionalLight",
                "EditorPos",
                "FloatOut",
                "Lfo",
                "Math",
                "MeshAsset",
                "PbrMaterial",
                "PointLight",
                "Remap",
                "SceneCamera",
                "Transform",
                "Vec3",
                "Vec3Out",
            ]
        );
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-nodes`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes/src/scene.rs crates/sway-nodes/src/lib.rs
git commit -m "feat(nodes): SceneCamera and the lights become authorable

Bevy's light types register directly; the camera gets a marker so M7 can
tell the scene camera from the editor camera. Drops scene.rs's transform
and rgb helpers, dead since D1."
```

---

### Task 8: The document authors the whole scene

Everything the app sets up in Rust goes away.

**Files:**
- Rewrite: `crates/sway-app/assets/demo.sway.ron`, `crates/sway-app/tests/demo_document.rs`
- Modify: `crates/sway-app/src/main.rs`
- Delete: `crates/sway-app/src/scene.rs`, `crates/sway-app/src/demo_assets.rs`, `crates/sway-app/src/lib.rs`

**Interfaces:**
- Consumes: every component and wire from Tasks 2-7.
- Produces: a demo document with ten entities — `camera`, `sun`, `mat`, `lfoA`, `lfoB`, `vec3A`, `vec3B`, `cubeA`, `cubeB`, `group`.

- [ ] **Step 1: Write the failing test**

Replace `crates/sway-app/tests/demo_document.rs` entirely:

```rust
//! The demo document's only non-visual coverage.
//!
//! Parses and applies the real `assets/demo.sway.ron`, then asserts the world
//! against the document's own comment-drawn diagram. A renamed short name, a
//! malformed payload, or a dropped `register_authorable`/`register_wire` call
//! would otherwise leave the suite green and only surface when a human ran the
//! app.
//!
//!   lfoA ──amplitude──▶ lfoB
//!   lfoA ──vec3.y────▶ vec3A ──translation──▶ cubeA ─┐
//!   lfoB ──vec3.y────▶ vec3B ──translation──▶ cubeB ─┤─parent─▶ group
//!   mat  ──material──▶ cubeA, cubeB

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use sway_graph::project::{DocId, to_document};
use sway_nodes::{AmplitudeFrom, MaterialFrom, MaterialOut, TranslationFrom, Vec3YFrom};

const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

fn demo_app() -> App {
    let mut app = App::new();
    app.add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin));
    app
}

fn entity_named(world: &mut World, id: &str) -> Entity {
    world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .find(|(_, doc_id)| doc_id.0 == id)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("demo document has no entity \"{id}\""))
}

#[test]
fn demo_document_parses() {
    sway_graph::project::parse(DEMO_DOCUMENT).expect("assets/demo.sway.ron parses");
}

#[test]
fn demo_document_loads_and_reconciles_cleanly() {
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();

    let diagnostics = sway_graph::project::apply(app.world_mut(), &document);

    assert!(
        diagnostics.is_clean(),
        "the demo document should be clean against the current registry, got: {:?}",
        diagnostics.items
    );

    let world = app.world_mut();
    let mut ids: Vec<String> = world.query::<&DocId>().iter(world).map(|id| id.0.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "camera".to_string(),
            "cubeA".to_string(),
            "cubeB".to_string(),
            "group".to_string(),
            "lfoA".to_string(),
            "lfoB".to_string(),
            "mat".to_string(),
            "sun".to_string(),
            "vec3A".to_string(),
            "vec3B".to_string(),
        ],
        "exactly the demo's 10 entities should carry a DocId"
    );

    let lfo_a = entity_named(world, "lfoA");
    let lfo_b = entity_named(world, "lfoB");
    let vec3_a = entity_named(world, "vec3A");
    let vec3_b = entity_named(world, "vec3B");
    let cube_a = entity_named(world, "cubeA");
    let cube_b = entity_named(world, "cubeB");
    let group = entity_named(world, "group");
    let material = entity_named(world, "mat");
    let camera = entity_named(world, "camera");
    let sun = entity_named(world, "sun");

    assert_eq!(world.get::<AmplitudeFrom>(lfo_b).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_a).map(|w| w.0), Some(lfo_a));
    assert_eq!(world.get::<Vec3YFrom>(vec3_b).map(|w| w.0), Some(lfo_b));
    assert_eq!(world.get::<TranslationFrom>(cube_a).map(|w| w.0), Some(vec3_a));
    assert_eq!(world.get::<TranslationFrom>(cube_b).map(|w| w.0), Some(vec3_b));
    assert_eq!(world.get::<MaterialFrom>(cube_a).map(|w| w.0), Some(material));
    assert_eq!(world.get::<MaterialFrom>(cube_b).map(|w| w.0), Some(material));
    assert_eq!(world.get::<ChildOf>(cube_a).map(|c| c.parent()), Some(group));
    assert_eq!(world.get::<ChildOf>(cube_b).map(|c| c.parent()), Some(group));

    // D4: the document names one component per node and Bevy supplies the rest.
    // None of these appear in the file.
    assert!(world.get::<sway_nodes::FloatOut>(lfo_a).is_some(), "Lfo requires FloatOut");
    assert!(world.get::<Mesh3d>(cube_a).is_some(), "MeshAsset requires Mesh3d");
    assert!(world.get::<Visibility>(cube_a).is_some(), "MeshAsset requires Visibility");
    assert!(world.get::<Transform>(cube_a).is_some(), "Mesh3d requires Transform");
    assert!(world.get::<MaterialOut>(material).is_some(), "PbrMaterial requires MaterialOut");
    assert!(world.get::<Camera3d>(camera).is_some(), "SceneCamera requires Camera3d");
    assert!(world.get::<Transform>(sun).is_some(), "DirectionalLight requires Transform");
}

#[test]
fn demo_document_survives_a_reload() {
    // The hot-reload path, and the sharp case for Task 1's exemption: on the
    // second apply the required companions are already present and still
    // unnamed, which is exactly what the removal pass looks for.
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();
    sway_graph::project::apply(app.world_mut(), &document);
    let cube = entity_named(app.world_mut(), "cubeA");

    sway_graph::project::apply(app.world_mut(), &document);

    assert!(app.world().get::<Mesh3d>(cube).is_some(), "Mesh3d survived the reload");
    assert!(app.world().get::<Transform>(cube).is_some(), "Transform survived the reload");
}

#[test]
fn demo_document_round_trips_through_the_world() {
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("parses");
    let mut app = demo_app();
    sway_graph::project::apply(app.world_mut(), &document);
    let once = to_document(app.world_mut());

    let mut second = demo_app();
    let diagnostics = sway_graph::project::apply(second.world_mut(), &once);
    assert!(diagnostics.is_clean(), "re-apply of emitted doc: {:?}", diagnostics.items);
    let twice = to_document(second.world_mut());

    assert_eq!(once, twice);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-app --test demo_document`
Expected: FAIL — the id list assertion reports 7 entities, not 10.

- [ ] **Step 3: Write the document**

Replace `crates/sway-app/assets/demo.sway.ron` entirely. The quaternions are `Transform::looking_at`'s output for those two positions, computed once — a document cannot call `looking_at`:

```ron
// The wire-model demo, as a document. Nothing about this scene is set up in
// Rust: the camera, the light, the mesh and the material are all nodes.
//
//   lfoA ──amplitude──▶ lfoB
//   lfoA ──vec3.y────▶ vec3A ──translation──▶ cubeA ─┐
//   lfoB ──vec3.y────▶ vec3B ──translation──▶ cubeB ─┤─parent─▶ group
//   mat  ──material──▶ cubeA, cubeB
//
// Each node names one component and lets `#[require]` supply the rest (roadmap
// D4), which is why there is no `FloatOut` here and no `Transform` on the
// cubes — the cubes' translation is wired, and an authored one would be
// overwritten on the first tick anyway.
//
// The cubes' x offsets live in the `Vec3` nodes because `translation` is
// written whole (roadmap D5).
Project(
    version: 1,
    entities: [
        Entity(
            id: "camera",
            components: {
                "SceneCamera": (),
                "Transform": (
                    translation: (0.0, 1.5, 5.0),
                    rotation: (-0.14521316, 0.0, 0.0, 0.98940045),
                    scale: (1.0, 1.0, 1.0),
                ),
                "EditorPos": ((420.0, -120.0)),
            },
        ),
        Entity(
            id: "sun",
            components: {
                "DirectionalLight": (illuminance: 6000.0),
                "Transform": (
                    translation: (4.0, 8.0, 4.0),
                    rotation: (-0.424708, 0.339851, 0.175920, 0.820473),
                    scale: (1.0, 1.0, 1.0),
                ),
                "EditorPos": ((420.0, -20.0)),
            },
        ),
        Entity(
            id: "mat",
            components: {
                "PbrMaterial": (base_color: (0.6, 0.7, 0.9), metallic: 0.1, roughness: 0.4),
                "EditorPos": ((180.0, 260.0)),
            },
        ),
        Entity(
            id: "lfoA",
            components: {
                "Lfo": (beats: 8.0, shape: Sine, phase: 0.0, amplitude: 0.5),
                "EditorPos": ((-460.0, 40.0)),
            },
        ),
        Entity(
            id: "lfoB",
            components: {
                "Lfo": (beats: 2.0, shape: Sine, phase: 0.0, amplitude: 0.0),
                "EditorPos": ((-460.0, 160.0)),
            },
            wires: { "amplitude": "lfoA" },
        ),
        Entity(
            id: "vec3A",
            components: {
                "Vec3": (x: -0.8, y: 0.0, z: 0.0),
                "EditorPos": ((-220.0, 40.0)),
            },
            wires: { "vec3.y": "lfoA" },
        ),
        Entity(
            id: "vec3B",
            components: {
                "Vec3": (x: 0.8, y: 0.0, z: 0.0),
                "EditorPos": ((-220.0, 160.0)),
            },
            wires: { "vec3.y": "lfoB" },
        ),
        Entity(
            id: "cubeA",
            components: {
                "MeshAsset": (path: "cube.gltf#Mesh0/Primitive0"),
                "EditorPos": ((20.0, 40.0)),
            },
            wires: { "translation": "vec3A", "material": "mat", "parent": "group" },
        ),
        Entity(
            id: "cubeB",
            components: {
                "MeshAsset": (path: "cube.gltf#Mesh0/Primitive0"),
                "EditorPos": ((20.0, 160.0)),
            },
            wires: { "translation": "vec3B", "material": "mat", "parent": "group" },
        ),
        Entity(
            id: "group",
            components: {
                "Transform": (),
                "EditorPos": ((260.0, 100.0)),
            },
        ),
    ],
)
```

- [ ] **Step 4: Strip the Rust-side scene setup**

```bash
git rm crates/sway-app/src/scene.rs crates/sway-app/src/demo_assets.rs crates/sway-app/src/lib.rs
```

`lib.rs` existed only so a test could reach `demo_assets`; with that gone, the crate is a binary again.

In `crates/sway-app/src/main.rs`:

1. Delete the first line, `use sway_app::demo_assets;`.
2. Delete `mod scene;` and `use scene::setup_scene;`.
3. Remove `demo_assets::DemoAssetsPlugin,` from the `add_plugins` tuple.
4. Replace the unconditional `.add_systems(Startup, load_project)` in that same builder chain — it moves into the demo dispatch below.
5. Replace the whole camera-collision comment and the `match demo` arm for `None`:

```rust
        // Camera-collision hazard: the project document now authors its own
        // camera (M5), and each render-spike demo spawns one of its own, and
        // Bevy renders every camera with the same (default) order to the same
        // target -- the last one drawn wins and the rest are invisibly
        // overdrawn. So the project document loads only when no demo is
        // selected. Before M5 the document loaded unconditionally, which is why
        // the sprite-depth spike found a stray cube drifting through its
        // screenshots.
        //   - `point-cloud` spawns its own camera (required: it carries
        //     `NoIndirectDrawing`, which the point-cloud pipeline needs).
        //   - `sprites` spawns its own dedicated camera via
        //     `sprite_layer::spawn_demo_camera`.
        //   - `scatter` spawns no camera at all: it is compute + readback
        //     only, proven by a log line, not by anything on screen.
        //   - `all` reuses the point cloud's camera for the sprite layers
        //     too (skipping `spawn_demo_camera`) rather than spawning a
        //     second one.
        match demo {
            None => {
                app.add_systems(Startup, load_project);
            }
```

leaving the other arms untouched.

- [ ] **Step 5: Run the whole workspace**

Run: `cargo test --workspace`
Expected: PASS. An `UnknownComponent` diagnostic naming `"DemoCube"` means the document still carries a stale entry; an `UnknownWire` naming `"material"` means Task 6's `register_wire::<MaterialFrom>` is missing.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-app/assets/demo.sway.ron crates/sway-app/tests/demo_document.rs \
        crates/sway-app/src/main.rs
git commit -m "feat(app): the demo document authors its own scene

Camera, light, material and both cubes are nodes now. Deletes DemoCube,
setup_scene and sway-app's lib target, and stops loading the project
document when a --demo spike is selected -- with a camera in the document
the two would fight over one render target."
```

---

### Task 9: The readback test

Spec M5-4. Architecture §9's "no pixel-diff tests" stands for how the scene *looks*; this answers a different question — whether anything reached the screen at all.

**Files:**
- Create: `crates/sway-app/tests/demo_renders.rs`

**Interfaces:**
- Consumes: the demo document from Task 8, `sway_runtime::headless::build_app`, `sway_gpu::{GpuContext, ViewportTexture}`.
- Produces: nothing other tasks use.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-app/tests/demo_renders.rs`:

```rust
//! Does the demo document actually put pixels on the screen?
//!
//! Architecture §9 says rendering is verified by eye, and that stands for how
//! the scene looks. It does not cover whether the glTF resolves at all: a wrong
//! sub-asset label, an asset root that differs under `cargo test`, or a mesh
//! whose material never arrives all produce a world of exactly the right shape
//! and an empty frame. This asserts only "lit geometry rendered", not what it
//! looks like — the by-eye run is still what judges that.
//!
//! Follows the readback precedent in `sway-runtime/tests/
//! sprite_depth_interpenetration.rs` and `sway-runtime/src/headless.rs`.

use bevy::prelude::*;
use sway_gpu::wgpu;

const VIEWPORT: u32 = 128;
const DEMO_DOCUMENT: &str = include_str!("../assets/demo.sway.ron");

/// Reads the whole viewport back as RGBA8 pixels.
///
/// `bytes_per_row` must be padded to `COPY_BYTES_PER_ROW_ALIGNMENT` (256);
/// wgpu does not do it for you. Mapping is async, so `device.poll` has to drive
/// the callback or the recv below hangs forever.
fn read_pixels(gpu: &sway_gpu::GpuContext, viewport: &sway_gpu::ViewportTexture) -> Vec<[u8; 4]> {
    let bytes_per_pixel = 4u32;
    let unpadded = VIEWPORT * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo renders readback"),
        size: u64::from(padded) * u64::from(VIEWPORT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: viewport.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(VIEWPORT),
            },
        },
        wgpu::Extent3d {
            width: VIEWPORT,
            height: VIEWPORT,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll failed");
    rx.recv()
        .expect("map_async callback never ran")
        .expect("buffer mapping failed");

    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((VIEWPORT * VIEWPORT) as usize);
    for row in 0..VIEWPORT {
        let start = (row * padded) as usize;
        for col in 0..VIEWPORT {
            let at = start + (col * bytes_per_pixel) as usize;
            pixels.push([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        }
    }
    drop(data);
    readback.unmap();
    pixels
}

/// The cubes are pale blue (`base_color: (0.6, 0.7, 0.9)`) and lit; the default
/// clear colour is a dark neutral grey (43, 44, 47). "Blue clearly ahead of red,
/// and bright" is true of the cube and of nothing else in this frame.
fn is_cube(pixel: [u8; 4]) -> bool {
    pixel[2] > 90 && pixel[2] as i16 - pixel[0] as i16 > 15
}

#[test]
fn the_demo_document_renders_its_cubes() {
    let gpu = sway_gpu::GpuContext::new(None);
    let size = UVec2::new(VIEWPORT, VIEWPORT);
    let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
    let mut app = sway_runtime::headless::build_app(&gpu, &viewport, size);
    app.add_plugins((sway_graph::WiresPlugin, sway_nodes::WireNodesPlugin));
    app.finish();
    app.cleanup();

    // The document is applied directly rather than through ProjectPlugin's
    // asset load: this test is about the scene it describes, not about the
    // .ron's own loading path, which demo_document.rs already covers.
    let document = sway_graph::project::parse(DEMO_DOCUMENT).expect("parses");
    let diagnostics = sway_graph::project::apply(app.world_mut(), &document);
    assert!(diagnostics.is_clean(), "{:?}", diagnostics.items);

    // A bounded poll, not a fixed count. Two independent asynchronous things
    // have to finish: bevy_core_pipeline's upscaling pipeline compiles (until
    // it does, the viewport is cleared to the wrong colour with no validation
    // error), and the glTF loads off disk. Cold caches in this codebase have
    // needed as many as 60 updates for the first alone.
    const MAX_UPDATES: u32 = 400;
    let total = (VIEWPORT * VIEWPORT) as usize;
    let mut cube_pixels = 0;
    let mut converged = None;
    for updates in 1..=MAX_UPDATES {
        app.update();
        cube_pixels = read_pixels(&gpu, &viewport).into_iter().filter(|p| is_cube(*p)).count();
        // Two cubes of 1 unit at ~5 units from a 45-degree camera cover a few
        // percent of the frame. 1% is far above stray-pixel noise and far below
        // what the real coverage should be.
        if cube_pixels * 100 > total {
            converged = Some(updates);
            break;
        }
    }

    let updates = converged.unwrap_or_else(|| {
        panic!(
            "no lit cube pixels after {MAX_UPDATES} updates ({cube_pixels} of {total} matched). \
             Either cube.gltf never loaded (check the path and its #Mesh0/Primitive0 label), \
             the material never reached the mesh (check the MaterialFrom wire), the light or \
             camera did not spawn, or nothing rendered at all."
        )
    });
    eprintln!("demo document rendered {cube_pixels}/{total} cube pixels after {updates} update(s)");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p sway-app --test demo_renders -- --nocapture`
Expected: PASS, with a line reporting the pixel count and update count. This test needs a real GPU adapter; it will not run in a headless CI container without one.

- [ ] **Step 3: If it fails, work down the panic message's list**

Take the failure modes in order — the cheapest checks first:
1. `cargo test -p sway-app --test cube_asset` (Task 5) — if that fails too, it is the asset, not the scene.
2. Add a temporary `app.world_mut().spawn((Mesh3d(...), MeshMaterial3d(...), Transform::default()))` beside the document and see whether *that* renders; if it does, the document's wiring is at fault, not the render setup.
3. Log `app.world().query::<&MeshMaterial3d<StandardMaterial>>()` handles after 100 updates to see whether `MaterialFrom` ever propagated — the graph tick runs in `FixedUpdate`, so it needs enough wall time to have fired at least once.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-app/tests/demo_renders.rs
git commit -m "test(app): prove the demo document reaches the screen

The one thing a world-shape assertion cannot reach: a wrong asset path or
an unresolved material leaves a perfectly-shaped world and an empty frame."
```

---

### Task 10: By-eye verification and the findings report

**Files:**
- Create: `docs/superpowers/reports/YYYY-MM-DD-m5-minimal-scene-slice-findings.md` (use the completion date)
- Modify: `docs/superpowers/specs/2026-07-25-sway-design.md`

- [ ] **Step 1: Run the full suite**

Run: `cargo test --workspace`
Expected: PASS. Record the counts.

- [ ] **Step 2: Run the app and look at it**

Run: `cargo run -p sway-app -- --editor --windowed`
Expected: two pale blue cubes side by side, bobbing on Y at different rates, lit from above-right, with the graph canvas showing ten nodes and the wires between them. Take a screenshot.

Then check the spike demos still behave: `cargo run -p sway-app -- --demo sprite-depth`. Expected: the spike's own scene with **no** stray cubes — the document no longer loads under `--demo`.

- [ ] **Step 3: Write the findings report**

Follow `docs/superpowers/reports/2026-08-10-sprite-depth-spike-findings.md`'s shape. Cover, at minimum:
- What was built, against the exit criterion: "the demo document authors its own camera, light and PBR cube. No Rust-side scene setup remains anywhere."
- The `apply` change from Task 1 — that `#[require]` and the document's removal pass were in direct conflict, and how it was resolved. This is the plan's one genuine discovery and the thing M6 most needs to know.
- Whether the two facts flagged in the spec's "verify before implementing" list held.
- The screenshot, and what it shows.
- What M6 and M8 inherit: the palette must insert `EditorPos` itself for foreign types like `DirectionalLight`, since `#[require(EditorPos)]` cannot be added to them.
- Anything that surprised you.

- [ ] **Step 4: Update the roadmap's status line**

In `docs/superpowers/specs/2026-07-25-sway-design.md`, change the status line to reflect that M5 is complete and M6/M7 are next, and add a link to the findings report beneath the M5 section heading, matching how the design spec is linked there.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/reports docs/superpowers/specs/2026-07-25-sway-design.md
git commit -m "docs: M5 findings — the minimal scene slice"
```

---

## Self-review

**Spec coverage.** Every section of `2026-08-10-m5-minimal-scene-slice-design.md` maps to a task: M5-1 → Task 5; M5-2 → Tasks 2-4 (nine new wires: `Vec3X/Y/ZFrom`, `MathA/BFrom`, `RemapInputFrom`, `Translation/Rotation/ScaleFrom`) plus `MaterialFrom` in Task 6; M5-3 → Task 6; M5-4 → Task 9. The node-set table is covered component by component across Tasks 2-7; the wire table by Tasks 2-6; the demo document by Task 8; deletions by Tasks 4 (`switch_value`), 5 (`mesh.rs`, the `sway-geo` dependency), 6 (`material.rs`), 7 (`scene.rs`'s helpers) and 8 (`DemoCube`, `setup_scene`, `lib.rs`, the `load_project` gate); the testing section by the test steps in every task plus Task 10's by-eye run.

**One item is not in the spec:** Task 1. Writing the plan surfaced that `apply_components` deliberately removes required components, which would have made D4 unworkable in the document path — the spec assumed `#[require]` and the document format composed cleanly, and they do not. Task 1 is the smallest fix that keeps both semantics.

**Interfaces.** `Vec3Value` (Rust) / `"Vec3"` (document) is the one deliberate name split, stated in Tasks 2 and 8. Wire document names are fixed once, in the task that creates each wire, and used verbatim in the demo document: `"vec3.x"`, `"vec3.y"`, `"vec3.z"`, `"math.a"`, `"math.b"`, `"remap.input"`, `"translation"`, `"rotation"`, `"scale"`, `"material"`, plus the pre-existing `"amplitude"` and `"parent"`. `PbrMaterial::to_standard_material` is named identically in Task 6's test and implementation. `MaterialOut` is `pub` and used by Task 8's test.

**The authorable-name list** is asserted in `the_plugin_registers_every_authorable_component` and grows in Tasks 2, 4, 5, 6 and 7; each of those tasks states the expected list at that point, and Task 7 states the final thirteen.
