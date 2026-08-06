# Project Format and Hot Reload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author a set by editing a RON file with the app running — a document
of entities, components and wires, loaded through `bevy_asset` and reconciled
into the live world by document id on every save.

**Architecture:** A pure `ProjectDoc` parser that never sees a `World`; two
registries mapping short names to reflect type data and to wire insert/read
functions; a four-pass applier that reconciles by `DocId` and then sets
`TopologyDirty`, letting the existing rebuild produce the new order. A
whole-document emitter exists to prove the format complete via round-trip. The
read-only inspector is the same reflect walk, rendered.

**Tech Stack:** Rust edition 2024, Bevy `=0.19.0` (pinned), `bevy_reflect`
serde, `ron` 0.12, `bevy_asset` with `file_watcher`, masonry for the inspector
pane.

## Global Constraints

- Bevy is pinned to `=0.19.0` across the workspace. Do not bump it.
- Rust edition is `2024`.
- **`sway-graph` may depend only on `bevy_app`, `bevy_ecs`, `bevy_math`,
  `bevy_reflect`, `bevy_time`, `bevy_transform`, and — added by this plan —
  `bevy_asset`, `ron`, `serde`.** Not the `bevy` facade, not `bevy_render`.
  `crates/sway-graph/Cargo.toml` is the only place this is enforced.
- **No reflection on the tick path.** Everything this plan adds runs at load,
  at reload, or in the editor. `graph_tick` must not gain a single reflect call.
- **A reload never stops the show.** No `unwrap`, no `expect`, no panic on any
  path reachable from a document's contents. Panics are allowed only in
  registration functions, which run at startup.
- Graph tick rate is `120.0` Hz (`TICK_HZ` in `crates/sway-app/src/main.rs:17`).
- Tests live beside the code in `#[cfg(test)] mod tests`, except Task 1's
  characterization tests, which are an integration test.
- Commit after every task. Work on branch `project-format`.
- Spec: `docs/superpowers/specs/2026-08-06-project-format-design.md`.
  Parent: `docs/superpowers/specs/2026-07-25-sway-design.md` §5 (M4).

---

## File Structure

**`sway-graph` — created:**

| File | Responsibility |
|---|---|
| `tests/reflect_ron.rs` | Task 1's characterization tests for reflect ↔ ron |
| `src/project/mod.rs` | Re-exports, `ProjectPlugin` |
| `src/project/doc.rs` | `ProjectDoc`, `EntityDoc`, `ParseError`, `parse` |
| `src/project/registry.rs` | `ComponentDocRegistry`, `register_authorable` |
| `src/project/diagnostics.rs` | `DocId`, `ProjectDiagnostics`, `ItemError` |
| `src/project/apply.rs` | The four-pass applier |
| `src/project/emit.rs` | `to_document`, `to_ron` |
| `src/project/asset.rs` | `ProjectAsset`, `ProjectLoader`, the event systems |

**`sway-graph` — modified:** `src/lib.rs` (exports), `src/registry_wires.rs`
(wire insert/remove/read), `Cargo.toml` (three dependencies).

**`sway-nodes` — modified:** `src/lib.rs` (`register_authorable` calls),
`src/osc.rs` and `src/outputs.rs` (`#[derive(Reflect)]`).

**`sway-app` — created:** `assets/demo.sway.ron` (at the repo root, not in the
crate), `src/demo_assets.rs` (the `DemoCube` marker and its mesh system).
**Modified:** `src/main.rs`, `src/demo_graph.rs` (deleted), `Cargo.toml`.

**`sway-editor` — created:** `src/inspector.rs`. **Modified:**
`src/snapshot.rs` (`inspect`), `src/lib.rs` (pane layout, `selected_entity`).

**`sway-runtime` — modified:** `src/headless.rs` (`AssetPlugin` watch override).

---

### Task 1: Pin what reflect and ron actually do

The design's §10 names three assumptions the whole format rests on: that a
`ron::Value` can drive a reflect deserializer, that a *partial* payload becomes
a whole component through `ReflectDefault`, and that `apply` leaves unnamed
fields alone. Each has a different fallback, so all three are settled before any
format code exists.

**Files:**
- Create: `crates/sway-graph/tests/reflect_ron.rs`
- Modify: `crates/sway-graph/Cargo.toml`, root `Cargo.toml`

**Interfaces:**
- Consumes: nothing.
- Produces: no API. Produces *decisions*: see Step 3.

- [ ] **Step 1: Add the dependencies**

In the root `Cargo.toml`, under `[workspace.dependencies]`, add:

```toml
ron = "0.12"
serde = { version = "1", features = ["derive"] }
```

In `crates/sway-graph/Cargo.toml`, add to `[dependencies]` (leave the existing
comment about the dependency rule in place, and extend it):

```toml
ron.workspace = true
serde.workspace = true
```

- [ ] **Step 2: Write the characterization tests**

Create `crates/sway-graph/tests/reflect_ron.rs`:

```rust
//! What bevy_reflect 0.19 and ron 0.12 actually do together, pinned.
//!
//! The project format (specs/2026-08-06-project-format-design.md §3, §10)
//! rests on these behaviours. They are checked here, against the real
//! libraries, before anything is built on them.

use std::any::TypeId;

use bevy_ecs::component::Component;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer};
use bevy_reflect::{PartialReflect, Reflect, TypeRegistry};
use serde::de::DeserializeSeed;

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
enum Shape {
    #[default]
    Sine,
    Saw,
}

#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, PartialEq)]
struct Osc {
    hz: f32,
    shape: Shape,
    amplitude: f32,
}

impl Default for Osc {
    fn default() -> Self {
        Self { hz: 1.0, shape: Shape::Sine, amplitude: 0.5 }
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    registry.register::<Osc>();
    registry.register::<Shape>();
    registry
}

/// The exact path the loader will take: text -> ron::Value -> partial reflect.
fn payload(text: &str, registry: &TypeRegistry) -> Box<dyn PartialReflect> {
    let value: ron::Value = ron::from_str(text).expect("payload parses as a ron::Value");
    let registration = registry
        .get(TypeId::of::<Osc>())
        .expect("Osc is registered");
    TypedReflectDeserializer::new(registration, registry)
        .deserialize(value)
        .expect("a ron::Value drives a reflect deserializer")
}

fn reflect_component(registry: &TypeRegistry) -> ReflectComponent {
    registry
        .get_type_data::<ReflectComponent>(TypeId::of::<Osc>())
        .expect("#[reflect(Component)] supplies ReflectComponent")
        .clone()
}

/// CLAIM 1: a ron::Value can drive a reflect deserializer at all, and a full
/// payload reconstructs the component exactly.
#[test]
fn a_full_payload_becomes_the_component() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("(hz: 2.0, shape: Saw, amplitude: 0.25)", &registry);

    let mut world = World::new();
    let entity = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(entity), &*value, &registry);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 2.0, shape: Shape::Saw, amplitude: 0.25 })
    );
}

/// CLAIM 2: a PARTIAL payload fills the rest from ReflectDefault. This is what
/// lets a document name one field of Transform. If it fails, the format needs
/// complete payloads everywhere.
#[test]
fn a_partial_payload_fills_the_rest_from_default() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("(hz: 2.0)", &registry);

    let mut world = World::new();
    let entity = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(entity), &*value, &registry);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 2.0, shape: Shape::Sine, amplitude: 0.5 }),
        "unnamed fields come from Default, not from zero"
    );
}

/// CLAIM 3: `apply` on an EXISTING component touches only the named fields.
/// This is what stops a reload from clobbering a field a wire is driving.
#[test]
fn apply_leaves_unnamed_fields_alone() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let value = payload("(hz: 3.0)", &registry);

    let mut world = World::new();
    let entity = world
        .spawn(Osc { hz: 1.0, shape: Shape::Saw, amplitude: 0.9 })
        .id();
    reflect.apply(&mut world.entity_mut(entity), &*value);

    assert_eq!(
        world.get::<Osc>(entity),
        Some(&Osc { hz: 3.0, shape: Shape::Saw, amplitude: 0.9 })
    );
}

/// CLAIM 4: a partial value compares equal to a component whose named fields
/// match. This is the skip-if-unchanged gate; without it every reload marks
/// every component Changed.
#[test]
fn a_partial_value_compares_against_the_live_component() {
    let registry = registry();
    let current = Osc { hz: 3.0, shape: Shape::Saw, amplitude: 0.9 };

    let same = payload("(hz: 3.0)", &registry);
    let different = payload("(hz: 4.0)", &registry);

    assert_eq!(same.reflect_partial_eq(current.as_partial_reflect()), Some(true));
    assert_eq!(different.reflect_partial_eq(current.as_partial_reflect()), Some(false));
}

/// CLAIM 5: a live component serializes back to text the loader can read.
/// The emitter's whole job, in one assertion.
#[test]
fn a_component_round_trips_through_text() {
    let registry = registry();
    let reflect = reflect_component(&registry);
    let original = Osc { hz: 7.5, shape: Shape::Saw, amplitude: 0.125 };

    let mut world = World::new();
    let entity = world.spawn(original).id();

    let entity_ref = world.entity(entity);
    let value = reflect.reflect(entity_ref).expect("component is present");
    let text = ron::to_string(&TypedReflectSerializer::new(
        value.as_partial_reflect(),
        &registry,
    ))
    .expect("a reflected component serializes");

    let back = payload(&text, &registry);
    let restored = world.spawn_empty().id();
    reflect.insert(&mut world.entity_mut(restored), &*back, &registry);

    assert_eq!(world.get::<Osc>(restored), Some(&original));
}

/// AppTypeRegistry is what the applier will read out of the world; check it
/// carries what a plain TypeRegistry does.
#[test]
fn the_app_registry_carries_the_same_type_data() {
    let mut world = World::new();
    world.init_resource::<AppTypeRegistry>();
    world.resource_mut::<AppTypeRegistry>().write().register::<Osc>();

    let registry = world.resource::<AppTypeRegistry>().clone();
    let read = registry.read();
    assert!(read.get_type_data::<ReflectComponent>(TypeId::of::<Osc>()).is_some());
}
```

- [ ] **Step 3: Run the tests, and record the verdicts**

Run: `cargo test -p sway-graph --test reflect_ron`

Expected: all six PASS.

**If `a_full_payload_becomes_the_component` fails on the `ron::Value` step**,
`ron::Value` is not usable as a `Deserializer` here. Do not proceed — change
`EntityDoc` to hold each payload as a `String` (via `ron::value::RawValue`) and
drive `TypedReflectDeserializer` with `ron::Deserializer::from_str` instead.
Every later task then reads a `&str` payload rather than a `ron::Value`; nothing
else changes. Record the change at the top of Task 2.

**If `a_partial_payload_fills_the_rest_from_default` fails**, partial payloads
are not supported on insert. Documents must then name every field of every
component; note it in Task 10's demo document and keep going.

**If `apply_leaves_unnamed_fields_alone` fails**, Task 6 cannot use `apply` and
must insert a fully reconstructed component every time. Note it in Task 6.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-graph/tests/reflect_ron.rs crates/sway-graph/Cargo.toml Cargo.toml
git commit -m "test(graph): pin reflect and ron behaviour before building a format on it"
```

---

### Task 2: The document and its parser

A pure function from text to data. No `World`, no registries, no Bevy — which
is what makes every malformed-input case a plain table test.

**Files:**
- Create: `crates/sway-graph/src/project/mod.rs`, `crates/sway-graph/src/project/doc.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: Task 1's verdict on payload representation.
- Produces:
  - `struct ProjectDoc { pub version: u32, pub entities: Vec<EntityDoc> }`
  - `struct EntityDoc { pub id: String, pub components: BTreeMap<String, ron::Value>, pub wires: BTreeMap<String, String> }`
  - `enum ParseError { Ron(String), UnsupportedVersion(u32), DuplicateId(String) }`
  - `fn parse(text: &str) -> Result<ProjectDoc, ParseError>`
  - `const FORMAT_VERSION: u32 = 1`

- [ ] **Step 1: Write the module and its failing tests**

Create `crates/sway-graph/src/project/doc.rs`:

```rust
//! The document, and the only code that reads text. Spec §2.
//!
//! Deliberately free of `World`, registries and Bevy: a document is data, and
//! every syntax-level failure is decided here so the applier can assume a
//! coherent one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Bumped when the document shape changes incompatibly. An unknown version is
/// rejected rather than guessed at.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Project")]
pub struct ProjectDoc {
    pub version: u32,
    pub entities: Vec<EntityDoc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "Entity")]
pub struct EntityDoc {
    /// Stable identity across reloads, and the entity's `Name` in the world
    /// (spec §2.4). Renaming is a delete plus an add.
    pub id: String,
    /// Short registered component name -> its payload, left unparsed here.
    /// A `BTreeMap` rather than the file's own order: the reader never
    /// rewrites the file, so only the emitter sees this order, and
    /// alphabetical is deterministic.
    #[serde(default)]
    pub components: BTreeMap<String, ron::Value>,
    /// Wire `NAME` -> the id of the producer.
    #[serde(default)]
    pub wires: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The text is not valid RON, or does not match the document shape.
    Ron(String),
    UnsupportedVersion(u32),
    /// Two entities share an id, so nothing in the document can be resolved
    /// unambiguously (spec §4.3: this rejects the whole reload).
    DuplicateId(String),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ron(message) => write!(f, "{message}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "project version {version} is not supported (this build reads {FORMAT_VERSION})"
            ),
            Self::DuplicateId(id) => write!(f, "two entities share the id \"{id}\""),
        }
    }
}

impl core::error::Error for ParseError {}

pub fn parse(text: &str) -> Result<ProjectDoc, ParseError> {
    let doc: ProjectDoc = ron::from_str(text).map_err(|e| ParseError::Ron(e.to_string()))?;
    if doc.version != FORMAT_VERSION {
        return Err(ParseError::UnsupportedVersion(doc.version));
    }
    let mut seen = std::collections::HashSet::new();
    for entity in &doc.entities {
        if !seen.insert(entity.id.as_str()) {
            return Err(ParseError::DuplicateId(entity.id.clone()));
        }
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
Project(
    version: 1,
    entities: [
        Entity(
            id: "lfoA",
            components: {
                // a comment, which RON keeps and the parser ignores
                "Lfo": (beats: 8.0, amplitude: 0.5),
                "FloatOut": (0.0),
            },
            wires: {},
        ),
        Entity(
            id: "cube",
            components: { "Transform": (translation: (0.8, 0.0, 0.0)) },
            wires: { "translation.y": "lfoA", "parent": "group" },
        ),
    ],
)
"#;

    #[test]
    fn a_document_parses_into_entities_components_and_wires() {
        let doc = parse(MINIMAL).expect("parses");

        assert_eq!(doc.version, 1);
        assert_eq!(doc.entities.len(), 2);
        assert_eq!(doc.entities[0].id, "lfoA");
        assert_eq!(doc.entities[0].components.len(), 2);
        assert!(doc.entities[0].components.contains_key("Lfo"));
        assert_eq!(
            doc.entities[1].wires.get("translation.y").map(String::as_str),
            Some("lfoA")
        );
        assert_eq!(
            doc.entities[1].wires.get("parent").map(String::as_str),
            Some("group")
        );
    }

    #[test]
    fn a_payload_is_kept_unparsed() {
        // The parser must not know what a Lfo is -- that is the registry's
        // job, one layer up.
        let doc = parse(MINIMAL).expect("parses");
        let payload = doc.entities[0].components.get("Lfo").expect("present");
        let text = ron::to_string(payload).expect("a payload re-serializes");
        assert!(text.contains("8"), "the payload survived as data: {text}");
    }

    #[test]
    fn missing_maps_default_to_empty() {
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "bare")])"#)
            .expect("parses");
        assert!(doc.entities[0].components.is_empty());
        assert!(doc.entities[0].wires.is_empty());
    }

    #[test]
    fn a_syntax_error_is_reported_not_panicked() {
        let error = parse("Project(version: 1, entities: [").expect_err("must fail");
        assert!(matches!(error, ParseError::Ron(_)), "got {error:?}");
    }

    #[test]
    fn an_unknown_version_is_rejected() {
        let error = parse(r#"Project(version: 99, entities: [])"#).expect_err("must fail");
        assert_eq!(error, ParseError::UnsupportedVersion(99));
    }

    #[test]
    fn a_duplicate_id_rejects_the_document() {
        // Spec §4.3: nothing in the document can be resolved unambiguously,
        // so this is a whole-reload failure rather than a per-item one.
        let error = parse(
            r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "a")])"#,
        )
        .expect_err("must fail");
        assert_eq!(error, ParseError::DuplicateId("a".to_string()));
    }

    #[test]
    fn an_empty_document_is_valid() {
        let doc = parse("Project(version: 1, entities: [])").expect("parses");
        assert!(doc.entities.is_empty());
    }
}
```

Create `crates/sway-graph/src/project/mod.rs`:

```rust
//! The project document: reading it, applying it, writing it.
//! Spec: docs/superpowers/specs/2026-08-06-project-format-design.md

pub mod doc;

pub use doc::{EntityDoc, FORMAT_VERSION, ParseError, ProjectDoc, parse};
```

Add to `crates/sway-graph/src/lib.rs`, with the other `pub mod` lines:

```rust
pub mod project;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p sway-graph --lib project::`

Expected: all seven PASS. If serde rejects `#[serde(rename = "Project")]`
against the RON struct-name syntax, drop both `rename` attributes and name the
structs `Project` and `Entity` internally instead — the file syntax is what
matters, not the Rust name.

- [ ] **Step 3: Commit**

```bash
git add crates/sway-graph/src/project crates/sway-graph/src/lib.rs
git commit -m "feat(graph): the project document and its parser"
```

---

### Task 3: The authorable-component registry

**Files:**
- Create: `crates/sway-graph/src/project/registry.rs`
- Modify: `crates/sway-graph/src/project/mod.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `struct ComponentEntry { pub name: &'static str, pub type_id: TypeId }`
  - `#[derive(Resource, Default)] struct ComponentDocRegistry { pub entries: Vec<ComponentEntry> }`
  - `impl ComponentDocRegistry { fn by_name(&self, &str) -> Option<&ComponentEntry> }`
  - `fn register_authorable<C: Component + Reflect + TypePath + GetTypeRegistration>(app: &mut App, name: &'static str)`

- [ ] **Step 1: Write the registry and its failing tests**

Create `crates/sway-graph/src/project/registry.rs`:

```rust
//! Which components a document may name, and what they are called there.
//! Spec §3.
//!
//! Short names, not reflect `TypePath`: nobody should type
//! `sway_nodes::osc::Lfo` into a hand-authored file, and a type path pins an
//! internal module layout into the file format.

use std::any::TypeId;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::resource::Resource;
use bevy_reflect::{GetTypeRegistration, Reflect, TypePath};
use bevy_reflect::std_traits::ReflectDefault;

pub struct ComponentEntry {
    /// The key used in a document, e.g. `"Lfo"`.
    pub name: &'static str,
    pub type_id: TypeId,
    /// For diagnostics and the inspector's fallback rendering.
    pub type_path: &'static str,
}

#[derive(Resource, Default)]
pub struct ComponentDocRegistry {
    pub entries: Vec<ComponentEntry>,
}

impl ComponentDocRegistry {
    pub fn by_name(&self, name: &str) -> Option<&ComponentEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub fn by_type(&self, type_id: TypeId) -> Option<&ComponentEntry> {
        self.entries.iter().find(|entry| entry.type_id == type_id)
    }
}

/// Makes `C` nameable in a document.
///
/// Panics on a duplicate name, on a type without `#[reflect(Component)]`, and
/// on one without `#[reflect(Default)]` — all three at startup, which is the
/// only place this plan allows a panic. A show that would fail to load its
/// project fails while the operator is still looking at a terminal.
pub fn register_authorable<C>(app: &mut App, name: &'static str)
where
    C: Component + Reflect + TypePath + GetTypeRegistration,
{
    app.register_type::<C>();
    app.init_resource::<ComponentDocRegistry>();

    let type_id = TypeId::of::<C>();
    {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        let registration = registry
            .get(type_id)
            .unwrap_or_else(|| panic!("{} was just registered", C::type_path()));
        assert!(
            registration.data::<ReflectComponent>().is_some(),
            "authorable component {} needs #[reflect(Component)]",
            C::type_path()
        );
        assert!(
            registration.data::<ReflectDefault>().is_some(),
            "authorable component {} needs #[reflect(Default)]: a document may \
             name a subset of its fields, and the rest come from Default",
            C::type_path()
        );
    }

    let mut docs = app.world_mut().resource_mut::<ComponentDocRegistry>();
    assert!(
        docs.by_name(name).is_none(),
        "two components are registered as \"{name}\"; document keys must be unique"
    );
    docs.entries.push(ComponentEntry {
        name,
        type_id,
        type_path: C::type_path(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Gain {
        factor: f32,
    }

    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Bias {
        offset: f32,
    }

    /// Missing #[reflect(Default)] on purpose.
    #[derive(Component, Reflect, Debug, Default, Clone, Copy, PartialEq)]
    #[reflect(Component)]
    struct NoDefault {
        value: f32,
    }

    #[test]
    fn registering_records_the_short_name_and_the_type() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Gain");

        let registry = app.world().resource::<ComponentDocRegistry>();
        let entry = registry.by_name("Gain").expect("registered");
        assert_eq!(entry.type_id, TypeId::of::<Gain>());
        assert!(registry.by_name("Bias").is_none());
    }

    #[test]
    fn two_components_register_independently() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<Bias>(&mut app, "Bias");

        assert_eq!(app.world().resource::<ComponentDocRegistry>().entries.len(), 2);
    }

    #[test]
    #[should_panic(expected = "document keys must be unique")]
    fn a_duplicate_name_panics_at_startup() {
        let mut app = App::new();
        register_authorable::<Gain>(&mut app, "Same");
        register_authorable::<Bias>(&mut app, "Same");
    }

    #[test]
    #[should_panic(expected = "needs #[reflect(Default)]")]
    fn a_component_without_reflect_default_panics_at_startup() {
        // Spec §3: partial payloads need a fallback for the fields the
        // document does not name, and this is where that is enforced.
        let mut app = App::new();
        register_authorable::<NoDefault>(&mut app, "NoDefault");
    }
}
```

Add to `crates/sway-graph/src/project/mod.rs`:

```rust
pub mod registry;

pub use registry::{ComponentDocRegistry, ComponentEntry, register_authorable};
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p sway-graph --lib project::registry::`

Expected: all four PASS. If `ReflectDefault` does not resolve, the import is
`bevy_reflect::std_traits::ReflectDefault` — this is the M2a finding recorded in
the parent spec §7, and it is easy to get wrong.

- [ ] **Step 3: Commit**

```bash
git add crates/sway-graph/src/project/registry.rs crates/sway-graph/src/project/mod.rs
git commit -m "feat(graph): register which components a document may name"
```

---

### Task 4: Wires gain insert, remove and read

The wire registry already knows how to *collect* wire instances. The document
needs to *write* them.

**Files:**
- Modify: `crates/sway-graph/src/registry_wires.rs`

**Interfaces:**
- Consumes: `Wire`, `WireEntry`, `register_wire` (existing).
- Produces, on `WireEntry`:
  - `pub insert: fn(&mut World, dst: Entity, src: Entity)`
  - `pub remove: fn(&mut World, dst: Entity)`
  - `pub read: fn(&World, dst: Entity) -> Option<Entity>`

- [ ] **Step 1: Write the failing tests**

Add to `crates/sway-graph/src/registry_wires.rs`'s `mod tests`:

```rust
    #[test]
    fn a_registered_wire_can_be_inserted_read_and_removed() {
        // The project format's whole write path for wires. Going through the
        // registry rather than the concrete type is what lets the applier be
        // generic over every wire type at once.
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let entry_index = 0;
        let (insert, read, remove) = {
            let entry = &app.world().resource::<WireRegistry>().entries[entry_index];
            (entry.insert, entry.read, entry.remove)
        };

        assert_eq!(read(app.world(), dst), None, "nothing wired yet");

        insert(app.world_mut(), dst, src);
        assert_eq!(read(app.world(), dst), Some(src));

        remove(app.world_mut(), dst);
        assert_eq!(read(app.world(), dst), None);
    }

    #[test]
    fn inserting_over_an_existing_wire_replaces_its_source() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let first = spawn_float(app.world_mut(), 1.0);
        let second = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let (insert, read) = {
            let entry = &app.world().resource::<WireRegistry>().entries[0];
            (entry.insert, entry.read)
        };

        insert(app.world_mut(), dst, first);
        insert(app.world_mut(), dst, second);

        assert_eq!(read(app.world(), dst), Some(second));
    }

    #[test]
    fn removing_a_wire_that_is_not_there_is_a_no_op() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let remove = app.world().resource::<WireRegistry>().entries[0].remove;
        remove(app.world_mut(), dst);

        assert!(app.world().get::<GainFrom>(dst).is_none());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph --lib registry_wires::`

Expected: compile error — `WireEntry` has no field `insert`.

- [ ] **Step 3: Add the three functions**

In `crates/sway-graph/src/registry_wires.rs`, extend `WireEntry`:

```rust
pub struct WireEntry {
    pub name: &'static str,
    /// Every instance of this wire type, as links.
    pub collect: fn(&mut World, &mut Vec<Link>),
    /// Whether an entity could be this wire's producer — the editor's
    /// legality rule.
    pub has_source: fn(&World, Entity) -> bool,
    /// Whether an entity could be this wire's consumer.
    pub has_target: fn(&World, Entity) -> bool,
    /// Wire `dst`'s inlet to `src`, replacing whatever was there. The project
    /// format's write path (project spec §3).
    pub insert: fn(&mut World, Entity, Entity),
    /// Disconnect `dst`'s inlet. A no-op if it is not connected.
    pub remove: fn(&mut World, Entity),
    /// The producer `dst`'s inlet currently names.
    pub read: fn(&World, Entity) -> Option<Entity>,
}
```

Add the three monomorphised functions next to `collect_wire_of`:

```rust
fn insert_wire_of<W: Wire>(world: &mut World, dst: Entity, src: Entity) {
    if let Ok(mut entity) = world.get_entity_mut(dst) {
        entity.insert(W::from(src));
    }
}

fn remove_wire_of<W: Wire>(world: &mut World, dst: Entity) {
    if let Ok(mut entity) = world.get_entity_mut(dst) {
        entity.remove::<W>();
    }
}

fn read_wire_of<W: Wire>(world: &World, dst: Entity) -> Option<Entity> {
    world.get::<W>(dst).map(Relationship::get)
}
```

Add `use bevy_ecs::relationship::Relationship;` to the imports, and the three
fields to the `WireEntry` built in `register_wire`:

```rust
            insert: insert_wire_of::<W>,
            remove: remove_wire_of::<W>,
            read: read_wire_of::<W>,
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph --lib registry_wires::`

Expected: the three new tests plus the four existing ones PASS.

If `W::from(src)` does not compile because `Relationship` has no `from`, add a
required method to the `Wire` trait in `src/wire.rs` instead:

```rust
    /// Build this wire from its producer. Bevy's `Relationship` derive gives
    /// every wire a one-field constructor; this names it for generic code.
    fn make(src: Entity) -> Self;
```

and implement it as `Self(src)` in each of the three wire impls
(`GainFrom` in `test_wires.rs`, `AmplitudeFrom`, `TranslationYFrom`) and as
`ChildOf(src)` in `wire.rs`'s `impl Wire for ChildOf`. Then use `W::make(src)`.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/registry_wires.rs crates/sway-graph/src/wire.rs
git commit -m "feat(graph): wires can be written, not only collected"
```

---

### Task 5: Identity, diagnostics, and the spawn/despawn passes

**Files:**
- Create: `crates/sway-graph/src/project/diagnostics.rs`, `crates/sway-graph/src/project/apply.rs`
- Modify: `crates/sway-graph/src/project/mod.rs`

**Interfaces:**
- Consumes: `ProjectDoc`, `EntityDoc` (Task 2).
- Produces:
  - `#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)] struct DocId(pub String)`
  - `#[derive(Resource, Default, Debug, Clone, PartialEq)] struct ProjectDiagnostics { pub parse: Option<String>, pub items: Vec<ItemError> }`
  - `enum ItemError { UnknownComponent { entity: String, name: String }, BadPayload { entity: String, name: String, message: String }, UnknownWire { entity: String, wire: String }, UnresolvedTarget { entity: String, wire: String, target: String } }`
  - `fn apply(world: &mut World, doc: &ProjectDoc) -> ProjectDiagnostics`
  - `fn reconcile_entities(world: &mut World, doc: &ProjectDoc) -> HashMap<String, Entity>` (private; tested through `apply`)

- [ ] **Step 1: Write the diagnostics types**

Create `crates/sway-graph/src/project/diagnostics.rs`:

```rust
//! What a reload could not do, and to which item. Spec §4.3.
//!
//! Mirrors `GraphDiagnostics`: a resource the editor renders, never an error
//! that stops the app.

use bevy_ecs::component::Component;
use bevy_ecs::resource::Resource;

/// An entity's identity in the document, and its identity across reloads.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocId(pub String);

#[derive(Debug, Clone, PartialEq)]
pub enum ItemError {
    UnknownComponent { entity: String, name: String },
    BadPayload { entity: String, name: String, message: String },
    UnknownWire { entity: String, wire: String },
    UnresolvedTarget { entity: String, wire: String, target: String },
}

impl core::fmt::Display for ItemError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownComponent { entity, name } => {
                write!(f, "{entity}: no component is registered as \"{name}\"")
            }
            Self::BadPayload { entity, name, message } => {
                write!(f, "{entity}.{name}: {message}")
            }
            Self::UnknownWire { entity, wire } => {
                write!(f, "{entity}: no wire is registered as \"{wire}\"")
            }
            Self::UnresolvedTarget { entity, wire, target } => {
                write!(f, "{entity}.{wire}: no entity has the id \"{target}\"")
            }
        }
    }
}

/// The result of the most recent load attempt.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct ProjectDiagnostics {
    /// Set when a reload was rejected whole. The running world is untouched.
    pub parse: Option<String>,
    /// Per-item failures; everything else applied.
    pub items: Vec<ItemError>,
}

impl ProjectDiagnostics {
    pub fn is_clean(&self) -> bool {
        self.parse.is_none() && self.items.is_empty()
    }
}
```

- [ ] **Step 2: Write the failing tests for the entity passes**

Create `crates/sway-graph/src/project/apply.rs`:

```rust
//! Applying a document to the world, by reconciling on `DocId`. Spec §4.
//!
//! Four passes, in order: index and despawn, spawn, components, wires. The
//! first two complete before any wire is resolved, so a wire may name an
//! entity declared later in the file.

use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::name::Name;
use bevy_ecs::world::World;

use crate::project::diagnostics::{DocId, ProjectDiagnostics};
use crate::project::doc::ProjectDoc;

/// Applies `doc` to `world` and returns what it could not do.
///
/// Never panics and never returns `Err`: a document is authored text, and a
/// half-typed one is the normal state of a file being edited.
pub fn apply(world: &mut World, doc: &ProjectDoc) -> ProjectDiagnostics {
    let diagnostics = ProjectDiagnostics::default();
    let _ids = reconcile_entities(world, doc);
    diagnostics
}

/// Passes 1 and 2: despawn what left, spawn what arrived, keep what stayed.
/// Returns the document-id -> entity map the later passes resolve against.
fn reconcile_entities(world: &mut World, doc: &ProjectDoc) -> HashMap<String, Entity> {
    let mut existing: HashMap<String, Entity> = world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();

    let wanted: Vec<&str> = doc.entities.iter().map(|e| e.id.as_str()).collect();

    // Pass 1. Despawn takes children and any wire on the despawned entity
    // with it; a wire *pointing at* it is left dangling until the next
    // rebuild, which is exactly what `propagate_of` already tolerates.
    let departed: Vec<String> = existing
        .keys()
        .filter(|id| !wanted.contains(&id.as_str()))
        .cloned()
        .collect();
    for id in departed {
        if let Some(entity) = existing.remove(&id) {
            world.despawn(entity);
        }
    }

    // Pass 2.
    for entity_doc in &doc.entities {
        if existing.contains_key(&entity_doc.id) {
            continue;
        }
        let entity = world
            .spawn((
                DocId(entity_doc.id.clone()),
                Name::new(entity_doc.id.clone()),
            ))
            .id();
        existing.insert(entity_doc.id.clone(), entity);
    }

    existing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::doc::parse;

    fn doc(text: &str) -> ProjectDoc {
        parse(text).expect("test document parses")
    }

    fn ids(world: &mut World) -> Vec<String> {
        let mut found: Vec<String> = world
            .query::<&DocId>()
            .iter(world)
            .map(|id| id.0.clone())
            .collect();
        found.sort();
        found
    }

    fn entity_of(world: &mut World, id: &str) -> Option<Entity> {
        world
            .query::<(Entity, &DocId)>()
            .iter(world)
            .find(|(_, doc_id)| doc_id.0 == id)
            .map(|(entity, _)| entity)
    }

    #[test]
    fn a_first_load_spawns_every_entity_with_its_id_and_name() {
        let mut world = World::new();
        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );

        assert_eq!(ids(&mut world), vec!["a".to_string(), "b".to_string()]);
        let a = entity_of(&mut world, "a").expect("spawned");
        assert_eq!(world.get::<Name>(a).map(|n| n.as_str().to_string()), Some("a".to_string()));
    }

    #[test]
    fn a_surviving_entity_keeps_its_entity_id() {
        // The whole point of reconciling rather than respawning: the editor's
        // identity, the entity's children, and anything a runtime system
        // attached all ride on this.
        let mut world = World::new();
        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        let before = entity_of(&mut world, "a").expect("spawned");

        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );

        assert_eq!(entity_of(&mut world, "a"), Some(before), "same Entity across reloads");
        assert!(entity_of(&mut world, "b").is_some(), "the new one arrived");
    }

    #[test]
    fn an_entity_dropped_from_the_document_is_despawned() {
        let mut world = World::new();
        apply(
            &mut world,
            &doc(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#),
        );
        let b = entity_of(&mut world, "b").expect("spawned");

        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));

        assert!(world.get_entity(b).is_err(), "b is gone");
        assert_eq!(ids(&mut world), vec!["a".to_string()]);
    }

    #[test]
    fn entities_without_a_doc_id_are_never_touched() {
        // The camera, the light, anything a runtime system spawned.
        let mut world = World::new();
        let runtime_owned = world.spawn(Name::new("camera")).id();

        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(world.get_entity(runtime_owned).is_ok(), "not ours, not despawned");
    }

    #[test]
    fn an_empty_document_clears_the_authored_world() {
        let mut world = World::new();
        apply(&mut world, &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        apply(&mut world, &doc("Project(version: 1, entities: [])"));

        assert!(ids(&mut world).is_empty());
    }
}
```

Add to `crates/sway-graph/src/project/mod.rs`:

```rust
pub mod apply;
pub mod diagnostics;

pub use apply::apply;
pub use diagnostics::{DocId, ItemError, ProjectDiagnostics};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p sway-graph --lib project::apply::`

Expected: all five PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-graph/src/project
git commit -m "feat(graph): reconcile a document's entities by id"
```

---

### Task 6: The component pass

**Files:**
- Modify: `crates/sway-graph/src/project/apply.rs`

**Interfaces:**
- Consumes: `ComponentDocRegistry` (Task 3), `reconcile_entities` (Task 5).
- Produces: `apply` now writes components and fills `ProjectDiagnostics::items`.

- [ ] **Step 1: Write the failing tests**

Add to `apply.rs`'s `mod tests`, and add these imports at the top of the test
module:

```rust
    use crate::project::registry::register_authorable;
    use crate::project::diagnostics::ItemError;
    use bevy_app::App;
    use bevy_ecs::component::Component;
    use bevy_ecs::query::Changed;
    use bevy_reflect::Reflect;

    #[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Osc {
        hz: f32,
        amplitude: f32,
    }

    impl Default for Osc {
        fn default() -> Self {
            Self { hz: 1.0, amplitude: 0.5 }
        }
    }

    /// An app with `Osc` authorable, which is all the component pass needs.
    fn doc_app() -> App {
        let mut app = App::new();
        register_authorable::<Osc>(&mut app, "Osc");
        app
    }

    #[test]
    fn a_named_component_is_inserted_from_its_payload() {
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
            ])"#),
        );

        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 3.0, amplitude: 0.25 })
        );
    }

    #[test]
    fn a_partial_payload_leaves_the_other_fields_alone_on_reload() {
        // Spec §4.1: `apply` on an existing component touches only the named
        // fields, so a reload does not clobber what a wire is driving.
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        // Something else — a wire — moves amplitude.
        app.world_mut().get_mut::<Osc>(entity).expect("present").amplitude = 0.9;

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 4.0) })
            ])"#),
        );

        assert_eq!(
            app.world().get::<Osc>(entity),
            Some(&Osc { hz: 4.0, amplitude: 0.9 })
        );
    }

    #[test]
    fn an_unchanged_component_is_not_marked_changed() {
        // The same discipline wires live under (parent spec §2.11): writing an
        // equal value destroys change detection for everything downstream.
        let mut app = doc_app();
        let text = r#"Project(version: 1, entities: [
            Entity(id: "a", components: { "Osc": (hz: 3.0, amplitude: 0.25) })
        ])"#;
        apply(app.world_mut(), &doc(text));
        app.world_mut().clear_trackers();

        apply(app.world_mut(), &doc(text));

        let changed = app
            .world_mut()
            .query_filtered::<(), Changed<Osc>>()
            .iter(app.world())
            .count();
        assert_eq!(changed, 0, "an identical reload must touch nothing");
    }

    #[test]
    fn a_component_dropped_from_the_document_is_removed() {
        let mut app = doc_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 3.0) })
            ])"#),
        );
        let entity = entity_of(app.world_mut(), "a").expect("spawned");

        apply(app.world_mut(), &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));

        assert!(app.world().get::<Osc>(entity).is_none());
    }

    #[test]
    fn an_unregistered_component_on_the_entity_survives_a_reload() {
        // A `Mesh3d` a runtime system attached. The applier only removes
        // components it is registered to author.
        #[derive(Component)]
        struct RuntimeOwned;

        let mut app = doc_app();
        apply(app.world_mut(), &doc(r#"Project(version: 1, entities: [Entity(id: "a")])"#));
        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        app.world_mut().entity_mut(entity).insert(RuntimeOwned);

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: 1.0) })
            ])"#),
        );

        assert!(app.world().get::<RuntimeOwned>(entity).is_some());
    }

    #[test]
    fn an_unknown_component_name_is_reported_and_the_rest_applies() {
        let mut app = doc_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Nope": (), "Osc": (hz: 2.0) })
            ])"#),
        );

        let entity = entity_of(app.world_mut(), "a").expect("spawned");
        assert_eq!(app.world().get::<Osc>(entity).map(|o| o.hz), Some(2.0));
        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnknownComponent {
                entity: "a".to_string(),
                name: "Nope".to_string(),
            }]
        );
    }

    #[test]
    fn a_payload_that_will_not_deserialize_is_reported_not_panicked() {
        let mut app = doc_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "a", components: { "Osc": (hz: "not a number") })
            ])"#),
        );

        assert!(
            matches!(diagnostics.items.as_slice(), [ItemError::BadPayload { name, .. }] if name == "Osc"),
            "got {:?}",
            diagnostics.items
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph --lib project::apply::`

Expected: the seven new tests FAIL (no component is ever written).

- [ ] **Step 3: Implement the component pass**

In `apply.rs`, replace `apply` and add the helper:

```rust
use std::any::TypeId;

use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_reflect::TypeRegistry;
use bevy_reflect::serde::TypedReflectDeserializer;
use serde::de::DeserializeSeed;

use crate::project::diagnostics::ItemError;
use crate::project::doc::EntityDoc;
use crate::project::registry::ComponentDocRegistry;

pub fn apply(world: &mut World, doc: &ProjectDoc) -> ProjectDiagnostics {
    let mut diagnostics = ProjectDiagnostics::default();
    let ids = reconcile_entities(world, doc);

    // Taken out so the passes can hold `&mut World`, put back after. The
    // registries are read-only here; this is a borrow move, not a mutation.
    let components = world
        .remove_resource::<ComponentDocRegistry>()
        .unwrap_or_default();
    let type_registry = world.resource::<AppTypeRegistry>().clone();

    {
        let type_registry = type_registry.read();
        for entity_doc in &doc.entities {
            let Some(&entity) = ids.get(&entity_doc.id) else {
                continue;
            };
            apply_components(
                world,
                entity,
                entity_doc,
                &components,
                &type_registry,
                &mut diagnostics,
            );
        }
    }

    world.insert_resource(components);
    diagnostics
}

fn apply_components(
    world: &mut World,
    entity: Entity,
    entity_doc: &EntityDoc,
    components: &ComponentDocRegistry,
    type_registry: &TypeRegistry,
    diagnostics: &mut ProjectDiagnostics,
) {
    let mut written: Vec<TypeId> = Vec::new();

    for (name, payload) in &entity_doc.components {
        let Some(entry) = components.by_name(name) else {
            diagnostics.items.push(ItemError::UnknownComponent {
                entity: entity_doc.id.clone(),
                name: name.clone(),
            });
            continue;
        };
        let Some(registration) = type_registry.get(entry.type_id) else {
            diagnostics.items.push(ItemError::BadPayload {
                entity: entity_doc.id.clone(),
                name: name.clone(),
                message: "type is not in the reflect registry".to_string(),
            });
            continue;
        };
        let value = match TypedReflectDeserializer::new(registration, type_registry)
            .deserialize(payload.clone())
        {
            Ok(value) => value,
            Err(error) => {
                diagnostics.items.push(ItemError::BadPayload {
                    entity: entity_doc.id.clone(),
                    name: name.clone(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            diagnostics.items.push(ItemError::BadPayload {
                entity: entity_doc.id.clone(),
                name: name.clone(),
                message: "type is not a reflectable component".to_string(),
            });
            continue;
        };

        written.push(entry.type_id);

        let current_matches = world
            .get_entity(entity)
            .ok()
            .and_then(|entity_ref| reflect_component.reflect(entity_ref))
            .and_then(|current| value.reflect_partial_eq(current.as_partial_reflect()))
            .unwrap_or(false);
        if current_matches {
            continue; // writing an equal value would mark Changed for nothing
        }

        let has_component = world
            .get_entity(entity)
            .ok()
            .and_then(|entity_ref| reflect_component.reflect(entity_ref))
            .is_some();
        let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
            continue;
        };
        if has_component {
            // Only the fields the document names; the rest keep their values.
            reflect_component.apply(&mut entity_mut, &*value);
        } else {
            // The unnamed fields come from ReflectDefault.
            reflect_component.insert(&mut entity_mut, &*value, type_registry);
        }
    }

    // Anything registered-authorable, present, and absent from the document.
    for entry in &components.entries {
        if written.contains(&entry.type_id) {
            continue;
        }
        let Some(registration) = type_registry.get(entry.type_id) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let present = world
            .get_entity(entity)
            .ok()
            .and_then(|entity_ref| reflect_component.reflect(entity_ref))
            .is_some();
        if !present {
            continue;
        }
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            reflect_component.remove(&mut entity_mut);
        }
    }
}
```

`apply`'s tests build a bare `App`, which already has `AppTypeRegistry`; the
`World::new()`-based tests from Task 5 do not, so add
`world.init_resource::<AppTypeRegistry>();` to those three tests' setup, or
switch them to `doc_app()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph --lib project::`

Expected: every test in `project::` PASSES — Task 5's five, Task 6's seven,
Task 2's seven, Task 3's four.

If `a_partial_payload_leaves_the_other_fields_alone_on_reload` fails because
Task 1 found `apply` does not work that way, replace the `apply`/`insert` branch
with `insert` unconditionally and change this test's expectation to
`amplitude: 0.5` (the default), noting the loss in the commit message.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/project/apply.rs
git commit -m "feat(graph): write a document's components, without touching what it does not name"
```

---

### Task 7: The wire pass

**Files:**
- Modify: `crates/sway-graph/src/project/apply.rs`

**Interfaces:**
- Consumes: `WireRegistry`'s `insert`/`remove`/`read` (Task 4), `TopologyDirty`.
- Produces: `apply` now wires entities and marks the topology dirty.

- [ ] **Step 1: Write the failing tests**

Add to `apply.rs`'s `mod tests`:

```rust
    use crate::order::TopologyDirty;
    use crate::registry_wires::register_wire;
    use crate::test_wires::{FloatOut, Gain, GainFrom};

    /// `Gain` is the wire fixture's target and `FloatOut` its source; both
    /// become authorable so a document can build the whole graph.
    fn wired_app() -> App {
        let mut app = doc_app();
        app.init_resource::<TopologyDirty>();
        register_wire::<GainFrom>(&mut app);
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<FloatOut>(&mut app, "FloatOut");
        app
    }

    const WIRED: &str = r#"Project(version: 1, entities: [
        Entity(id: "src", components: { "FloatOut": (2.0) }),
        Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
               wires: { "factor": "src" }),
    ])"#;

    #[test]
    fn a_document_wire_becomes_a_relationship_component() {
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));

        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn a_wire_may_name_an_entity_declared_later_in_the_file() {
        let mut app = wired_app();
        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "src" }),
                Entity(id: "src", components: { "FloatOut": (2.0) }),
            ])"#),
        );

        let src = entity_of(app.world_mut(), "src").expect("spawned");
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");
        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn a_wire_dropped_from_the_document_is_removed() {
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        let dst = entity_of(app.world_mut(), "dst").expect("spawned");

        apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) }),
            ])"#),
        );

        assert!(app.world().get::<GainFrom>(dst).is_none());
    }

    #[test]
    fn an_unchanged_wire_is_not_churned() {
        // Removing and re-inserting would rewrite the producer's
        // RelationshipTarget collection for nothing.
        let mut app = wired_app();
        apply(app.world_mut(), &doc(WIRED));
        app.world_mut().clear_trackers();

        apply(app.world_mut(), &doc(WIRED));

        let changed = app
            .world_mut()
            .query_filtered::<(), Changed<GainFrom>>()
            .iter(app.world())
            .count();
        assert_eq!(changed, 0);
    }

    #[test]
    fn a_wire_naming_a_missing_entity_is_reported() {
        let mut app = wired_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "factor": "ghost" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnresolvedTarget {
                entity: "dst".to_string(),
                wire: "factor".to_string(),
                target: "ghost".to_string(),
            }]
        );
        let dst = entity_of(app.world_mut(), "dst").expect("spawned anyway");
        assert!(app.world().get::<GainFrom>(dst).is_none());
    }

    #[test]
    fn an_unknown_wire_name_is_reported() {
        let mut app = wired_app();
        let diagnostics = apply(
            app.world_mut(),
            &doc(r#"Project(version: 1, entities: [
                Entity(id: "src", components: { "FloatOut": (2.0) }),
                Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.0) },
                       wires: { "nope": "src" }),
            ])"#),
        );

        assert_eq!(
            diagnostics.items,
            vec![ItemError::UnknownWire {
                entity: "dst".to_string(),
                wire: "nope".to_string(),
            }]
        );
    }

    #[test]
    fn applying_marks_the_topology_dirty() {
        // Spec §4.1: the applier never touches GraphOrder; it sets the flag
        // and the existing rebuild does the rest on the next FixedUpdate.
        let mut app = wired_app();
        app.world_mut().resource_mut::<TopologyDirty>().0 = false;

        apply(app.world_mut(), &doc(WIRED));

        assert!(app.world().resource::<TopologyDirty>().0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph --lib project::apply::`

Expected: the seven new tests FAIL.

- [ ] **Step 3: Implement the wire pass**

In `apply.rs`, add to the imports:

```rust
use crate::order::TopologyDirty;
use crate::registry_wires::WireRegistry;
```

Inside `apply`, after the component loop's block closes and before
`world.insert_resource(components);`, add:

```rust
    let wires = world.remove_resource::<WireRegistry>().unwrap_or_default();
    for entity_doc in &doc.entities {
        let Some(&entity) = ids.get(&entity_doc.id) else {
            continue;
        };
        apply_wires(world, entity, entity_doc, &ids, &wires, &mut diagnostics);
    }
    world.insert_resource(wires);

    if let Some(mut dirty) = world.get_resource_mut::<TopologyDirty>() {
        dirty.0 = true;
    }
```

And add the function:

```rust
fn apply_wires(
    world: &mut World,
    entity: Entity,
    entity_doc: &EntityDoc,
    ids: &HashMap<String, Entity>,
    wires: &WireRegistry,
    diagnostics: &mut ProjectDiagnostics,
) {
    for (name, target_id) in &entity_doc.wires {
        if wires.entries.iter().all(|entry| entry.name != name) {
            diagnostics.items.push(ItemError::UnknownWire {
                entity: entity_doc.id.clone(),
                wire: name.clone(),
            });
        } else if !ids.contains_key(target_id) {
            diagnostics.items.push(ItemError::UnresolvedTarget {
                entity: entity_doc.id.clone(),
                wire: name.clone(),
                target: target_id.clone(),
            });
        }
    }

    for entry in &wires.entries {
        let wanted = entity_doc
            .wires
            .get(entry.name)
            .and_then(|target_id| ids.get(target_id))
            .copied();
        let current = (entry.read)(world, entity);
        if wanted == current {
            continue; // never churn a RelationshipTarget for nothing
        }
        match wanted {
            Some(src) => (entry.insert)(world, entity, src),
            None => (entry.remove)(world, entity),
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph --lib project::`

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/project/apply.rs
git commit -m "feat(graph): wire a document's entities, and hand the order its flag"
```

---

### Task 8: The asset, the loader, and the plugin

**Files:**
- Create: `crates/sway-graph/src/project/asset.rs`
- Modify: `crates/sway-graph/src/project/mod.rs`, `crates/sway-graph/Cargo.toml`, root `Cargo.toml`

**Interfaces:**
- Consumes: `parse`, `apply`, `ProjectDiagnostics`.
- Produces:
  - `#[derive(Asset, TypePath)] struct ProjectAsset { pub doc: ProjectDoc }`
  - `struct ProjectLoader`
  - `#[derive(Resource, Default)] struct ProjectHandle(pub Option<Handle<ProjectAsset>>)`
  - `struct ProjectPlugin` — registers the asset, the loader, both systems, and `ProjectDiagnostics`

- [ ] **Step 1: Add `bevy_asset`**

Root `Cargo.toml`, under `[workspace.dependencies]`:

```toml
bevy_asset = "=0.19.0"
```

`crates/sway-graph/Cargo.toml`, under `[dependencies]`:

```toml
bevy_asset.workspace = true
```

Update the manifest's dependency-rule comment to say `bevy_asset` has joined,
as M4 anticipated.

- [ ] **Step 2: Write the asset layer**

Create `crates/sway-graph/src/project/asset.rs`:

```rust
//! The document as a Bevy asset. Spec §4.
//!
//! `AssetServer` supplies file watching, debounce and the write-then-rename
//! behaviour real text editors use; none of that is hand-rolled here.

use bevy_app::{App, Plugin, PreUpdate};
use bevy_asset::io::Reader;
use bevy_asset::{
    Asset, AssetApp, AssetEvent, AssetId, AssetLoadFailedEvent, AssetLoader, Assets, Handle,
    LoadContext,
};
use bevy_ecs::event::EventReader;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::system::ResMut;
use bevy_ecs::world::World;
use bevy_reflect::TypePath;

use crate::project::apply::apply;
use crate::project::diagnostics::ProjectDiagnostics;
use crate::project::doc::{ParseError, ProjectDoc, parse};

#[derive(Asset, TypePath, Debug, Clone)]
pub struct ProjectAsset {
    pub doc: ProjectDoc,
}

/// The project the app is currently running. Set by whatever loads it.
#[derive(Resource, Default)]
pub struct ProjectHandle(pub Option<Handle<ProjectAsset>>);

/// Set by [`note_project_changes`], drained by [`apply_pending_project`].
#[derive(Resource, Default)]
struct PendingProject(Option<AssetId<ProjectAsset>>);

#[derive(Default)]
pub struct ProjectLoader;

impl AssetLoader for ProjectLoader {
    type Asset = ProjectAsset;
    type Settings = ();
    type Error = ParseError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _context: &mut LoadContext<'_>,
    ) -> Result<ProjectAsset, ParseError> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| ParseError::Ron(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|e| ParseError::Ron(e.to_string()))?;
        Ok(ProjectAsset { doc: parse(&text)? })
    }

    fn extensions(&self) -> &[&str] {
        &["sway.ron"]
    }
}

/// Records that the project asset arrived or changed. Ordinary system, so it
/// can read events.
fn note_project_changes(
    mut events: EventReader<AssetEvent<ProjectAsset>>,
    mut pending: ResMut<PendingProject>,
) {
    for event in events.read() {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => pending.0 = Some(*id),
            _ => {}
        }
    }
}

/// Records a load that failed. Spec §4.3: a syntax error rejects the reload
/// whole and leaves the running world exactly as it was — which is what
/// happens naturally, since a failed load produces no asset. All that is
/// needed is to make it visible.
fn note_load_failures(
    mut events: EventReader<AssetLoadFailedEvent<ProjectAsset>>,
    mut diagnostics: ResMut<ProjectDiagnostics>,
) {
    for event in events.read() {
        diagnostics.parse = Some(event.error.to_string());
    }
}

/// Applies the pending document. Exclusive, because applying spawns,
/// despawns and inserts relationship components.
fn apply_pending_project(world: &mut World) {
    let Some(id) = world.resource_mut::<PendingProject>().0.take() else {
        return;
    };
    let Some(doc) = world
        .resource::<Assets<ProjectAsset>>()
        .get(id)
        .map(|asset| asset.doc.clone())
    else {
        return;
    };

    let mut diagnostics = apply(world, &doc);
    // A successful apply clears the previous parse error: the file is
    // readable again.
    diagnostics.parse = None;
    world.insert_resource(diagnostics);
}

/// Loading, watching and applying the project document.
///
/// Added alongside `WiresPlugin`. Requires `AssetPlugin`, which
/// `DefaultPlugins` supplies; a headless test app adds `AssetPlugin::default()`
/// itself.
pub struct ProjectPlugin;

impl Plugin for ProjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<ProjectAsset>()
            .init_asset_loader::<ProjectLoader>()
            .init_resource::<ProjectHandle>()
            .init_resource::<PendingProject>()
            .init_resource::<ProjectDiagnostics>()
            .add_systems(
                PreUpdate,
                (note_project_changes, note_load_failures, apply_pending_project).chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::diagnostics::DocId;
    use bevy_app::App;
    use bevy_asset::AssetPlugin;
    use bevy_ecs::entity::Entity;

    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins(AssetPlugin::default())
            .add_plugins(ProjectPlugin);
        app
    }

    fn doc_ids(app: &mut App) -> Vec<String> {
        let mut ids: Vec<String> = app
            .world_mut()
            .query::<&DocId>()
            .iter(app.world())
            .map(|id| id.0.clone())
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn adding_the_asset_applies_it_to_the_world() {
        let mut app = asset_app();
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "a")])"#).expect("parses");
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .add(ProjectAsset { doc });
        app.world_mut().resource_mut::<ProjectHandle>().0 = Some(handle);

        app.update();

        assert_eq!(doc_ids(&mut app), vec!["a".to_string()]);
    }

    #[test]
    fn modifying_the_asset_reapplies_it() {
        let mut app = asset_app();
        let doc = parse(r#"Project(version: 1, entities: [Entity(id: "a")])"#).expect("parses");
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .add(ProjectAsset { doc });
        app.world_mut().resource_mut::<ProjectHandle>().0 = Some(handle.clone());
        app.update();
        let before: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &DocId)>()
            .iter(app.world())
            .map(|(entity, _)| entity)
            .collect();

        let next =
            parse(r#"Project(version: 1, entities: [Entity(id: "a"), Entity(id: "b")])"#)
                .expect("parses");
        app.world_mut()
            .resource_mut::<Assets<ProjectAsset>>()
            .insert(&handle, ProjectAsset { doc: next });
        app.update();

        assert_eq!(doc_ids(&mut app), vec!["a".to_string(), "b".to_string()]);
        let after: Vec<Entity> = app
            .world_mut()
            .query::<(Entity, &DocId)>()
            .iter(app.world())
            .map(|(entity, _)| entity)
            .collect();
        assert!(
            before.iter().all(|entity| after.contains(entity)),
            "the surviving entity kept its Entity across the reload"
        );
    }

    #[test]
    fn the_loader_reads_text_into_a_document() {
        // The loader's own parse path, without an AssetServer.
        let text = r#"Project(version: 1, entities: [Entity(id: "a")])"#;
        let doc = parse(text).expect("parses");
        assert_eq!(doc.entities.len(), 1);
    }
}
```

Add to `crates/sway-graph/src/project/mod.rs`:

```rust
pub mod asset;

pub use asset::{ProjectAsset, ProjectHandle, ProjectLoader, ProjectPlugin};
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p sway-graph --lib project::asset::`

Expected: all three PASS.

If `AssetLoadFailedEvent`'s field is not `error`, print the type with
`cargo doc --open -p bevy_asset` or read
`~/.cargo/registry/src/*/bevy_asset-0.19.0/src/event.rs` and use whatever names
the failure; the assertion is only that a failure becomes a string in
`ProjectDiagnostics::parse`.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-graph/src/project/asset.rs crates/sway-graph/src/project/mod.rs \
        crates/sway-graph/Cargo.toml Cargo.toml
git commit -m "feat(graph): load and hot-reload the project document"
```

---

### Task 9: The emitter, and the round-trip

**Files:**
- Create: `crates/sway-graph/src/project/emit.rs`
- Modify: `crates/sway-graph/src/project/mod.rs`

**Interfaces:**
- Consumes: `ComponentDocRegistry`, `WireRegistry`, `DocId`.
- Produces:
  - `fn to_document(world: &mut World) -> ProjectDoc`
  - `fn to_ron(doc: &ProjectDoc) -> Result<String, ron::Error>`

- [ ] **Step 1: Write the emitter and its failing tests**

Create `crates/sway-graph/src/project/emit.rs`:

```rust
//! World -> document. Spec §5.
//!
//! Exists to prove the format complete: a round-trip through here and back is
//! the only check that every authorable component and wire can be written
//! down. The *in-place, comment-preserving* writer is M7's; this one emits a
//! whole document.

use std::collections::BTreeMap;

use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::world::World;
use bevy_reflect::serde::TypedReflectSerializer;

use crate::project::diagnostics::DocId;
use crate::project::doc::{EntityDoc, FORMAT_VERSION, ProjectDoc};
use crate::project::registry::ComponentDocRegistry;
use crate::registry_wires::WireRegistry;

pub fn to_document(world: &mut World) -> ProjectDoc {
    let mut carriers: Vec<(String, Entity)> = world
        .query::<(Entity, &DocId)>()
        .iter(world)
        .map(|(entity, id)| (id.0.clone(), entity))
        .collect();
    carriers.sort_by(|a, b| a.0.cmp(&b.0));

    let ids: BTreeMap<Entity, String> = carriers
        .iter()
        .map(|(id, entity)| (*entity, id.clone()))
        .collect();

    let components = world
        .remove_resource::<ComponentDocRegistry>()
        .unwrap_or_default();
    let wires = world.remove_resource::<WireRegistry>().unwrap_or_default();
    let type_registry = world.resource::<AppTypeRegistry>().clone();

    let mut entities = Vec::with_capacity(carriers.len());
    {
        let type_registry = type_registry.read();
        for (id, entity) in &carriers {
            let mut component_map = BTreeMap::new();
            for entry in &components.entries {
                let Some(registration) = type_registry.get(entry.type_id) else {
                    continue;
                };
                let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                    continue;
                };
                let Ok(entity_ref) = world.get_entity(*entity) else {
                    continue;
                };
                let Some(value) = reflect_component.reflect(entity_ref) else {
                    continue;
                };
                let serializer =
                    TypedReflectSerializer::new(value.as_partial_reflect(), &type_registry);
                let Ok(text) = ron::to_string(&serializer) else {
                    continue;
                };
                let Ok(payload) = ron::from_str::<ron::Value>(&text) else {
                    continue;
                };
                component_map.insert(entry.name.to_string(), payload);
            }

            let mut wire_map = BTreeMap::new();
            for entry in &wires.entries {
                let Some(src) = (entry.read)(world, *entity) else {
                    continue;
                };
                let Some(src_id) = ids.get(&src) else {
                    continue; // wired to something the document does not own
                };
                wire_map.insert(entry.name.to_string(), src_id.clone());
            }

            entities.push(EntityDoc {
                id: id.clone(),
                components: component_map,
                wires: wire_map,
            });
        }
    }

    world.insert_resource(components);
    world.insert_resource(wires);

    ProjectDoc { version: FORMAT_VERSION, entities }
}

/// One component per line, one wire per line — the format constraint M7's
/// in-place writer depends on (spec §2.2). `depth_limit` is what enforces it:
/// Project / entities / Entity / maps are formatted, and a payload below that
/// is written compactly on one line.
pub fn to_ron(doc: &ProjectDoc) -> Result<String, ron::Error> {
    let config = ron::ser::PrettyConfig::new()
        .struct_names(true)
        .depth_limit(4)
        .indentor("    ")
        .compact_arrays(false);
    ron::ser::to_string_pretty(doc, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::TopologyDirty;
    use crate::project::apply::apply;
    use crate::project::doc::parse;
    use crate::project::registry::register_authorable;
    use crate::registry_wires::register_wire;
    use crate::test_wires::{FloatOut, Gain, GainFrom};
    use bevy_app::App;

    fn round_trip_app() -> App {
        let mut app = App::new();
        app.init_resource::<TopologyDirty>();
        register_wire::<GainFrom>(&mut app);
        register_authorable::<Gain>(&mut app, "Gain");
        register_authorable::<FloatOut>(&mut app, "FloatOut");
        app
    }

    const SOURCE: &str = r#"Project(version: 1, entities: [
        Entity(id: "src", components: { "FloatOut": (2.0) }),
        Entity(id: "dst", components: { "Gain": (factor: 0.0, value: 0.5) },
               wires: { "factor": "src" }),
    ])"#;

    #[test]
    fn a_world_emits_the_document_that_built_it() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));

        let emitted = to_document(app.world_mut());

        assert_eq!(emitted.version, FORMAT_VERSION);
        assert_eq!(emitted.entities.len(), 2);
        let dst = emitted.entities.iter().find(|e| e.id == "dst").expect("present");
        assert_eq!(dst.wires.get("factor").map(String::as_str), Some("src"));
        assert!(dst.components.contains_key("Gain"));
    }

    #[test]
    fn document_to_world_to_document_is_stable() {
        // The completeness check: anything the format cannot express is lost
        // here and the assertion fails.
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let once = to_document(app.world_mut());

        let mut second = round_trip_app();
        apply(second.world_mut(), &once);
        let twice = to_document(second.world_mut());

        assert_eq!(once, twice);
    }

    #[test]
    fn the_emitted_text_reparses() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let doc = to_document(app.world_mut());

        let text = to_ron(&doc).expect("emits");
        let reparsed = parse(&text).expect("the emitter writes what the parser reads");

        assert_eq!(reparsed, doc);
    }

    #[test]
    fn each_component_and_wire_gets_its_own_line() {
        // Spec §2.2: this is what lets M7's writer replace one line in place.
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        let text = to_ron(&to_document(app.world_mut())).expect("emits");

        let gain_line = text
            .lines()
            .find(|line| line.contains("\"Gain\""))
            .expect("the Gain component is written");
        assert!(
            gain_line.contains("factor") && gain_line.contains("value"),
            "the whole payload is on one line: {gain_line}"
        );
        assert_eq!(
            text.lines().filter(|line| line.contains("\"factor\": \"src\"")).count(),
            1,
            "the wire is one line"
        );
    }

    #[test]
    fn an_entity_without_a_doc_id_is_not_in_the_document() {
        let mut app = round_trip_app();
        apply(app.world_mut(), &parse(SOURCE).expect("parses"));
        app.world_mut().spawn(FloatOut(9.0));

        let doc = to_document(app.world_mut());

        assert_eq!(doc.entities.len(), 2, "the runtime-owned entity stayed out");
    }
}
```

Add to `crates/sway-graph/src/project/mod.rs`:

```rust
pub mod emit;

pub use emit::{to_document, to_ron};
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p sway-graph --lib project::emit::`

Expected: all five PASS. If `each_component_and_wire_gets_its_own_line` fails
because a payload is split across lines, adjust `depth_limit` by one and re-run
— 4 is the calculated depth (Project → entities → Entity → map), not a measured
one, and the test is the measurement.

- [ ] **Step 3: Commit**

```bash
git add crates/sway-graph/src/project/emit.rs crates/sway-graph/src/project/mod.rs
git commit -m "feat(graph): emit a document from the world, proving the format complete"
```

---

### Task 10: Register the slice, and author the demo as a document

**Files:**
- Modify: `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/src/osc.rs`, `crates/sway-nodes/src/outputs.rs`
- Create: `crates/sway-app/src/demo_assets.rs`, `assets/demo.sway.ron`
- Modify: `crates/sway-app/src/main.rs`, `crates/sway-app/Cargo.toml`, `crates/sway-runtime/src/headless.rs`
- Delete: `crates/sway-app/src/demo_graph.rs`

**Interfaces:**
- Consumes: `register_authorable`, `ProjectPlugin`, `ProjectHandle`.
- Produces:
  - `sway_nodes::WireNodesPlugin` also registers `Lfo`, `FloatOut`, `Vec3Out`, `Transform`, `EditorPos` as authorable.
  - `sway_app::demo_assets::{DemoCube, DemoAssetsPlugin}`

- [ ] **Step 1: Make the slice's components reflectable**

In `crates/sway-nodes/src/outputs.rs`:

```rust
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FloatOut(pub f32);

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct Vec3Out(pub Vec3);
```

In `crates/sway-nodes/src/osc.rs`, on `Lfo`:

```rust
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct Lfo {
```

`Waveform` (in `src/lfo.rs`) must also derive `Reflect` and be registered, or
`Lfo`'s registration fails at startup. Add `Reflect` to its derive list and
`app.register_type::<Waveform>()` in `WireNodesPlugin`.

- [ ] **Step 2: Register them as authorable**

In `crates/sway-nodes/src/lib.rs`, extend `WireNodesPlugin::build`:

```rust
impl bevy_app::Plugin for WireNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_behaviour::<Lfo>(app, lfo_behaviour);
        sway_graph::register_wire::<AmplitudeFrom>(app);
        sway_graph::register_wire::<TranslationYFrom>(app);
        sway_graph::register_wire::<bevy::prelude::ChildOf>(app);

        // What a project document may name (M4). Short names, not type paths.
        app.register_type::<Waveform>();
        sway_graph::register_authorable::<Lfo>(app, "Lfo");
        sway_graph::register_authorable::<FloatOut>(app, "FloatOut");
        sway_graph::register_authorable::<Vec3Out>(app, "Vec3Out");
        sway_graph::register_authorable::<bevy::prelude::Transform>(app, "Transform");
        sway_graph::register_authorable::<sway_graph::EditorPos>(app, "EditorPos");
    }
}
```

`EditorPos` needs `#[reflect(Component, Default, PartialEq)]` and a `Default`
impl; add both in `crates/sway-graph/src/ctx.rs`:

```rust
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component, Default, PartialEq)]
pub struct EditorPos(pub Vec2);
```

Add a test to `crates/sway-nodes/src/lib.rs`'s `mod tests`:

```rust
    #[test]
    fn the_plugin_registers_every_authorable_component() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(WireNodesPlugin);

        let registry = app.world().resource::<sway_graph::ComponentDocRegistry>();
        let mut names: Vec<&str> = registry.entries.iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["EditorPos", "FloatOut", "Lfo", "Transform", "Vec3Out"]
        );
    }
```

- [ ] **Step 3: Write the demo document**

Create `assets/demo.sway.ron` at the **repo root** (Bevy's asset root is
`assets/` relative to the working directory):

```ron
// The wire-model demo, as a document.
//
//   Lfo A ──amplitude──▶ Lfo B ──translation.y──▶ cube B
//         └─translation.y──▶ cube A            (fan-out)
//   group ──parent──▶ cube A, cube B
//
// The cubes' mesh and material are not here: a Handle is asset flow, which is
// M5. `DemoCube` is a marker a plain Bevy system reacts to — deliberately the
// ugly seam, and exactly the shape of what M5 replaces.
Project(
    version: 1,
    entities: [
        Entity(
            id: "lfoA",
            components: {
                "Lfo": (beats: 8.0, shape: Sine, phase: 0.0, amplitude: 0.5),
                "FloatOut": (0.0),
                "EditorPos": ((-320.0, 40.0)),
            },
        ),
        Entity(
            id: "lfoB",
            components: {
                "Lfo": (beats: 2.0, shape: Sine, phase: 0.0, amplitude: 0.0),
                "FloatOut": (0.0),
                "EditorPos": ((-120.0, 40.0)),
            },
            wires: { "amplitude": "lfoA" },
        ),
        Entity(
            id: "group",
            components: { "Transform": (), "EditorPos": ((80.0, 200.0)) },
        ),
        Entity(
            id: "cubeA",
            components: {
                "Transform": (translation: (-0.8, 0.0, 0.0)),
                "DemoCube": (),
                "EditorPos": ((80.0, 40.0)),
            },
            wires: { "translation.y": "lfoA", "parent": "group" },
        ),
        Entity(
            id: "cubeB",
            components: {
                "Transform": (translation: (0.8, 0.0, 0.0)),
                "DemoCube": (),
                "EditorPos": ((80.0, 120.0)),
            },
            wires: { "translation.y": "lfoB", "parent": "group" },
        ),
    ],
)
```

- [ ] **Step 4: The marker and its mesh system**

Create `crates/sway-app/src/demo_assets.rs`:

```rust
//! The one thing the document cannot say yet.
//!
//! A `Handle<Mesh>` is asset flow and asset flow is M5 (project spec §8), so
//! the document authors a marker and this attaches the renderable parts. When
//! M5 lands, this file goes away.

use bevy::prelude::*;
use sway_graph::register_authorable;

#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct DemoCube;

#[derive(Resource)]
struct CubeAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn create_cube_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(CubeAssets {
        mesh: meshes.add(Cuboid::new(0.6, 0.6, 0.6)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.7, 0.9),
            ..default()
        }),
    });
}

/// An ordinary `Added<T>` system — the second row of the parent spec's
/// behaviour table (§2.2): it consumes and produces nothing the graph reads,
/// so it has no business being in the order.
fn attach_cube_visuals(
    mut commands: Commands,
    assets: Res<CubeAssets>,
    added: Query<Entity, Added<DemoCube>>,
) {
    for entity in &added {
        commands.entity(entity).insert((
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.material.clone()),
            Visibility::default(),
        ));
    }
}

pub struct DemoAssetsPlugin;

impl Plugin for DemoAssetsPlugin {
    fn build(&self, app: &mut App) {
        register_authorable::<DemoCube>(app, "DemoCube");
        app.add_systems(Startup, create_cube_assets)
            .add_systems(Update, attach_cube_visuals.run_if(resource_exists::<CubeAssets>));
    }
}
```

- [ ] **Step 5: Load it from the app**

In `crates/sway-app/src/main.rs`: replace `mod demo_graph;` with
`mod demo_assets;`, delete the `use demo_graph::setup_demo_graph;` import, and
replace `.add_systems(Startup, setup_demo_graph)` with a startup system that
asks the asset server for the document:

```rust
fn load_project(asset_server: Res<AssetServer>, mut handle: ResMut<sway_graph::ProjectHandle>) {
    handle.0 = Some(asset_server.load("demo.sway.ron"));
}
```

and register it plus the two plugins:

```rust
        .add_plugins((
            sway_graph::WiresPlugin,
            sway_graph::ProjectPlugin,
            sway_nodes::WireNodesPlugin,
            demo_assets::DemoAssetsPlugin,
        ))
        .add_systems(Startup, load_project)
```

Then `git rm crates/sway-app/src/demo_graph.rs`.

- [ ] **Step 6: Turn on file watching**

In `crates/sway-app/Cargo.toml`, give the `bevy` dependency the feature:

```toml
bevy = { workspace = true, features = ["file_watcher"] }
```

In `crates/sway-runtime/src/headless.rs`, add to the `DefaultPlugins` chain,
beside the existing `.set(...)` calls:

```rust
            .set(AssetPlugin {
                // M4: editing the project document with the app running is the
                // whole point of the milestone.
                watch_for_changes_override: Some(true),
                ..default()
            })
```

with `use bevy::asset::AssetPlugin;` added to the imports.

- [ ] **Step 7: Run the tests**

Run: `cargo test --workspace`

Expected: all PASS. `sway-app`'s deleted `demo_graph` tests go with the file;
`sway-nodes`'s new registration test passes.

Run: `cargo build --workspace`

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(app): author the demo as a document rather than as Rust"
```

---

### Task 11: The read-only inspector

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`
- Create: `crates/sway-editor/src/inspector.rs`
- Modify: `crates/sway-editor/src/lib.rs`, `crates/sway-app/src/presenter.rs`

**Interfaces:**
- Consumes: `ComponentDocRegistry`, `AppTypeRegistry`.
- Produces:
  - `struct InspectorComponent { pub name: String, pub fields: Vec<(String, String)> }`
  - `struct InspectorView { pub entity: Option<Entity>, pub components: Vec<InspectorComponent> }`
  - `fn inspect(world: &World, entity: Entity) -> InspectorView`
  - `WorldSnapshot::inspector: InspectorView`
  - `EditorUi::selected_entity(&mut self) -> Option<Entity>`
  - `pub const INSPECTOR_TAG: WidgetTag<Inspector>`

- [ ] **Step 1: Write the reflect walk and its failing tests**

Add to `crates/sway-editor/src/snapshot.rs`:

```rust
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_reflect::{PartialReflect, ReflectRef};
use sway_graph::ComponentDocRegistry;

/// One component's authored fields, flattened for display.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorComponent {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

/// What the inspector pane shows for the current selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorView {
    pub entity: Option<Entity>,
    pub components: Vec<InspectorComponent>,
}

/// The authorable components on `entity`, walked by reflection.
///
/// The same walk the project format's reader and emitter perform, which is the
/// point: this is what finally exercises editor `TypeData` (parent spec §7).
pub fn inspect(world: &World, entity: Entity) -> InspectorView {
    let (Some(docs), Some(registry)) = (
        world.get_resource::<ComponentDocRegistry>(),
        world.get_resource::<AppTypeRegistry>(),
    ) else {
        return InspectorView::default();
    };
    let registry = registry.read();
    let Ok(entity_ref) = world.get_entity(entity) else {
        return InspectorView::default();
    };

    let mut components = Vec::new();
    for entry in &docs.entries {
        let Some(registration) = registry.get(entry.type_id) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(value) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        components.push(InspectorComponent {
            name: entry.name.to_string(),
            fields: fields_of(value.as_partial_reflect()),
        });
    }

    InspectorView { entity: Some(entity), components }
}

fn fields_of(value: &dyn PartialReflect) -> Vec<(String, String)> {
    match value.reflect_ref() {
        ReflectRef::Struct(s) => (0..s.field_len())
            .map(|i| {
                (
                    s.name_at(i).unwrap_or("?").to_string(),
                    format_value(s.field_at(i).expect("index in range")),
                )
            })
            .collect(),
        ReflectRef::TupleStruct(t) => (0..t.field_len())
            .map(|i| (i.to_string(), format_value(t.field(i).expect("index in range"))))
            .collect(),
        ReflectRef::Enum(e) => vec![("variant".to_string(), e.variant_name().to_string())],
        _ => vec![(String::new(), format_value(value))],
    }
}

/// Renders the types a set actually uses; anything else falls back to its
/// debug form, which is the signal that the type wants editor `TypeData`.
fn format_value(value: &dyn PartialReflect) -> String {
    if let Some(v) = value.try_downcast_ref::<f32>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<f64>() {
        return format!("{v:.3}");
    }
    if let Some(v) = value.try_downcast_ref::<bool>() {
        return v.to_string();
    }
    if let Some(v) = value.try_downcast_ref::<u32>() {
        return v.to_string();
    }
    if let Some(v) = value.try_downcast_ref::<String>() {
        return v.clone();
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec2>() {
        return format!("{:.2}, {:.2}", v.x, v.y);
    }
    if let Some(v) = value.try_downcast_ref::<bevy_math::Vec3>() {
        return format!("{:.2}, {:.2}, {:.2}", v.x, v.y, v.z);
    }
    if let ReflectRef::Enum(e) = value.reflect_ref() {
        return e.variant_name().to_string();
    }
    format!("{value:?}")
}
```

Add `pub inspector: InspectorView,` to `WorldSnapshot`.

Add tests to `snapshot.rs`'s `mod tests` (the module already builds a test
world — follow whatever `test_graph.rs` provides):

```rust
    #[test]
    fn the_inspector_lists_authorable_components_and_their_fields() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app
            .world_mut()
            .spawn(sway_nodes::Lfo { beats: 4.0, shape: sway_nodes::Waveform::Saw, phase: 0.25, amplitude: 0.5 })
            .id();

        let view = inspect(app.world(), entity);

        let lfo = view
            .components
            .iter()
            .find(|c| c.name == "Lfo")
            .expect("Lfo is authorable and present");
        assert_eq!(lfo.fields.len(), 4);
        assert_eq!(lfo.fields[0], ("beats".to_string(), "4.000".to_string()));
        assert!(lfo.fields.iter().any(|(name, value)| name == "shape" && value == "Saw"));
    }

    #[test]
    fn a_component_the_entity_does_not_have_is_not_listed() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin)
            .add_plugins(sway_nodes::WireNodesPlugin);
        let entity = app.world_mut().spawn(sway_nodes::FloatOut(1.0)).id();

        let view = inspect(app.world(), entity);

        assert_eq!(view.components.len(), 1);
        assert_eq!(view.components[0].name, "FloatOut");
    }

    #[test]
    fn inspecting_a_dead_entity_is_empty_not_a_panic() {
        let mut app = bevy_app::App::new();
        app.add_plugins(sway_graph::WiresPlugin);
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(entity);

        assert_eq!(inspect(app.world(), entity), InspectorView::default());
    }
```

`sway-editor` will need `sway-nodes` as a **dev-dependency** for these tests
(only the tests — the crate itself must not depend on it). Add to
`crates/sway-editor/Cargo.toml`:

```toml
[dev-dependencies]
sway-nodes.workspace = true
bevy_app.workspace = true
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p sway-editor --lib snapshot::`

Expected: the three new tests PASS alongside the existing ones. Any component
whose fields come out as `Dynamic...` debug output is a type wanting
`TypeData` — note which, since that discovery is the reason this exists.

- [ ] **Step 3: Commit the read half**

```bash
git add crates/sway-editor/src/snapshot.rs crates/sway-editor/Cargo.toml
git commit -m "feat(editor): walk an entity's authorable components by reflection"
```

- [ ] **Step 4: Write the pane**

Create `crates/sway-editor/src/inspector.rs`, following `scene_tree.rs`'s
structure exactly — `Label` children, a signature compared each frame so an
unchanged selection rebuilds nothing, and a `generation` counter a test can
assert on:

```rust
//! `Inspector` -- the selected entity's authored values, read-only.
//!
//! Rows are `Label` children for the same reason `SceneTree`'s are:
//! `imaging::Painter` takes only pre-shaped glyphs. Editing is M7; this pane
//! exists to prove the reflect walk and to surface types that still want
//! editor `TypeData`.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PropertiesMut, PropertiesRef,
    RegisterCtx, Widget, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

use crate::snapshot::WorldSnapshot;

pub const ROW_HEIGHT: f64 = 18.0;
const PADDING: f64 = 8.0;
const NATURAL_WIDTH: f64 = 240.0;

struct Row {
    pod: WidgetPod<Label>,
    /// Component headers are indented less than their fields.
    header: bool,
}

pub struct Inspector {
    rows: Vec<Row>,
    signature: Vec<String>,
    generation: u64,
}

impl Default for Inspector {
    fn default() -> Self {
        Self::new()
    }
}

impl Inspector {
    pub fn new() -> Self {
        Self { rows: Vec::new(), signature: Vec::new(), generation: 0 }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `(text, is_header)` for the current selection.
    fn lines(snap: &WorldSnapshot) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        if snap.inspector.entity.is_none() {
            lines.push(("nothing selected".to_string(), true));
            return lines;
        }
        for component in &snap.inspector.components {
            lines.push((component.name.clone(), true));
            for (name, value) in &component.fields {
                lines.push((format!("{name}  {value}"), false));
            }
        }
        if lines.is_empty() {
            lines.push(("no authored components".to_string(), true));
        }
        lines
    }

    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let lines = Self::lines(snap);
        let signature: Vec<String> = lines.iter().map(|(text, _)| text.clone()).collect();
        if signature == this.widget.signature {
            return;
        }

        for row in std::mem::take(&mut this.widget.rows) {
            this.ctx.remove_child(row.pod);
        }
        for (text, header) in &lines {
            let pod = WidgetPod::new(Label::new(text.clone()));
            this.ctx.register_child(&pod);
            this.widget.rows.push(Row { pod, header: *header });
        }
        this.widget.signature = signature;
        this.widget.generation += 1;
        this.ctx.request_layout();
    }
}
```

Implement `Widget` for `Inspector` by copying `SceneTree`'s impl and
simplifying: no selection, no pointer handling, rows laid out top to bottom at
`ROW_HEIGHT`, header rows at `PADDING` and field rows at `PADDING * 2`,
`measure` reporting `rows.len() * ROW_HEIGHT`, `paint` filling the background
the same colour `SceneTree` uses, and `accessibility` reporting `Role::List`.

Add to `crates/sway-editor/src/lib.rs`:

```rust
pub mod inspector;

use crate::inspector::Inspector;

/// Reaches the inspector pane from `EditorUi::apply_snapshot`.
pub const INSPECTOR_TAG: WidgetTag<Inspector> = WidgetTag::named("sway-inspector");
```

In `graph_root`, put the inspector under the scene tree in the left column:

```rust
    let inspector = Portal::new(Inspector::new().prepare().with_tag(INSPECTOR_TAG))
        .constrain_horizontal(true)
        .prepare();

    let left = Split::new(tree, inspector)
        .split_axis(Axis::Vertical)
        .split_fraction(0.6)
        .draggable(true)
        .solid_bar(true)
        .prepare();

    let panes = Split::new(left, right)
```

In `apply_snapshot`, push into it:

```rust
        self.root.edit_widget_with_tag(INSPECTOR_TAG, |mut inspector| {
            Inspector::apply_snapshot(&mut inspector, snap);
        });
```

And add the accessor the host needs:

```rust
    /// The entity the panes currently agree is selected.
    ///
    /// `sync_selection` keeps the tree and the canvas in step, so the tree's
    /// answer is the shared one.
    pub fn selected_entity(&mut self) -> Option<Entity> {
        self.root
            .edit_widget_with_tag(SCENE_TREE_TAG, |tree| tree.widget.selected())
    }
```

- [ ] **Step 5: Feed it from the host**

In `crates/sway-app/src/presenter.rs`, replace `apply_snapshot`'s body:

```rust
    fn apply_snapshot(&mut self, app: &App) {
        let mut snapshot = sway_editor::snapshot::capture(app.world());
        if let Some(entity) = self.editor.selected_entity() {
            snapshot.inspector = sway_editor::snapshot::inspect(app.world(), entity);
        }
        self.editor.apply_snapshot(&snapshot);
    }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-editor`

Expected: PASS, including whatever masonry widget tests exist. Add one:

```rust
    #[test]
    fn an_unchanged_selection_does_not_rebuild_the_inspector() {
        // Same discipline as SceneTree: a steady-state world costs one
        // comparison per frame.
        let mut ui = EditorUi::new(PhysicalSize::new(1200, 800), 1.0);
        let snap = WorldSnapshot::default();
        ui.apply_snapshot(&snap);
        let first = ui
            .root
            .edit_widget_with_tag(crate::INSPECTOR_TAG, |i| i.widget.generation());
        ui.apply_snapshot(&snap);
        let second = ui
            .root
            .edit_widget_with_tag(crate::INSPECTOR_TAG, |i| i.widget.generation());

        assert_eq!(first, second);
    }
```

- [ ] **Step 7: Commit**

```bash
git add crates/sway-editor crates/sway-app/src/presenter.rs
git commit -m "feat(editor): a read-only inspector, three milestones late"
```

---

### Task 12: End-to-end, by hand and by suite

The exit criterion is behavioural — "a set can be authored by editing text with
the app running" — so it is checked by doing exactly that.

**Files:**
- Modify: `docs/superpowers/specs/2026-07-25-sway-design.md` (status lines only)

**Interfaces:**
- Consumes: everything.
- Produces: a verified milestone.

- [ ] **Step 1: Full suite**

Run: `cargo build --workspace`
Expected: clean.

Run: `cargo test --workspace`
Expected: all PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: Check the tick path stayed clean**

Run:

```bash
grep -n "reflect\|Reflect" crates/sway-graph/src/run.rs crates/sway-graph/src/order.rs \
                           crates/sway-graph/src/wire.rs
```

Expected: nothing. A reflect call on the tick path violates this plan's global
constraints and the wires spec's success criteria.

- [ ] **Step 3: Run it**

Run: `cargo run -p sway-app -- --windowed --editor`

Expected: the two cubes appear and bob, exactly as before the document existed.
The inspector shows the selected entity's components.

- [ ] **Step 4: Edit with the app running — the actual exit criterion**

With the app still running, edit `assets/demo.sway.ron`:

1. Change `lfoB`'s `beats: 2.0` to `beats: 0.5`. Expected: cube B's bob speeds
   up within a frame or two of saving, without a restart.
2. Change `cubeA`'s wire `"translation.y": "lfoA"` to `"lfoB"`. Expected: both
   cubes now follow the same LFO.
3. Add a third entity copying `cubeB` with a different `translation.x`.
   Expected: a third cube appears, wired and parented.
4. Delete that entity. Expected: it disappears; the other two are unaffected
   and keep bobbing — **this is the reconcile working**; a respawn-everything
   implementation would visibly restart the whole scene.
5. Break the syntax — delete a closing paren. Expected: the scene keeps
   running, unchanged, and the error is reported rather than swallowed.
6. Fix it. Expected: the scene resumes updating from the file.

If step 1 does nothing, file watching is not on: check `file_watcher` in
`crates/sway-app/Cargo.toml` and `watch_for_changes_override` in
`headless.rs`, and confirm the app's working directory is the repo root.

- [ ] **Step 5: Update the roadmap status**

In `docs/superpowers/specs/2026-07-25-sway-design.md`:
- The header's status line: M4 complete, M5 next.
- §5's "Status at 2026-08-06" paragraph: the same.
- The M4 heading: `### M4 — Project format and hot reload (M) — **complete**`.

Add an *Outcome* paragraph under M4 recording what was actually found —
especially Task 1's verdicts, any component that needed `TypeData` the
inspector exposed, and anything the format could not express.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "docs: M4 complete — a set is authored by editing text"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §2.1 document shape, three keys | 2 |
| §2.2 one item per line | 9 (`each_component_and_wire_gets_its_own_line`) |
| §2.3 why not `DynamicScene` | n/a — a decision, not work |
| §2.4 id doubles as `Name`, identity | 5 |
| §3 `register_authorable`, `Default`/`PartialEq` requirements | 3 |
| §3 wire insert/remove/read | 4 |
| §4 loader, `AssetEvent`, four passes | 5, 6, 7, 8 |
| §4.2 what a reload preserves | 5 (`a_surviving_entity_keeps_its_entity_id`), 6 (`an_unregistered_component_...survives`) |
| §4.3 split failure handling | 6, 7 (per-item), 8 (`note_load_failures`) |
| §5 `to_document`, round-trip | 9 |
| §6 the inspector | 11 |
| §7 test 1 (the gate) | 1 |
| §7 tests 2–6 | 2, 5, 6, 7, 9, 12 |
| §8 the demo document and `DemoCube` | 10 |
| §8 `file_watcher` wiring | 10 |
| §9 scope: nothing from the "Out" list appears | — |

**Gaps found and closed during review:**

1. Task 5's first three tests used `World::new()`, which has no
   `AppTypeRegistry`; Task 6's component pass reads it. Task 6 Step 3 now says
   to add `init_resource::<AppTypeRegistry>()` or switch those tests to
   `doc_app()`.
2. `EditorPos` had no `Default` and no `#[reflect(Component)]`, so
   `register_authorable::<EditorPos>` would have panicked at startup — Task 10
   Step 2 fixes the type itself.
3. `Waveform` is a field of `Lfo`, so `Lfo`'s payload cannot deserialize unless
   `Waveform` is registered. Task 10 Step 1 registers it explicitly.
4. `sway-editor` must not depend on `sway-nodes` (its module doc states the
   rule), but Task 11's tests need real components. Added as a
   **dev**-dependency only, and Step 1 says so.
5. Task 4's `W::from(src)` depends on `Relationship::from` existing in Bevy
   0.19. Step 4 carries the exact fallback — a `Wire::make` method with four
   one-line impls — rather than leaving it to be improvised.

**Type consistency:** `ProjectDoc`/`EntityDoc` field names are identical across
Tasks 2, 5, 6, 7, 9. `apply(world, doc) -> ProjectDiagnostics` has the same
signature in Tasks 5, 6, 7, 8, 9. `ItemError`'s four variants are constructed in
Tasks 6 and 7 exactly as declared in Task 5. `ComponentEntry`'s fields
(`name`, `type_id`, `type_path`) are read in Tasks 6, 9 and 11 as declared in
Task 3. `WireEntry`'s three new fields keep the signatures declared in Task 4
through Tasks 7 and 9.

**Known risk carried into execution:** Task 1 may fail on any of its three
claims, and each has a different, named fallback that changes Tasks 2, 6 or 10
only. That is why it is first, and why its Step 3 records verdicts rather than
adapting silently.
