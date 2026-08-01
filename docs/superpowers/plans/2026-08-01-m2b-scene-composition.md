# M2b — Structure edges, geometry, and the cook gate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the graph structure edges (`ParentEdge`, `FeedsEdge`), a `Geometry` component, and a cook gate, then prove all three by replacing the M0 cube with a graph-authored `Grid → Displace → Mesh` scene driven by live MIDI.

**Architecture:** Every edge is an entity carrying `EdgeFrom`/`EdgeTo`. `compile` runs a structure pass (parenting and slots) before the existing dataflow pass, applies `ChildOf` only once validation passes, and emits a second `cook_order` over the `Feeds` DAG. `graph_tick` gains a second pass that runs a node's `cook` fn only when a sticky dirty flag on `NodeRuntime` says its effective inputs moved.

**Tech Stack:** Rust 2024, Bevy 0.19 (pinned), `bevy_reflect` for schema derivation, `bevy_ecs` relationships for edge cascade delete.

**Spec:** `docs/superpowers/specs/2026-08-01-m2b-scene-composition-design.md`. Section references below (§3, §6, …) are to that document unless prefixed "parent §", which means `docs/superpowers/specs/2026-07-25-sway-design.md`.

## Global Constraints

- **Bevy is pinned `=0.19.0`.** Do not change `bevy`, `wgpu` (`=29.0.4`), or `winit` (`=0.30.13`) versions in `Cargo.toml`. The pin holds the bevy↔vello alignment proven at M1b.
- **`sway-graph` must not depend on `bevy_render` or the `bevy` facade.** It may add `bevy_transform`. This manifest constraint is the only place the layering rule is enforced (parent §2.9).
- **`sway-geo` is headless at M2b.** No `bevy_render` dependency — it joins at M5 with the first GPU cook (§2).
- **Clippy gate is scoped**, per M2a's findings: `cargo clippy -p sway-graph -p sway-nodes -p sway-geo -- -D warnings`. `cargo clippy --workspace` was already red on `main` before this milestone; do not attribute that debt here.
- **Use `reflect_clone()`, never `to_dynamic()`,** for any arena or params value that must later downcast to its declared concrete type. `to_dynamic()` turns enums and structs into `Dynamic*` proxies that no longer downcast (M2a finding).
- **Ordinal identity is `(name, ordinal)`,** never name alone — an input and an output may share a name (`Remap.value`).
- **Derive time-varying values from absolute time; never accumulate per tick** (parent §2.6).
- **Asset writes are `get`, compare, then `get_mut`.** `Assets::get_mut` marks the asset changed by the act of being called (parent §2.11).
- **No node fires observer triggers at M2b.** Carried forward from M2a: the mechanism waits for a consumer.
- **`TICK_HZ` stays 120.0 and provisional.** §11 keeps the tick-rate question open.
- **Capability markers are bounded `TypePath + Send + Sync + 'static`, not `Reflect`.** The spec's §4 says `Produces: 'static`; `TypePath` is the working refinement — it supplies the name error messages print and is required for `Slot<T>`'s own type path, while still not forcing reflection onto `Geometry`'s `Arc<Vec<_>>` contents. `Geometry` therefore derives `TypePath` only.

---

## File Structure

**Created:**
- `crates/sway-geo/Cargo.toml`, `src/lib.rs` — new crate (§2)
- `crates/sway-geo/src/geometry.rs` — the `Geometry` component and `Attribute` (§5)
- `crates/sway-geo/src/grid.rs`, `src/displace.rs` — the two CPU operators (§8)
- `crates/sway-graph/src/slots.rs` — `Slot<T>`, `ReflectSlot`, `NoSlots`, `NoOutputs`, `derive_slots` (§4)
- `crates/sway-graph/src/structure.rs` — the structure validation pass (§4)
- `crates/sway-nodes/src/scene.rs` — `Group`, `Rgb` (§8)
- `crates/sway-nodes/src/material.rs` — `StandardMaterialNode`, `MaterialOf<M>` (§8)
- `crates/sway-nodes/src/mesh.rs` — the `Mesh` node (§8)
- `crates/sway-app/src/midi_feed.rs` — MIDI ingress moved out of `bridge.rs` (§9)
- `crates/sway-app/src/demo_graph.rs` — the M2b demo graph (§8, §9)

**Modified:**
- `crates/sway-graph/src/edges.rs` — `ParentEdge`, `FeedsEdge`, `NodeRuntime` gains the gate fields
- `crates/sway-graph/src/registry.rs` — `NodeType` gains `Slots`/`Produces`/`SLOT_ORDINALS`/`SPATIAL`/`COOKS`/`cook`/`produced_change_tick`
- `crates/sway-graph/src/compile.rs` — structure pass wiring, `ChildOf` application, `cook_order`
- `crates/sway-graph/src/tick.rs` — dirty-setting in gather/prefill, the cook pass
- `crates/sway-graph/src/view.rs` — `SlotView`
- `crates/sway-graph/src/schema.rs` — `SchemaError::UnregisteredSlotField`
- `crates/sway-graph/src/test_nodes.rs` — new associated types on existing probes, plus a cooking probe
- `crates/sway-nodes/src/{lfo,envelope,math,midi}.rs` — two associated types each
- `crates/sway-app/src/{main,scene}.rs` — wiring and the cube's removal

**Deleted:**
- `crates/sway-app/src/bridge.rs`

---

## Task 1: The `sway-geo` crate and the `Geometry` component

**Files:**
- Create: `crates/sway-geo/Cargo.toml`
- Create: `crates/sway-geo/src/lib.rs`
- Create: `crates/sway-geo/src/geometry.rs`
- Modify: `Cargo.toml` (workspace members and dependencies)

**Interfaces:**
- Consumes: nothing.
- Produces: `sway_geo::Geometry` with `Geometry::new(point_count: usize)`, `set(&mut self, name: impl Into<String>, attr: Attribute)`, `get(&self, name: &str) -> Option<&Attribute>`, `point_count(&self) -> usize`, `indices(&self) -> Option<&Arc<Vec<u32>>>`, `set_indices(&mut self, indices: Option<Arc<Vec<u32>>>)`, `attr_names(&self) -> impl Iterator<Item = &str>`; `sway_geo::Attribute` with variants `F32/Vec2/Vec3/Vec4/U32` and accessors `len`, `is_empty`, `as_f32`, `as_vec2`, `as_vec3`.

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, add `"crates/sway-geo"` to `members`, and under `[workspace.dependencies]` add:

```toml
sway-geo = { path = "crates/sway-geo" }
bevy_math = "=0.19.0"
```

- [ ] **Step 2: Write the crate manifest**

`crates/sway-geo/Cargo.toml`:

```toml
[package]
name = "sway-geo"
version.workspace = true
edition.workspace = true

# Headless at M2b. The parent spec's §3 puts this crate on the render side,
# but every cook here is CPU-side, so bevy_render has no consumer yet and
# joins at M5 (design §2).
[dependencies]
bevy_app.workspace = true
bevy_ecs.workspace = true
bevy_math.workspace = true
bevy_reflect.workspace = true
sway-graph.workspace = true
```

- [ ] **Step 3: Write the failing test**

`crates/sway-geo/src/geometry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn grid_of(count: usize) -> Geometry {
        let mut g = Geometry::new(count);
        g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; count])));
        g.set("N", Attribute::Vec3(Arc::new(vec![Vec3::Y; count])));
        g
    }

    #[test]
    fn cloning_shares_attribute_buffers_rather_than_copying_them() {
        // Design §5: "passing an unchanged attribute through an operator is a
        // refcount bump rather than a copy". This is the property that claim
        // rests on, so it is asserted rather than described.
        let a = grid_of(4);
        let b = a.clone();

        let (Some(Attribute::Vec3(pa)), Some(Attribute::Vec3(pb))) = (a.get("P"), b.get("P"))
        else {
            panic!("P must be a Vec3 attribute");
        };
        assert!(Arc::ptr_eq(pa, pb), "clone must share, not copy");
    }

    #[test]
    fn attribute_names_iterate_in_deterministic_order() {
        // BTreeMap, not HashMap: cook output is asserted directly and mesh
        // upload walks this map, so iteration order is observable (§5).
        let mut g = Geometry::new(2);
        g.set("uv", Attribute::Vec2(Arc::new(vec![Vec2::ZERO; 2])));
        g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; 2])));
        g.set("Cd", Attribute::Vec4(Arc::new(vec![Vec4::ONE; 2])));

        assert_eq!(g.attr_names().collect::<Vec<_>>(), vec!["Cd", "P", "uv"]);
    }

    #[test]
    fn an_attribute_of_the_wrong_length_is_rejected() {
        // A mismatched attribute is a cook bug that would otherwise surface
        // as an out-of-bounds index deep in mesh upload.
        let mut g = Geometry::new(4);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; 3])));
        }));
        assert!(result.is_err(), "length mismatch must not be accepted");
    }

    #[test]
    fn indices_round_trip_and_default_to_none() {
        let mut g = Geometry::new(3);
        assert!(g.indices().is_none());
        g.set_indices(Some(Arc::new(vec![0, 1, 2])));
        assert_eq!(g.indices().map(|i| i.as_slice()), Some([0u32, 1, 2].as_slice()));
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p sway-geo`
Expected: FAIL — `Geometry`, `Attribute` do not exist.

- [ ] **Step 5: Implement `Geometry` and `Attribute`**

At the top of `crates/sway-geo/src/geometry.rs`:

```rust
//! `Geometry` — a named, planar attribute table. Design §5.
//!
//! Planar rather than interleaved, as in Houdini and USD, which is also the
//! layout the GPU wants when M5 moves these buffers onto it. One component
//! holding a map rather than one component per attribute, because an author
//! can create `@myattr` at runtime and component types cannot be registered
//! then (parent §2.10).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::component::Component;
use bevy_math::{Vec2, Vec3, Vec4};
use bevy_reflect::TypePath;

/// One planar attribute column. `Arc` so an operator that rewrites `P` and
/// passes `N` through copies neither.
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    F32(Arc<Vec<f32>>),
    Vec2(Arc<Vec<Vec2>>),
    Vec3(Arc<Vec<Vec3>>),
    Vec4(Arc<Vec<Vec4>>),
    U32(Arc<Vec<u32>>),
}

impl Attribute {
    pub fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::Vec2(v) => v.len(),
            Self::Vec3(v) => v.len(),
            Self::Vec4(v) => v.len(),
            Self::U32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_f32(&self) -> Option<&Arc<Vec<f32>>> {
        match self {
            Self::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_vec2(&self) -> Option<&Arc<Vec<Vec2>>> {
        match self {
            Self::Vec2(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<&Arc<Vec<Vec3>>> {
        match self {
            Self::Vec3(v) => Some(v),
            _ => None,
        }
    }
}

/// Derives `TypePath` and not `Reflect`: this type is used as a *capability
/// marker* on `Slot<Geometry>` and `NodeType::Produces`, which need only a
/// name and a `TypeId`. Reflecting `Arc<Vec<Vec3>>` would be work with no
/// consumer (plan Global Constraints).
#[derive(Component, Clone, Debug, Default, TypePath)]
pub struct Geometry {
    attrs: BTreeMap<String, Attribute>,
    point_count: usize,
    indices: Option<Arc<Vec<u32>>>,
}

impl Geometry {
    pub fn new(point_count: usize) -> Self {
        Self {
            attrs: BTreeMap::new(),
            point_count,
            indices: None,
        }
    }

    pub fn point_count(&self) -> usize {
        self.point_count
    }

    /// Panics if `attr`'s length disagrees with `point_count`. A mismatched
    /// column is a cook bug, and the panic names it here rather than letting
    /// it surface as an out-of-bounds index during mesh upload.
    pub fn set(&mut self, name: impl Into<String>, attr: Attribute) {
        let name = name.into();
        assert_eq!(
            attr.len(),
            self.point_count,
            "attribute `{name}` has {} elements but this Geometry has {} points",
            attr.len(),
            self.point_count
        );
        self.attrs.insert(name, attr);
    }

    pub fn get(&self, name: &str) -> Option<&Attribute> {
        self.attrs.get(name)
    }

    pub fn attr_names(&self) -> impl Iterator<Item = &str> {
        self.attrs.keys().map(|k| k.as_str())
    }

    pub fn indices(&self) -> Option<&Arc<Vec<u32>>> {
        self.indices.as_ref()
    }

    pub fn set_indices(&mut self, indices: Option<Arc<Vec<u32>>>) {
        self.indices = indices;
    }
}
```

`crates/sway-geo/src/lib.rs`:

```rust
//! Geometry attribute tables and the CPU operators over them.
//! Spec: docs/superpowers/specs/2026-08-01-m2b-scene-composition-design.md

pub mod geometry;

pub use geometry::{Attribute, Geometry};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-geo`
Expected: PASS, 4 tests.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/sway-geo
git commit -m "feat(geo): sway-geo crate and the Geometry attribute table"
```

---

## Task 2: `Slot<T>`, `ReflectSlot`, and slot schema derivation

**Files:**
- Create: `crates/sway-graph/src/slots.rs`
- Modify: `crates/sway-graph/src/schema.rs` (new `SchemaError` variant)
- Modify: `crates/sway-graph/src/lib.rs` (module + re-exports)

**Interfaces:**
- Consumes: `SchemaError` from `crates/sway-graph/src/schema.rs:69`.
- Produces: `Slot<T>`, `NoSlots`, `NoOutputs`, `ReflectSlot { capability: TypeId, capability_path: &'static str }`, `register_slot::<T>(app: &mut App)`, `SlotField { name, field_index, capability, capability_path }`, `derive_slots<T: Typed>(&TypeRegistry) -> Result<Vec<SlotField>, SchemaError>`, `SlotSource { entity: Entity, plan_index: usize }`.

- [ ] **Step 1: Write the failing test**

`crates/sway-graph/src/slots.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_reflect::{Reflect, TypePath, TypeRegistry};

    #[derive(TypePath)]
    struct FakeGeometry;

    #[derive(TypePath)]
    struct FakeMaterial;

    #[derive(Reflect, Default)]
    struct MeshSlots {
        geo: Slot<FakeGeometry>,
        material: Slot<FakeMaterial>,
    }

    fn registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<MeshSlots>();
        r.register::<Slot<FakeGeometry>>();
        r.register_type_data::<Slot<FakeGeometry>, ReflectSlot>();
        r.register::<Slot<FakeMaterial>>();
        r.register_type_data::<Slot<FakeMaterial>, ReflectSlot>();
        r
    }

    #[test]
    fn a_slot_field_carries_its_capability_not_the_marker_type() {
        // The structure pass compares a source's Produces TypeId against
        // this, so it must be the capability, not Slot<capability> (§4).
        let slots = derive_slots::<MeshSlots>(&registry()).expect("slots");

        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].name, "geo");
        assert_eq!(slots[0].capability, core::any::TypeId::of::<FakeGeometry>());
        assert_ne!(
            slots[0].capability,
            core::any::TypeId::of::<Slot<FakeGeometry>>()
        );
        assert_eq!(slots[1].name, "material");
        assert_eq!(slots[1].capability, core::any::TypeId::of::<FakeMaterial>());
    }

    #[test]
    fn slot_ordinals_are_field_order() {
        let slots = derive_slots::<MeshSlots>(&registry()).expect("slots");
        assert_eq!(slots[0].field_index, 0);
        assert_eq!(slots[1].field_index, 1);
    }

    #[test]
    fn a_node_with_no_slots_derives_an_empty_list() {
        let mut r = TypeRegistry::new();
        r.register::<NoSlots>();
        assert!(derive_slots::<NoSlots>(&r).expect("empty").is_empty());
    }

    #[test]
    fn an_unregistered_slot_field_is_an_error_not_a_silent_omission() {
        // The failure this prevents: a node author adds a Slot<T> field but
        // forgets register_slot, the slot vanishes from the schema, and every
        // FeedsEdge into it reports "slot ordinal out of range" instead of
        // naming the real mistake.
        let mut r = TypeRegistry::new();
        r.register::<MeshSlots>();
        r.register::<Slot<FakeGeometry>>();
        r.register::<Slot<FakeMaterial>>();
        // deliberately NOT register_type_data::<_, ReflectSlot>

        let msg = derive_slots::<MeshSlots>(&r).unwrap_err().to_string();
        assert!(msg.contains("geo"), "message must name the field: {msg}");
        assert!(msg.contains("register_slot"), "message must say the fix: {msg}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sway-graph slots`
Expected: FAIL — module `slots` does not exist.

- [ ] **Step 3: Add the new `SchemaError` variant**

In `crates/sway-graph/src/schema.rs`, add to `enum SchemaError` (after `UnregisteredEventField`):

```rust
    UnregisteredSlotField {
        type_path: &'static str,
        field: &'static str,
    },
```

and to its `Display` impl:

```rust
            Self::UnregisteredSlotField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like a Feeds slot but its type is not \
                 registered as one — call `register_slot::<Capability>(app)` in this \
                 node type's `register`"
            ),
```

- [ ] **Step 4: Implement `slots.rs`**

Above the test module in `crates/sway-graph/src/slots.rs`:

```rust
//! `Feeds` slots: named, typed structural inputs. Design §4.
//!
//! A node's slots are derived from its `Slots` associated type exactly as its
//! ports are derived from `Params`/`Outputs` — the schema comes from the
//! types, never written beside them (parent §2.4). A field typed `Slot<T>` is
//! a slot accepting capability `T`.

use core::any::TypeId;
use core::marker::PhantomData;

use bevy_app::App;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_reflect::{FromType, Reflect, TypePath, TypeRegistry, Typed};

use crate::schema::SchemaError;

/// Type data marking a type as a slot marker, carrying the capability the
/// slot accepts.
#[derive(Clone)]
pub struct ReflectSlot {
    pub capability: TypeId,
    pub capability_path: &'static str,
}

impl<T: TypePath + Send + Sync + 'static> FromType<Slot<T>> for ReflectSlot {
    fn from_type() -> Self {
        Self {
            capability: TypeId::of::<T>(),
            capability_path: T::type_path(),
        }
    }
}

/// Marks a `Slots` field as a named `Feeds` input accepting capability `T`.
///
/// Zero-sized: a `Feeds` edge carries no value, and the target reads its
/// source's component or handle (parent §2.10). `PhantomData<fn() -> T>`
/// rather than `PhantomData<T>` so the marker is `Send + Sync` regardless of
/// `T`, matching `Event<T>`'s shape in `ports.rs`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default)]
pub struct Slot<T: TypePath + Send + Sync + 'static> {
    #[reflect(ignore)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: TypePath + Send + Sync + 'static> Default for Slot<T> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// The `Slots` type for a node with no structural inputs.
#[derive(Reflect, Default, Debug, Clone, Copy)]
pub struct NoSlots;

/// The `Outputs` type for a node with no output ports — the geometry
/// operators, whose product is a component rather than a port.
#[derive(Reflect, Default, Debug, Clone, Copy)]
pub struct NoOutputs;

/// Registers `Slot<T>` and its `ReflectSlot` data. A node type with a
/// `Slot<T>` field must call this in its `register`.
pub fn register_slot<T: TypePath + Send + Sync + 'static>(app: &mut App) {
    let registry = app.world().resource::<AppTypeRegistry>().clone();
    let mut registry = registry.write();
    registry.register::<Slot<T>>();
    registry.register_type_data::<Slot<T>, ReflectSlot>();
}

/// A resolved `Feeds` source: the entity a cook reads from, plus its position
/// in the compiled plans, which is how the cook gate reaches its
/// `produced_change_tick` fn without a second registry lookup.
///
/// Lives here rather than in `compile` so that `SlotView` (Task 3) can name it
/// without depending on compilation.
#[derive(Debug, Clone, Copy)]
pub struct SlotSource {
    pub entity: bevy_ecs::entity::Entity,
    pub plan_index: usize,
}

/// One slot, as derived from one `Slots` field.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotField {
    pub name: &'static str,
    pub field_index: usize,
    /// The capability this slot accepts — compared against the source node's
    /// `Produces` in the structure pass.
    pub capability: TypeId,
    pub capability_path: &'static str,
}

pub fn derive_slots<T: Typed>(registry: &TypeRegistry) -> Result<Vec<SlotField>, SchemaError> {
    let info = T::type_info();
    let s = info.as_struct().map_err(|_| SchemaError::NotAStruct {
        type_path: info.type_path(),
    })?;

    let mut slots = Vec::new();
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");
        match registry.get_type_data::<ReflectSlot>(field.type_id()) {
            Some(slot) => slots.push(SlotField {
                name: field.name(),
                field_index: i,
                capability: slot.capability,
                capability_path: slot.capability_path,
            }),
            None => {
                // Mirrors `schema::is_event_marker_path`: a `Slot<_>` field
                // whose type data is missing would otherwise silently not be
                // a slot at all.
                if is_slot_marker_path(field.type_path()) {
                    return Err(SchemaError::UnregisteredSlotField {
                        type_path: info.type_path(),
                        field: field.name(),
                    });
                }
            }
        }
    }
    Ok(slots)
}

/// Recognises `sway_graph::slots::Slot<..>` by path. The authoritative test
/// is the `ReflectSlot` type data above; this is the diagnostic for its
/// absence.
fn is_slot_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::slots::Slot<")
}
```

**If `#[derive(Reflect)]` on `Slot<T>` demands `T: Reflect`:** bevy_reflect derives bounds from *active* (non-ignored) fields, and `T` appears only behind `#[reflect(ignore)]`, so it should not. If the compiler disagrees, add `#[reflect(where T: TypePath + Send + Sync + 'static)]` above the struct rather than widening the bound on `T` — widening it would force `Reflect` onto `Geometry`, which the Global Constraints rule out.

**If `NoSlots`/`NoOutputs` unit structs do not report `TypeInfo::Struct`:** give each an explicit zero-field body (`pub struct NoSlots {}`) instead of making `derive_slots`/`derive_schema` special-case them.

- [ ] **Step 5: Wire the module**

In `crates/sway-graph/src/lib.rs`, add `pub mod slots;` alongside the other modules and extend the re-exports:

```rust
pub use slots::{
    NoOutputs, NoSlots, ReflectSlot, Slot, SlotField, SlotSource, derive_slots, register_slot,
};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-graph slots`
Expected: PASS, 4 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-graph/src/slots.rs crates/sway-graph/src/schema.rs crates/sway-graph/src/lib.rs
git commit -m "feat(graph): Slot<T>, ReflectSlot, and slot schema derivation"
```

---

## Task 3: `NodeType` gains slots, capabilities, and a cook fn

**Files:**
- Modify: `crates/sway-graph/src/registry.rs:32-45` (the trait), `:63-71` (`NodeTypeEntry`), `:88-128` (`register_node_type`)
- Modify: `crates/sway-graph/src/test_nodes.rs` (all probe types)
- Modify: `crates/sway-nodes/src/{lfo,envelope,math,midi}.rs` (all eight signal nodes)

**Interfaces:**
- Consumes: `SlotField`, `derive_slots`, `NoSlots`, `SlotSource` from Task 2.
- Produces: `SlotView::new(sources: &[Option<SlotSource>]) -> SlotView`, `SlotView::source(&self, slot: u16) -> Option<Entity>`; `NodeType::{Slots, Produces, SLOT_ORDINALS, SPATIAL, COOKS, cook, produced_change_tick}`; `NodeTypeEntry::{slots, produces, produces_path, spatial, cook, produced_change_tick}`; `pub type CookFn = fn(&mut World, Entity, &SlotView)`; `pub type ProducedTickFn = fn(&World, Entity) -> Option<Tick>`.

- [ ] **Step 0: Add `SlotView`, which the trait signature needs**

Append to `crates/sway-graph/src/view.rs`:

```rust
/// A node's scoped window onto its `Feeds` sources — what `PortView` is to
/// ports. Indices are the node's own slot ordinals, so a node cannot reach
/// another node's slot table by arithmetic (design §7).
pub struct SlotView<'a> {
    sources: &'a [Option<crate::slots::SlotSource>],
}

impl<'a> SlotView<'a> {
    pub fn new(sources: &'a [Option<crate::slots::SlotSource>]) -> Self {
        Self { sources }
    }

    /// The entity feeding `slot`, or `None` if the slot is empty. Panics on
    /// an out-of-range ordinal, for the same reason `PortView` does: a
    /// compiled graph has already validated every slot ordinal, so this can
    /// only be a stale index const.
    pub fn source(&self, slot: u16) -> Option<Entity> {
        let ordinal = slot as usize;
        assert!(
            ordinal < self.sources.len(),
            "SlotView: slot ordinal {slot} is out of range for this node's {} slot(s)",
            self.sources.len()
        );
        self.sources[ordinal].map(|source| source.entity)
    }
}
```

Add `use bevy_ecs::entity::Entity;` to `view.rs`'s imports, and add `SlotView` to `lib.rs`'s `view::` re-export line.

- [ ] **Step 1: Write the failing test**

Add to `crates/sway-graph/src/registry.rs`'s `mod tests`:

```rust
    #[test]
    fn a_node_type_registers_its_slots_capability_and_cook_flag() {
        use crate::slots::{NoSlots, Slot, register_slot};
        use bevy_reflect::TypePath;

        #[derive(TypePath)]
        struct Blob;

        #[derive(Reflect, Default)]
        struct ConsumerSlots {
            input: Slot<Blob>,
        }

        struct Producer;
        impl NodeType for Producer {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = NoSlots;
            type Produces = Blob;
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            const COOKS: bool = true;
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
            fn cook(_w: &mut World, _n: Entity, _s: &crate::view::SlotView) {}
        }

        struct Consumer;
        impl NodeType for Consumer {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = ConsumerSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("input", 0)];
            const SPATIAL: bool = true;
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
                register_slot::<Blob>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let producer = register_node_type::<Producer>(&mut app);
        let consumer = register_node_type::<Consumer>(&mut app);
        let reg = app.world().resource::<NodeTypeRegistry>();

        let p = reg.get(producer).expect("registered");
        assert!(p.slots.is_empty());
        assert_eq!(p.produces, core::any::TypeId::of::<Blob>());
        assert!(p.cook.is_some(), "COOKS = true must store the cook fn");
        assert!(!p.spatial);

        let c = reg.get(consumer).expect("registered");
        assert_eq!(c.slots.len(), 1);
        assert_eq!(c.slots[0].name, "input");
        assert_eq!(c.slots[0].capability, core::any::TypeId::of::<Blob>());
        assert_eq!(c.produces, core::any::TypeId::of::<()>());
        assert!(c.cook.is_none(), "COOKS defaults false — no cook stored");
        assert!(c.spatial);
    }

    #[test]
    fn a_wrong_slot_ordinal_fails_registration_and_names_the_slot() {
        use crate::slots::{Slot, register_slot};
        use bevy_reflect::TypePath;

        #[derive(TypePath)]
        struct Blob;

        #[derive(Reflect, Default)]
        struct TwoSlots {
            first: Slot<Blob>,
            second: Slot<Blob>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type Slots = TwoSlots;
            type Produces = ();
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0),
                ("bias", 1),
                ("value", 2),
                ("trigger", 0),
            ];
            // `second` is slot #1, not #0 — the mistake a field reorder makes.
            const SLOT_ORDINALS: &'static [(&'static str, u16)] =
                &[("first", 0), ("second", 0)];
            fn register(app: &mut App) {
                crate::schema::register_event_port::<NoteMsg>(app);
                register_slot::<Blob>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("second"), "must name the slot: {msg}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sway-graph registry`
Expected: FAIL — `NodeType` has no `Slots`/`Produces`, `NodeTypeEntry` has no `slots`.

- [ ] **Step 3: Extend the trait and the registry entry**

In `crates/sway-graph/src/registry.rs`, add the new fn-pointer aliases beside the existing ones (`:26-30`):

```rust
pub type CookFn = fn(&mut World, Entity, &crate::view::SlotView);
pub type ProducedTickFn = fn(&World, Entity) -> Option<Tick>;
```

Replace the trait (`:32-45`) with:

```rust
pub trait NodeType: 'static {
    type Params: Reflect + Typed + GetTypeRegistration + Component + Default;
    type Outputs: Reflect + Typed + GetTypeRegistration + Default;
    /// Named, typed `Feeds` inputs. `NoSlots` when the node has none (§4).
    type Slots: Reflect + Typed + GetTypeRegistration + Default;
    /// The capability a `Feeds` edge *from* this node carries. `()` means the
    /// node cannot be a `Feeds` source. Bounded `TypePath`, not `Reflect`:
    /// the structure pass needs identity and a name, nothing more (§4).
    type Produces: TypePath + Send + Sync + 'static;
    type State: Component + Default;

    /// `(field name, the ordinal the node's index const uses)` for every
    /// port. Verified against the reflect-derived schema at registration, so
    /// a field reorder fails at startup instead of silently swapping two
    /// ports (spec §3).
    const PORT_ORDINALS: &'static [(&'static str, u16)];
    /// The same guard for slots. Empty for a node with no slots.
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[];
    /// Does this node carry a `Transform`, i.e. may it appear in the scene
    /// tree? Parenting a non-spatial node is a compile error (§4).
    const SPATIAL: bool = false;
    /// Whether `cook` is meaningful. Rust cannot distinguish a defaulted
    /// trait method from an overridden one, and the gate needs to know
    /// whether a node has a cook at all rather than calling an empty one on
    /// every dirty node (§4).
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);

    /// Reads this node's `Feeds` sources and writes its own product. Runs in
    /// `cook_order`, only when the gate says the node is dirty (§6, §7).
    fn cook(_world: &mut World, _node: Entity, _slots: &crate::view::SlotView) {}

    /// The change tick of whatever this node's `Feeds` consumers depend on.
    ///
    /// `None` — the default — means "changes to what I produce do not require
    /// my consumers to re-cook", which is correct for a material node: its
    /// consumers hold its `Handle`, and editing the material's params does
    /// not change the handle. A geometry operator overrides this with its own
    /// `Geometry` change tick. Keeping it node-supplied is what lets the
    /// engine gate on geometry without `sway-graph` knowing `Geometry`
    /// exists (§6).
    fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> {
        None
    }
}
```

Extend `NodeTypeEntry` (`:63-71`) with:

```rust
    pub slots: Vec<crate::slots::SlotField>,
    pub produces: core::any::TypeId,
    pub produces_path: &'static str,
    pub spatial: bool,
    /// `Some` iff `N::COOKS`.
    pub cook: Option<CookFn>,
    pub produced_change_tick: ProducedTickFn,
```

- [ ] **Step 4: Fill the new fields in `register_node_type`**

In `register_node_type` (`:88`), register the slots type alongside `Params`/`Outputs`:

```rust
        w.register::<N::Slots>();
```

derive the slot schema beside the port schema:

```rust
    let slots = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        crate::slots::derive_slots::<N::Slots>(&r)
            .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>()))
    };

    check_slot_ordinals::<N>(&slots);
```

and extend the `NodeTypeEntry` construction:

```rust
        slots,
        produces: core::any::TypeId::of::<N::Produces>(),
        produces_path: <N::Produces as TypePath>::type_path(),
        spatial: N::SPATIAL,
        cook: if N::COOKS { Some(N::cook as CookFn) } else { None },
        produced_change_tick: N::produced_change_tick,
```

Add `TypePath` to the `bevy_reflect` import list at `:15`. Then add the guard beside `check_ordinals`:

```rust
/// The slot half of §3's startup guard. Slots occupy one flat space in field
/// order, so this is simpler than `check_ordinals` — but the hazard is the
/// same, and a swapped slot silently feeds a material into a geometry input.
fn check_slot_ordinals<N: NodeType>(slots: &[crate::slots::SlotField]) {
    let node = core::any::type_name::<N>();
    for (ordinal, slot) in slots.iter().enumerate() {
        let want = ordinal as u16;
        match N::SLOT_ORDINALS.iter().find(|(name, _)| *name == slot.name) {
            Some(&(_, got)) if got == want => {}
            Some(&(_, got)) => panic!(
                "{node}: slot `{}` is ordinal {want}, but SLOT_ORDINALS declares {got} \
                 — a field was reordered, or the const is stale",
                slot.name
            ),
            None => panic!(
                "{node}: slot `{}` is undeclared in SLOT_ORDINALS (expected ordinal {want})",
                slot.name
            ),
        }
    }
    for (name, _) in N::SLOT_ORDINALS {
        if !slots.iter().any(|s| s.name == *name) {
            panic!("{node}: SLOT_ORDINALS declares `{name}`, which is not a slot");
        }
    }
}
```

- [ ] **Step 5: Add the two associated types to every existing node type**

Associated type defaults are unstable in Rust, so each existing `impl NodeType` needs two lines. Add `type Slots = NoSlots;` and `type Produces = ();` (importing `sway_graph::NoSlots`, or `crate::slots::NoSlots` inside `sway-graph`) to every one of:

- `crates/sway-graph/src/test_nodes.rs` — `Probe`, `IntProbe`, `Emitter`, `Gain`, `Sink`
- `crates/sway-graph/src/registry.rs` `mod tests` — `Probe`, `Bad`, `Incomplete`, `Phantom`, `SameName`
- `crates/sway-nodes/src/midi.rs` — `MidiNote`, `MidiCC`
- `crates/sway-nodes/src/lfo.rs` — `LFO`
- `crates/sway-nodes/src/envelope.rs` — `Envelope`
- `crates/sway-nodes/src/math.rs` — `Math`, `Remap`, `Switch`, `Select`

No other member of those impls changes.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-graph && cargo test -p sway-nodes`
Expected: PASS, including the two new registry tests and every pre-existing test.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-graph crates/sway-nodes
git commit -m "feat(graph): NodeType declares slots, capabilities, and a cook fn"
```

---

## Task 4: `ParentEdge`, `FeedsEdge`, and the structure validation pass

**Files:**
- Modify: `crates/sway-graph/src/edges.rs`
- Create: `crates/sway-graph/src/structure.rs`
- Modify: `crates/sway-graph/src/compile.rs` (new `CompileError` variants and their `Display` arms)
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `SlotField` (Task 2), `NodeTypeEntry::{slots, produces, produces_path, spatial}` (Task 3), `EdgeFrom`/`EdgeTo` from `edges.rs:44-62`.
- Produces: `ParentEdge`, `FeedsEdge { slot: u16 }`; `structure::StructureNode`, `structure::Structure { cook_order: Vec<usize>, slots: Vec<Vec<Option<usize>>>, parents: Vec<Option<usize>> }` (all indices are into the `nodes` slice), `structure::validate(world: &mut World, nodes: &[StructureNode], index_of: &HashMap<Entity, usize>) -> Result<Structure, CompileError>`.

- [ ] **Step 1: Add the two edge components**

In `crates/sway-graph/src/edges.rs`, after `ParamEdge`:

```rust
/// A hierarchy edge. **Source is the child, target is the parent** — dataflow
/// runs leaf→root while parenting runs root→leaf (parent §2.10).
///
/// Authored as an edge entity rather than as Bevy's `ChildOf` directly, and
/// compiled into `ChildOf` once validation passes. §2.5 requires a `ChildOf`
/// fan-out to be a diagnosable error, and an entity holds exactly one
/// `ChildOf` — inserting a second replaces the first silently, so the illegal
/// state would be unrepresentable and the diagnostic unwritable (design §3).
#[derive(Component)]
pub struct ParentEdge;

/// A structural input edge into a named, typed slot on the target.
///
/// Also an edge entity, for the same diagnostic reason plus one of its own: a
/// node needs several slots at once (`Mesh` has `geo` and `material`) and one
/// Bevy relationship component per entity cannot carry two targets.
#[derive(Component)]
pub struct FeedsEdge {
    /// Ordinal within the target node type's `Slots` schema.
    pub slot: u16,
}
```

- [ ] **Step 2: Write the failing test**

`crates/sway-graph/src/structure.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::test_nodes::{
        spawn_group, spawn_probe, spawn_sinkgeo, spawn_source, structure_app,
    };

    fn feeds(world: &mut World, from: Entity, to: Entity, slot: u16) -> Entity {
        world
            .spawn((FeedsEdge { slot }, EdgeFrom(from), EdgeTo(to)))
            .id()
    }

    fn parent(world: &mut World, child: Entity, parent: Entity) -> Entity {
        world
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(parent)))
            .id()
    }

    #[test]
    fn a_feeds_chain_orders_producer_before_consumer() {
        let mut app = structure_app();
        let src = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), src, sink, 0);

        let compiled = compile(app.world_mut()).expect("compiles");
        let cooked: Vec<Entity> = compiled
            .cook_order
            .iter()
            .map(|&i| compiled.plans[i].entity)
            .collect();
        let src_at = cooked.iter().position(|&e| e == src).expect("source cooks");
        let sink_at = cooked.iter().position(|&e| e == sink).expect("sink cooks");
        assert!(src_at < sink_at, "a Feeds source must cook first");
    }

    #[test]
    fn two_parent_edges_from_one_child_are_rejected() {
        let mut app = structure_app();
        let child = spawn_group(app.world_mut());
        let a = spawn_group(app.world_mut());
        let b = spawn_group(app.world_mut());
        parent(app.world_mut(), child, a);
        parent(app.world_mut(), child, b);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("one parent"), "vocabulary of the edge kind: {msg}");
        assert!(msg.contains(&format!("{child}")), "must name the child: {msg}");
        assert!(
            msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")),
            "must name both proposed parents: {msg}"
        );
    }

    #[test]
    fn parenting_a_non_spatial_node_is_rejected() {
        let mut app = structure_app();
        let lfo_like = spawn_probe(app.world_mut()); // SPATIAL = false
        let group = spawn_group(app.world_mut());
        parent(app.world_mut(), lfo_like, group);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("scene node"), "{msg}");
        assert!(msg.contains(&format!("{lfo_like}")), "must name the node: {msg}");
    }

    #[test]
    fn a_parenting_cycle_is_rejected() {
        let mut app = structure_app();
        let a = spawn_group(app.world_mut());
        let b = spawn_group(app.world_mut());
        parent(app.world_mut(), a, b);
        parent(app.world_mut(), b, a);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("parent"), "must speak of parenting, not dataflow: {msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_slot_filled_twice_is_rejected() {
        let mut app = structure_app();
        let a = spawn_source(app.world_mut());
        let b = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), a, sink, 0);
        feeds(app.world_mut(), b, sink, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("already filled"), "{msg}");
        assert!(msg.contains("input"), "must name the slot: {msg}");
        assert!(
            msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")),
            "must name both sources: {msg}"
        );
    }

    #[test]
    fn a_slot_type_mismatch_names_the_capability_on_both_sides() {
        let mut app = structure_app();
        // `Group` produces nothing; feeding it into a Blob slot must not be
        // reported as a generic "cycle" or "out of range".
        let group = spawn_group(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), group, sink, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{group}")), "must name the source: {msg}");
        assert!(msg.contains("input"), "must name the slot: {msg}");
    }

    #[test]
    fn a_slot_ordinal_out_of_range_reports_the_arity() {
        let mut app = structure_app();
        let src = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), src, sink, 9);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains('9'), "{msg}");
        assert!(msg.contains('1'), "must state the Slots arity: {msg}");
    }

    #[test]
    fn a_feeds_cycle_is_rejected_in_feeds_vocabulary() {
        let mut app = structure_app();
        let a = spawn_sinkgeo(app.world_mut()); // has a slot AND produces
        let b = spawn_sinkgeo(app.world_mut());
        feeds(app.world_mut(), a, b, 0);
        feeds(app.world_mut(), b, a, 0);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("Feeds"), "must name the edge kind: {msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }
}
```

- [ ] **Step 3: Add the test node types the tests need**

In `crates/sway-graph/src/test_nodes.rs`, add a capability marker, three node types, their spawn helpers, and `structure_app()`. `Source` produces `Blob` and cooks; `SinkGeo` has one `Blob` slot, produces `Blob` and cooks; `Group` is spatial and produces nothing.

```rust
use bevy_reflect::TypePath;
use crate::slots::{NoOutputs, NoSlots, Slot, register_slot};

/// A stand-in capability. `sway-graph` cannot depend on `sway-geo`, so its
/// structural tests carry their own marker and their own produced component.
#[derive(TypePath)]
pub(crate) struct Blob;

/// What a `Source`/`SinkGeo` cook writes. Its change tick is what
/// `produced_change_tick` reports.
#[derive(Component, Default, Debug, Clone, PartialEq)]
pub(crate) struct BlobData(pub u32);

#[derive(Reflect, Component, Default)]
pub(crate) struct SourceParams {
    pub seed: f32,
}

#[derive(Component, Default)]
pub(crate) struct SourceState;

pub(crate) struct Source;

impl Source {
    pub(crate) const SEED: u16 = 0;
}

impl NodeType for Source {
    type Params = SourceParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = Blob;
    type State = SourceState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("seed", Self::SEED)];
    const COOKS: bool = true;

    fn register(_app: &mut App) {}

    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _slots: &SlotView) {
        let seed = world.get::<SourceParams>(node).map(|p| p.seed).unwrap_or(0.0);
        world.entity_mut(node).insert(BlobData(seed as u32));
        world.resource_mut::<CookCounter>().0 += 1;
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<BlobData>()
            .map(|t| t.changed)
    }
}

#[derive(Reflect, Default)]
pub(crate) struct SinkGeoSlots {
    pub input: Slot<Blob>,
}

#[derive(Reflect, Component, Default)]
pub(crate) struct SinkGeoParams {
    pub scale: f32,
}

#[derive(Component, Default)]
pub(crate) struct SinkGeoState;

pub(crate) struct SinkGeo;

impl SinkGeo {
    pub(crate) const SCALE: u16 = 0;
    pub(crate) const IN_INPUT: u16 = 0;
}

impl NodeType for SinkGeo {
    type Params = SinkGeoParams;
    type Outputs = NoOutputs;
    type Slots = SinkGeoSlots;
    type Produces = Blob;
    type State = SinkGeoState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("scale", Self::SCALE)];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("input", Self::IN_INPUT)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_slot::<Blob>(app);
    }

    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        let upstream = slots
            .source(SinkGeo::IN_INPUT)
            .and_then(|src| world.get::<BlobData>(src))
            .map(|b| b.0)
            .unwrap_or(0);
        let scale = world.get::<SinkGeoParams>(node).map(|p| p.scale).unwrap_or(1.0);
        world
            .entity_mut(node)
            .insert(BlobData(upstream * scale as u32));
        world.resource_mut::<CookCounter>().0 += 1;
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<BlobData>()
            .map(|t| t.changed)
    }
}

#[derive(Reflect, Component, Default)]
pub(crate) struct GroupParams {
    pub y: f32,
}

#[derive(Component, Default)]
pub(crate) struct GroupState;

pub(crate) struct Group;

impl NodeType for Group {
    type Params = GroupParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = GroupState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("y", 0)];
    const SPATIAL: bool = true;

    fn register(_app: &mut App) {}
    fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
}

/// Counts cooks, so the gate's negative assertions have something to assert
/// on rather than an output that merely happens to be unchanged (§7).
#[derive(Resource, Default)]
pub(crate) struct CookCounter(pub u32);

pub(crate) fn spawn_source(world: &mut World) -> Entity {
    let node_type = node_type_id::<Source>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            SourceParams { seed: 1.0 },
            SourceState,
        ))
        .id()
}

pub(crate) fn spawn_sinkgeo(world: &mut World) -> Entity {
    let node_type = node_type_id::<SinkGeo>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            SinkGeoParams { scale: 1.0 },
            SinkGeoState,
        ))
        .id()
}

pub(crate) fn spawn_group(world: &mut World) -> Entity {
    let node_type = node_type_id::<Group>(world);
    world
        .spawn((
            GraphNode { id: next_node_id(), node_type },
            GroupParams::default(),
            GroupState,
        ))
        .id()
}

pub(crate) fn structure_app() -> App {
    let mut app = App::new();
    app.add_plugins(crate::tick::GraphPlugin);
    app.init_resource::<CookCounter>();
    register_node_type::<Probe>(&mut app);
    register_node_type::<Source>(&mut app);
    register_node_type::<SinkGeo>(&mut app);
    register_node_type::<Group>(&mut app);
    app
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p sway-graph structure`
Expected: FAIL — `structure` module and the new `CompileError` variants do not exist.

- [ ] **Step 5: Add the `CompileError` variants**

In `crates/sway-graph/src/compile.rs`, extend `enum CompileError`:

```rust
    DuplicateParent {
        child: Entity,
        first: Entity,
        second: Entity,
    },
    NotSpatial {
        node: Entity,
        type_name: &'static str,
        /// `"parented"` or `"used as a parent"`.
        role: &'static str,
    },
    ParentCycle {
        nodes: Vec<Entity>,
    },
    SlotOutOfRange {
        node: Entity,
        slot: u16,
        arity: usize,
    },
    DuplicateSlot {
        target: Entity,
        slot: &'static str,
        first: Entity,
        second: Entity,
    },
    SlotTypeMismatch {
        target: Entity,
        slot: &'static str,
        expected: &'static str,
        source: Entity,
        produces: &'static str,
    },
    SourceProducesNothing {
        source: Entity,
        type_name: &'static str,
        target: Entity,
        slot: &'static str,
    },
    FeedsCycle {
        nodes: Vec<Entity>,
    },
```

and their `Display` arms — each speaking its own edge kind's vocabulary, per parent §4:

```rust
            Self::DuplicateParent { child, first, second } => write!(
                f,
                "node {child} already has parent {first}; a second parent edge to {second} is \
                 rejected — a scene node has exactly one parent"
            ),
            Self::NotSpatial { node, type_name, role } => write!(
                f,
                "node {node} (`{type_name}`) is not a scene node and cannot be {role} — only \
                 node types carrying a Transform take part in the hierarchy"
            ),
            Self::ParentCycle { nodes } => {
                write!(f, "parent edges form a cycle through: ")?;
                for (i, node) in nodes.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{node}")?;
                }
                Ok(())
            }
            Self::SlotOutOfRange { node, slot, arity } => write!(
                f,
                "node {node}: Feeds slot {slot} is out of range — this node type declares \
                 {arity} slot(s)"
            ),
            Self::DuplicateSlot { target, slot, first, second } => write!(
                f,
                "node {target}: Feeds slot `{slot}` is already filled by node {first}; a second \
                 edge from node {second} is rejected — a slot takes exactly one input"
            ),
            Self::SlotTypeMismatch { target, slot, expected, source, produces } => write!(
                f,
                "node {target}: Feeds slot `{slot}` expects `{expected}`, but node {source} \
                 produces `{produces}`"
            ),
            Self::SourceProducesNothing { source, type_name, target, slot } => write!(
                f,
                "node {source} (`{type_name}`) produces nothing and cannot feed node {target}'s \
                 slot `{slot}`"
            ),
            Self::FeedsCycle { nodes } => {
                write!(f, "Feeds edges did not fully order — a cycle, or downstream of one: ")?;
                for (i, node) in nodes.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{node}")?;
                }
                Ok(())
            }
```

- [ ] **Step 6: Implement the structure pass**

Above the test module in `crates/sway-graph/src/structure.rs`:

```rust
//! The structure pass: `ParentEdge` and `FeedsEdge`. Design §4.
//!
//! Separate from the dataflow pass because structure edges are not param
//! dependencies and their failures need their own vocabulary — "cycle
//! detected" is unhelpful when the author filled one slot twice (parent
//! §2.5). `ParentEdge` enters no ordering at all; `FeedsEdge` produces
//! `cook_order`, which is a second topological sort over a different DAG.

use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::compile::CompileError;
use crate::edges::{EdgeFrom, EdgeTo, FeedsEdge, ParentEdge};
use crate::slots::SlotField;

/// What the structure pass needs to know about one node — its registry entry
/// flattened, so this module reads no resources.
pub(crate) struct StructureNode {
    pub entity: Entity,
    pub type_name: &'static str,
    pub slots: Vec<SlotField>,
    pub produces: core::any::TypeId,
    pub produces_path: &'static str,
    pub spatial: bool,
}

pub(crate) struct Structure {
    /// Node indices in `Feeds`-topological order.
    pub cook_order: Vec<usize>,
    /// Per node index, per slot ordinal: the source node's index.
    pub slots: Vec<Vec<Option<usize>>>,
    /// Per node index: the parent node's index.
    pub parents: Vec<Option<usize>>,
}

pub(crate) fn validate(
    world: &mut World,
    nodes: &[StructureNode],
    index_of: &HashMap<Entity, usize>,
) -> Result<Structure, CompileError> {
    let n = nodes.len();

    // --- Parent edges -------------------------------------------------
    struct RawParent {
        edge: Entity,
        child: Entity,
        parent: Entity,
    }
    let mut parent_query = world.query_filtered::<(Entity, &EdgeFrom, &EdgeTo), With<ParentEdge>>();
    let raw_parents: Vec<RawParent> = parent_query
        .iter(world)
        .map(|(edge, from, to)| RawParent {
            edge,
            child: from.0,
            parent: to.0,
        })
        .collect();

    let mut parents: Vec<Option<usize>> = vec![None; n];
    let mut parent_entity: Vec<Option<Entity>> = vec![None; n];
    for raw in raw_parents {
        let &child_idx = index_of
            .get(&raw.child)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.child })?;
        let &parent_idx = index_of
            .get(&raw.parent)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.parent })?;

        if !nodes[child_idx].spatial {
            return Err(CompileError::NotSpatial {
                node: raw.child,
                type_name: nodes[child_idx].type_name,
                role: "parented",
            });
        }
        if !nodes[parent_idx].spatial {
            return Err(CompileError::NotSpatial {
                node: raw.parent,
                type_name: nodes[parent_idx].type_name,
                role: "used as a parent",
            });
        }
        if let Some(first) = parent_entity[child_idx] {
            return Err(CompileError::DuplicateParent {
                child: raw.child,
                first,
                second: raw.parent,
            });
        }
        parent_entity[child_idx] = Some(raw.parent);
        parents[child_idx] = Some(parent_idx);
    }

    // Parenting acyclicity: walk each chain, bounded by n steps.
    for start in 0..n {
        let mut seen = 0usize;
        let mut cursor = parents[start];
        let mut chain = vec![nodes[start].entity];
        while let Some(idx) = cursor {
            if idx == start {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            chain.push(nodes[idx].entity);
            seen += 1;
            if seen > n {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            cursor = parents[idx];
        }
    }

    // --- Feeds edges ---------------------------------------------------
    struct RawFeeds {
        edge: Entity,
        slot: u16,
        source: Entity,
        target: Entity,
    }
    let mut feeds_query = world.query::<(Entity, &FeedsEdge, &EdgeFrom, &EdgeTo)>();
    let raw_feeds: Vec<RawFeeds> = feeds_query
        .iter(world)
        .map(|(edge, feeds, from, to)| RawFeeds {
            edge,
            slot: feeds.slot,
            source: from.0,
            target: to.0,
        })
        .collect();

    let mut slots: Vec<Vec<Option<usize>>> =
        nodes.iter().map(|node| vec![None; node.slots.len()]).collect();
    let mut slot_source_entity: Vec<Vec<Option<Entity>>> =
        nodes.iter().map(|node| vec![None; node.slots.len()]).collect();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree = vec![0u32; n];

    for raw in raw_feeds {
        let &source_idx = index_of
            .get(&raw.source)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.source })?;
        let &target_idx = index_of
            .get(&raw.target)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.target })?;

        let target = &nodes[target_idx];
        let slot = target.slots.get(raw.slot as usize).ok_or(CompileError::SlotOutOfRange {
            node: raw.target,
            slot: raw.slot,
            arity: target.slots.len(),
        })?;

        let source = &nodes[source_idx];
        if source.produces == core::any::TypeId::of::<()>() {
            return Err(CompileError::SourceProducesNothing {
                source: raw.source,
                type_name: source.type_name,
                target: raw.target,
                slot: slot.name,
            });
        }
        if source.produces != slot.capability {
            return Err(CompileError::SlotTypeMismatch {
                target: raw.target,
                slot: slot.name,
                expected: slot.capability_path,
                source: raw.source,
                produces: source.produces_path,
            });
        }
        if let Some(first) = slot_source_entity[target_idx][raw.slot as usize] {
            return Err(CompileError::DuplicateSlot {
                target: raw.target,
                slot: slot.name,
                first,
                second: raw.source,
            });
        }

        slot_source_entity[target_idx][raw.slot as usize] = Some(raw.source);
        slots[target_idx][raw.slot as usize] = Some(source_idx);
        adjacency[source_idx].push(target_idx);
        in_degree[target_idx] += 1;
    }

    for adj in &mut adjacency {
        adj.sort_unstable();
    }

    // Kahn's again, over the Feeds DAG. Seeded in node order so a tie the
    // edges leave unresolved still breaks deterministically.
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut cook_order: Vec<usize> = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while let Some(idx) = queue.pop_front() {
        cook_order.push(idx);
        placed[idx] = true;
        for &next in &adjacency[idx] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    if cook_order.len() != n {
        let remaining: Vec<Entity> =
            (0..n).filter(|&i| !placed[i]).map(|i| nodes[i].entity).collect();
        return Err(CompileError::FeedsCycle { nodes: remaining });
    }

    Ok(Structure {
        cook_order,
        slots,
        parents,
    })
}
```

Add `use bevy_ecs::query::With;` to the imports. Register the module in `crates/sway-graph/src/lib.rs` with `mod structure;` (private — `compile` is its only caller) and re-export the two new edge components from `edges`:

```rust
pub use edges::{
    EdgeFrom, EdgeTo, FeedsEdge, GraphNode, InEdges, NodeId, NodeRuntime, OutEdges, ParamEdge,
    ParentEdge, PortKind,
};
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS — the eight structure tests plus every pre-existing test. (The `cook_order` test needs Task 5; if `CompiledGraph` has no `cook_order` field yet, that one test fails to compile — implement Task 5 before re-running, or temporarily `#[ignore]` it and remove the attribute in Task 5.)

- [ ] **Step 8: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): ParentEdge, FeedsEdge, and the structure validation pass"
```

---

## Task 5: Applying `ChildOf`, slot tables, and `cook_order`

**Files:**
- Modify: `crates/sway-graph/src/compile.rs` (`NodePlan`, `CompiledGraph`, `compile`)
- Modify: `crates/sway-graph/src/edges.rs` (`NodeRuntime`)
- Modify: `crates/sway-graph/Cargo.toml` (add `bevy_transform`)

**Interfaces:**
- Consumes: `structure::validate`, `structure::StructureNode`, `structure::Structure` (Task 4); `slots::SlotSource` (Task 2).
- Produces: `NodePlan::slots: Vec<Option<SlotSource>>`; `CompiledGraph::cook_order: Vec<usize>` (plan indices); `NodeRuntime::{cook_dirty: bool, last_slot_ticks: Vec<Option<Tick>>}`.

- [ ] **Step 1: Write the failing test**

Add to `crates/sway-graph/src/compile.rs`'s `mod tests`:

```rust
    #[test]
    fn a_valid_hierarchy_is_applied_as_bevy_child_of() {
        use crate::test_nodes::{spawn_group, structure_app};
        use bevy_ecs::hierarchy::ChildOf;

        let mut app = structure_app();
        let child = spawn_group(app.world_mut());
        let root = spawn_group(app.world_mut());
        app.world_mut()
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(root)));

        compile(app.world_mut()).expect("compiles");

        assert_eq!(
            app.world().get::<ChildOf>(child).map(|c| c.0),
            Some(root),
            "compile applies the hierarchy"
        );
    }

    #[test]
    fn a_rejected_hierarchy_applies_nothing() {
        // Design §3: validation gates application, which is what M4's reload
        // needs — a bad edit must leave the previous graph in force rather
        // than half-applying itself.
        use crate::test_nodes::{spawn_group, spawn_probe, structure_app};
        use bevy_ecs::hierarchy::ChildOf;

        let mut app = structure_app();
        let good_child = spawn_group(app.world_mut());
        let root = spawn_group(app.world_mut());
        let bad_child = spawn_probe(app.world_mut()); // not SPATIAL
        app.world_mut()
            .spawn((ParentEdge, EdgeFrom(good_child), EdgeTo(root)));
        app.world_mut()
            .spawn((ParentEdge, EdgeFrom(bad_child), EdgeTo(root)));

        assert!(compile(app.world_mut()).is_err());

        assert!(
            app.world().get::<ChildOf>(good_child).is_none(),
            "a failed structure pass must not apply the edges that were legal"
        );
    }

    #[test]
    fn reparenting_removes_the_previous_child_of() {
        use crate::test_nodes::{spawn_group, structure_app};
        use bevy_ecs::hierarchy::ChildOf;

        let mut app = structure_app();
        let child = spawn_group(app.world_mut());
        let first = spawn_group(app.world_mut());
        let second = spawn_group(app.world_mut());
        let edge = app
            .world_mut()
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(first)))
            .id();
        compile(app.world_mut()).expect("compiles");

        app.world_mut().despawn(edge);
        app.world_mut()
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(second)));
        compile(app.world_mut()).expect("recompiles");

        assert_eq!(app.world().get::<ChildOf>(child).map(|c| c.0), Some(second));
    }

    #[test]
    fn unparenting_removes_child_of_entirely() {
        use crate::test_nodes::{spawn_group, structure_app};
        use bevy_ecs::hierarchy::ChildOf;

        let mut app = structure_app();
        let child = spawn_group(app.world_mut());
        let root = spawn_group(app.world_mut());
        let edge = app
            .world_mut()
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(root)))
            .id();
        compile(app.world_mut()).expect("compiles");

        app.world_mut().despawn(edge);
        compile(app.world_mut()).expect("recompiles");

        assert!(app.world().get::<ChildOf>(child).is_none());
    }

    #[test]
    fn an_applied_hierarchy_propagates_global_transforms() {
        // The point of compiling to Bevy's own hierarchy rather than to
        // something of ours: propagation is free (parent §2.10). Assert it
        // actually happens rather than assuming the component alone suffices.
        use crate::test_nodes::{spawn_group, structure_app};
        use bevy_transform::TransformPlugin;
        use bevy_transform::prelude::{GlobalTransform, Transform};

        let mut app = structure_app();
        app.add_plugins(TransformPlugin);
        let child = spawn_group(app.world_mut());
        let root = spawn_group(app.world_mut());
        app.world_mut()
            .spawn((ParentEdge, EdgeFrom(child), EdgeTo(root)));
        compile(app.world_mut()).expect("compiles");

        app.world_mut()
            .entity_mut(root)
            .insert(Transform::from_xyz(10.0, 0.0, 0.0));
        app.world_mut()
            .entity_mut(child)
            .insert(Transform::from_xyz(0.0, 5.0, 0.0));
        app.update();

        let global = app
            .world()
            .get::<GlobalTransform>(child)
            .expect("propagation inserts GlobalTransform")
            .translation();
        assert_eq!(global, bevy_transform::prelude::Transform::from_xyz(10.0, 5.0, 0.0).translation);
    }

    #[test]
    fn a_plan_carries_its_slot_sources() {
        use crate::test_nodes::{spawn_sinkgeo, spawn_source, structure_app};

        let mut app = structure_app();
        let src = spawn_source(app.world_mut());
        let sink = spawn_sinkgeo(app.world_mut());
        app.world_mut()
            .spawn((FeedsEdge { slot: 0 }, EdgeFrom(src), EdgeTo(sink)));

        let compiled = compile(app.world_mut()).expect("compiles");
        let plan = compiled.plans.iter().find(|p| p.entity == sink).unwrap();
        assert_eq!(plan.slots.len(), 1);
        assert_eq!(plan.slots[0].as_ref().map(|s| s.entity), Some(src));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph compile`
Expected: FAIL — `NodePlan` has no `slots`, `CompiledGraph` has no `cook_order`.

- [ ] **Step 3: Add `bevy_transform` to the manifest**

In `crates/sway-graph/Cargo.toml`, add `bevy_transform.workspace = true` and update the comment to say `bevy_asset` remains the only deferred one (it joins at M4). Add `bevy_transform = "=0.19.0"` to `[workspace.dependencies]` in the root `Cargo.toml`.

- [ ] **Step 4: Extend `NodeRuntime`**

In `crates/sway-graph/src/edges.rs`:

```rust
#[derive(Component, Default)]
pub struct NodeRuntime {
    pub continuous_base: usize,
    pub event_base: usize,
    /// The `Params` change tick this node last prefilled against. `None`
    /// forces a prefill, which is how a recompile makes a disconnect take
    /// effect.
    pub last_params_tick: Option<Tick>,
    /// The cook gate (design §6). Sticky: set when a driven input changes,
    /// when prefill fires, or when an upstream product's change tick moves;
    /// cleared only by a cook that actually ran. Stickiness is what makes it
    /// survive a skipped cadence, which a `Changed<T>` filter cannot.
    pub cook_dirty: bool,
    /// Per slot ordinal: the source's `produced_change_tick` at this node's
    /// last cook.
    pub last_slot_ticks: Vec<Option<Tick>>,
}
```

- [ ] **Step 5: Extend `NodePlan` and `CompiledGraph`**

In `crates/sway-graph/src/compile.rs`, import `SlotSource` from `crate::slots` (Task 2 defines it there so `SlotView` can name it without depending on compilation) and add to `NodePlan`:

```rust
    /// Per slot ordinal: the resolved source, or `None` if the slot is empty.
    pub slots: Vec<Option<SlotSource>>,
```

Add to `CompiledGraph`:

```rust
    /// Plan indices in `Feeds` order — the second of the tick's two orders
    /// (design §7). Distinct from `plans`' own param order, and `ParentEdge`
    /// enters neither.
    pub cook_order: Vec<usize>,
```

- [ ] **Step 6: Wire the structure pass into `compile`**

After pass 2 (base allocation) and before pass 3 (edge validation) in `compile`, build the structure nodes and validate:

```rust
    // --- Pass 2b: structure (design §4) ---------------------------------
    //
    // Before the dataflow pass, and separate from it: structure edges are not
    // param dependencies, and their failures need their own vocabulary
    // (parent §2.5).
    let structure_nodes: Vec<crate::structure::StructureNode> = {
        let registry = world.resource::<NodeTypeRegistry>();
        nodes
            .iter()
            .map(|node| {
                let entry = registry
                    .get(node.node_type)
                    .expect("node type resolved in pass 1");
                crate::structure::StructureNode {
                    entity: node.entity,
                    type_name: entry.name,
                    slots: entry.slots.clone(),
                    produces: entry.produces,
                    produces_path: entry.produces_path,
                    spatial: entry.spatial,
                }
            })
            .collect()
    };
    let structure = crate::structure::validate(world, &structure_nodes, &index_of)?;
```

In pass 5, give each plan its slots — mapping node indices to plan indices requires `topo_rank`, which pass 4 already computes:

```rust
        let slots: Vec<Option<SlotSource>> = structure.slots[idx]
            .iter()
            .map(|source| {
                source.map(|source_idx| SlotSource {
                    entity: nodes[source_idx].entity,
                    plan_index: topo_rank[source_idx],
                })
            })
            .collect();
```

and add `slots,` to the `NodePlan` construction.

In pass 6, apply the hierarchy and size the gate's state. This runs last, after every validation has passed, which is what makes "a rejected hierarchy applies nothing" true:

```rust
    // --- Pass 6: apply structure, write NodeRuntime -----------------------
    for (idx, node) in nodes.iter().enumerate() {
        match structure.parents[idx] {
            Some(parent_idx) => {
                let parent = nodes[parent_idx].entity;
                world
                    .entity_mut(node.entity)
                    .insert(bevy_ecs::hierarchy::ChildOf(parent));
            }
            None => {
                world.entity_mut(node.entity).remove::<bevy_ecs::hierarchy::ChildOf>();
            }
        }
        world.entity_mut(node.entity).insert(NodeRuntime {
            continuous_base: node.continuous_base,
            event_base: node.event_base,
            last_params_tick: None,
            // Compilation dirties every node, so each cooks once after a load
            // (design §6).
            cook_dirty: true,
            last_slot_ticks: vec![None; structure.slots[idx].len()],
        });
    }
```

Finally map `cook_order` from node indices to plan indices in the returned `CompiledGraph`:

```rust
        cook_order: structure.cook_order.iter().map(|&i| topo_rank[i]).collect(),
```

**Note on `ChildOf`'s import path:** it is `bevy_ecs::hierarchy::ChildOf` in Bevy 0.19. If the path differs, `bevy_transform::prelude` re-exports it — do not reach for the `bevy` facade, which the Global Constraints forbid in this crate.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, including Task 4's `a_feeds_chain_orders_producer_before_consumer` (remove any `#[ignore]` added there).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/sway-graph
git commit -m "feat(graph): apply ChildOf, resolve slot tables, emit cook_order"
```

---

## Task 6: The sticky dirty flag

**Files:**
- Modify: `crates/sway-graph/src/tick.rs` (`graph_tick`'s gather and prefill)

**Interfaces:**
- Consumes: `NodeRuntime::cook_dirty` (Task 5).
- Produces: no new API — `cook_dirty` is set as a side effect of gather and prefill, and read by Task 7.

- [ ] **Step 1: Write the failing test**

Add to `crates/sway-graph/src/tick.rs`'s `mod tests`:

```rust
    #[test]
    fn a_changed_driven_input_dirties_the_node() {
        // The case that fails if the gate reads Params change ticks: a
        // connected port shadows the authored value, so Params never moves
        // while the effective parameter changes every tick (design §6).
        use crate::test_nodes::{Gain, spawn_gain};

        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 2.0, 0.0);
        let dst = spawn_gain(app.world_mut(), 1.0, 0.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();
        // Clear the compile-time dirty so the next assertion is about gather.
        app.world_mut().get_mut::<NodeRuntime>(dst).unwrap().cook_dirty = false;

        app.world_mut().get_mut::<GainParams>(src).unwrap().gain = 5.0;
        app.update();

        assert!(
            app.world().get::<NodeRuntime>(dst).unwrap().cook_dirty,
            "a driven input that changed must dirty its node"
        );
    }

    #[test]
    fn a_steady_driven_input_does_not_dirty_the_node() {
        use crate::test_nodes::{Gain, spawn_gain};

        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 2.0, 0.0);
        let dst = spawn_gain(app.world_mut(), 1.0, 0.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();
        app.world_mut().get_mut::<NodeRuntime>(dst).unwrap().cook_dirty = false;

        for _ in 0..5 {
            app.update();
        }

        assert!(
            !app.world().get::<NodeRuntime>(dst).unwrap().cook_dirty,
            "an unchanged value must not dirty its node every tick"
        );
    }

    #[test]
    fn an_authored_param_edit_dirties_the_node() {
        use crate::test_nodes::{spawn_gain};

        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 0.0);
        recompile(&mut app);
        app.update();
        app.world_mut().get_mut::<NodeRuntime>(a).unwrap().cook_dirty = false;

        app.world_mut().get_mut::<GainParams>(a).unwrap().gain = 3.0;
        app.update();

        assert!(app.world().get::<NodeRuntime>(a).unwrap().cook_dirty);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph tick`
Expected: FAIL — `cook_dirty` is never set after compilation.

- [ ] **Step 3: Set the flag from gather and prefill**

In `graph_tick`'s per-node loop in `crates/sway-graph/src/tick.rs`, replace the continuous gather and the prefill block with:

```rust
            // `dirty` accumulates this tick's reasons to cook; it is OR-ed
            // into the sticky flag below rather than assigned, so a reason
            // raised on an earlier tick is not lost (design §6).
            let mut dirty = false;

            for &(src, dst) in &plan.continuous_copies {
                let incoming = clone_slot(&*arena.continuous[src]);
                // `reflect_partial_eq` returns None for values that cannot be
                // compared — including the `()` a freshly-resized arena slot
                // holds — and None must mean "changed", never "unchanged".
                let changed = arena.continuous[dst]
                    .reflect_partial_eq(&*incoming)
                    .map(|equal| !equal)
                    .unwrap_or(true);
                arena.continuous[dst] = incoming;
                dirty |= changed;
            }
```

(the event merge loop and the offset sort are unchanged), then:

```rust
            let current = params_changed_tick_fn(world, plan.entity);
            let last = world
                .get::<NodeRuntime>(plan.entity)
                .and_then(|r| r.last_params_tick);
            if last != current {
                prefill_fn(world, plan.entity, &mut arena, plan);
                dirty = true;
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_params_tick = current;
                }
            }

            // Only touch NodeRuntime when there is something to record —
            // an unconditional `get_mut` would churn its change tick every
            // tick for every node.
            if dirty && let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = true;
            }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, three new tests plus every pre-existing one.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/tick.rs
git commit -m "feat(graph): sticky cook-dirty flag from gather and prefill"
```

---

## Task 7: `SlotView` and the cook pass

**Files:**
- Modify: `crates/sway-graph/src/view.rs` (`SlotView`)
- Modify: `crates/sway-graph/src/tick.rs` (`graph_tick`'s second pass)
- Modify: `crates/sway-graph/src/lib.rs` (re-export)

**Interfaces:**
- Consumes: `SlotView` (Task 3); `CompiledGraph::cook_order`, `NodePlan::slots` (Task 5); `NodeTypeEntry::{cook, produced_change_tick}` (Task 3); `NodeRuntime::{cook_dirty, last_slot_ticks}` (Task 5).
- Produces: no new API — the gate is internal to `graph_tick`.

- [ ] **Step 1: Write the failing test**

Add to `crates/sway-graph/src/tick.rs`'s `mod tests`:

```rust
    mod cooking {
        use super::*;
        use crate::edges::{EdgeFrom, EdgeTo, FeedsEdge};
        use crate::test_nodes::{
            BlobData, CookCounter, SinkGeoParams, SourceParams, spawn_sinkgeo, spawn_source,
            structure_app,
        };

        fn cooks(app: &App) -> u32 {
            app.world().resource::<CookCounter>().0
        }

        fn chain(app: &mut App) -> (Entity, Entity) {
            let src = spawn_source(app.world_mut());
            let sink = spawn_sinkgeo(app.world_mut());
            app.world_mut()
                .spawn((FeedsEdge { slot: 0 }, EdgeFrom(src), EdgeTo(sink)));
            recompile(app);
            (src, sink)
        }

        #[test]
        fn every_node_cooks_exactly_once_after_compilation() {
            let mut app = structure_app();
            let (_src, sink) = chain(&mut app);

            app.update();

            assert_eq!(cooks(&app), 2, "one cook each");
            assert!(app.world().get::<BlobData>(sink).is_some());
        }

        #[test]
        fn a_steady_graph_cooks_nothing_after_the_first_tick() {
            // The negative assertion §10 asks for, on a counter rather than
            // on an output that merely happens to be unchanged.
            let mut app = structure_app();
            let _ = chain(&mut app);
            app.update();
            let after_first = cooks(&app);

            for _ in 0..10 {
                app.update();
            }

            assert_eq!(cooks(&app), after_first, "an idle graph must not cook");
        }

        #[test]
        fn an_upstream_cook_propagates_to_its_feeds_consumer_in_the_same_tick() {
            let mut app = structure_app();
            let (src, sink) = chain(&mut app);
            app.update();
            let baseline = cooks(&app);

            app.world_mut().get_mut::<SourceParams>(src).unwrap().seed = 7.0;
            app.update();

            assert_eq!(cooks(&app), baseline + 2, "both ends re-cook");
            assert_eq!(app.world().get::<BlobData>(sink), Some(&BlobData(7)));
        }

        #[test]
        fn a_param_change_on_one_node_does_not_cook_its_upstream() {
            // Dirt flows with Feeds direction only. A downstream param edit
            // must not re-cook the operator above it.
            let mut app = structure_app();
            let (_src, sink) = chain(&mut app);
            app.update();
            let baseline = cooks(&app);

            app.world_mut().get_mut::<SinkGeoParams>(sink).unwrap().scale = 2.0;
            app.update();

            assert_eq!(cooks(&app), baseline + 1, "only the edited node cooks");
        }

        #[test]
        fn a_node_added_after_an_upstream_cook_still_cooks_against_it() {
            // §2.11's robustness case: the gate must survive a node joining
            // mid-session, which a `Changed<T>` filter would not.
            let mut app = structure_app();
            let src = spawn_source(app.world_mut());
            recompile(&mut app);
            app.update();
            for _ in 0..5 {
                app.update();
            }
            let baseline = cooks(&app);

            let sink = spawn_sinkgeo(app.world_mut());
            app.world_mut()
                .spawn((FeedsEdge { slot: 0 }, EdgeFrom(src), EdgeTo(sink)));
            recompile(&mut app);
            app.update();

            assert!(cooks(&app) > baseline, "the new node must cook");
            assert!(app.world().get::<BlobData>(sink).is_some());
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-graph cooking`
Expected: FAIL — `SlotView` does not exist and nothing calls a cook fn.

- [ ] **Step 3: Add the cook pass to `graph_tick`**

The registry snapshot at the top of `graph_tick` currently copies four fn pointers per plan; extend the tuple to six by adding `entry.cook` and `entry.produced_change_tick`, and widen its type annotation to
`Vec<(TickFn, PrefillFn, SeedOutputsFn, TickOfFn, Option<CookFn>, ProducedTickFn)>`.

Then, still **inside** the `resource_scope` closure and after the per-node tick loop ends, add:

```rust
        // --- Pass 2: cooks, in Feeds order (design §7) --------------------
        //
        // Ticks precede cooks globally, so a cook always sees its own node's
        // effective params already applied — parent §2.11's step B before its
        // step C. Inside the resource_scope, so the arena is provably out of
        // the world here too: a cook has no business touching ports.
        for &plan_idx in &compiled.cook_order {
            let Some(cook_fn) = entries[plan_idx].4 else {
                continue;
            };
            let plan = &compiled.plans[plan_idx];

            // Stored ticks, kept for the geometry side only — a product is
            // large and not usefully value-compared (design §6). A source
            // whose `produced_change_tick` is None never dirties its
            // consumers, which is exactly right for a material handle.
            let current: Vec<Option<Tick>> = plan
                .slots
                .iter()
                .map(|slot| {
                    slot.and_then(|source| (entries[source.plan_index].5)(world, source.entity))
                })
                .collect();

            let dirty = match world.get::<NodeRuntime>(plan.entity) {
                Some(rt) => rt.cook_dirty || rt.last_slot_ticks != current,
                None => false,
            };
            if !dirty {
                continue;
            }

            let view = SlotView::new(&plan.slots);
            cook_fn(world, plan.entity, &view);

            if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = false;
                rt.last_slot_ticks = current;
            }
        }
```

Add `use bevy_ecs::change_detection::Tick;`, `use crate::registry::{CookFn, ProducedTickFn}` and `use crate::view::SlotView;` to `tick.rs`'s imports.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, five new cooking tests plus every pre-existing one.

- [ ] **Step 5: Run the scoped clippy gate**

Run: `cargo clippy -p sway-graph -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): SlotView and the gated cook pass"
```

---

## Task 8: `Grid` and `Displace`

**Files:**
- Create: `crates/sway-geo/src/grid.rs`
- Create: `crates/sway-geo/src/displace.rs`
- Modify: `crates/sway-geo/src/lib.rs` (modules, re-exports, `GeoNodesPlugin`)

**Interfaces:**
- Consumes: `Geometry`, `Attribute` (Task 1); `NodeType`, `Slot`, `NoSlots`, `NoOutputs`, `register_slot`, `SlotView` (Tasks 2–7).
- Produces: `Grid` with `Grid::{ROWS, COLS, WIDTH, HEIGHT}`, `GridParams { rows: u32, cols: u32, width: f32, height: f32 }`; `Displace` with `Displace::{AMOUNT, FREQUENCY, IN_GEO}`, `DisplaceParams { amount: f32, frequency: f32 }`; `GeoNodesPlugin`.

- [ ] **Step 1: Write the failing test**

`crates/sway-geo/src/grid.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sway_graph::{PortArena, PortView, TickCtx};

    fn cooked_grid(rows: u32, cols: u32) -> Geometry {
        let mut world = World::new();
        let node = world
            .spawn((
                GridParams {
                    rows,
                    cols,
                    width: 2.0,
                    height: 2.0,
                },
                GridState,
            ))
            .id();
        Grid::cook(&mut world, node, &SlotView::new(&[]));
        world.get::<Geometry>(node).cloned().expect("Grid cooks a Geometry")
    }

    #[test]
    fn a_grid_has_rows_times_cols_points() {
        let g = cooked_grid(3, 4);
        assert_eq!(g.point_count(), 12);
        assert_eq!(g.get("P").map(|a| a.len()), Some(12));
        assert_eq!(g.get("N").map(|a| a.len()), Some(12));
        assert_eq!(g.get("uv").map(|a| a.len()), Some(12));
    }

    #[test]
    fn a_grid_spans_its_width_and_height_centred_on_the_origin() {
        let g = cooked_grid(2, 2);
        let p = g.get("P").and_then(|a| a.as_vec3()).expect("P is Vec3");
        assert_eq!(p[0], Vec3::new(-1.0, 0.0, -1.0));
        assert_eq!(p[3], Vec3::new(1.0, 0.0, 1.0));
    }

    #[test]
    fn a_grid_emits_two_triangles_per_cell() {
        let g = cooked_grid(3, 3);
        // 2x2 cells, 2 triangles each, 3 indices each.
        assert_eq!(g.indices().map(|i| i.len()), Some(24));
    }

    #[test]
    fn a_cook_is_a_pure_function_of_its_params() {
        assert_eq!(
            cooked_grid(3, 3).get("P").and_then(|a| a.as_vec3()).cloned(),
            cooked_grid(3, 3).get("P").and_then(|a| a.as_vec3()).cloned()
        );
    }
}
```

`crates/sway-geo/src/displace.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Grid, GridParams, GridState};

    fn chain(amount: f32) -> (Geometry, Geometry) {
        let mut world = World::new();
        let src = world
            .spawn((
                GridParams { rows: 3, cols: 3, width: 2.0, height: 2.0 },
                GridState,
            ))
            .id();
        Grid::cook(&mut world, src, &SlotView::new(&[]));

        let node = world
            .spawn((
                DisplaceParams { amount, frequency: 1.0 },
                DisplaceState,
            ))
            .id();
        let slots = [Some(sway_graph::SlotSource { entity: src, plan_index: 0 })];
        Displace::cook(&mut world, node, &SlotView::new(&slots));

        (
            world.get::<Geometry>(src).cloned().unwrap(),
            world.get::<Geometry>(node).cloned().unwrap(),
        )
    }

    #[test]
    fn an_untouched_attribute_is_shared_not_copied() {
        // Design §5: the refcount-bump claim, asserted rather than described.
        let (src, out) = chain(0.5);
        let (Some(Attribute::Vec3(a)), Some(Attribute::Vec3(b))) = (src.get("N"), out.get("N"))
        else {
            panic!("N must be a Vec3 attribute on both");
        };
        assert!(Arc::ptr_eq(a, b), "N passed through must not be copied");
    }

    #[test]
    fn positions_are_a_new_buffer() {
        let (src, out) = chain(0.5);
        let (Some(Attribute::Vec3(a)), Some(Attribute::Vec3(b))) = (src.get("P"), out.get("P"))
        else {
            panic!("P must be a Vec3 attribute on both");
        };
        assert!(!Arc::ptr_eq(a, b), "P was rewritten, so it must be its own buffer");
    }

    #[test]
    fn zero_amount_leaves_positions_unmoved() {
        let (src, out) = chain(0.0);
        assert_eq!(
            src.get("P").and_then(|a| a.as_vec3()).cloned(),
            out.get("P").and_then(|a| a.as_vec3()).cloned()
        );
    }

    #[test]
    fn displacement_follows_the_normal() {
        let (src, out) = chain(1.0);
        let before = src.get("P").and_then(|a| a.as_vec3()).unwrap();
        let after = out.get("P").and_then(|a| a.as_vec3()).unwrap();
        let moved = before.iter().zip(after.iter()).any(|(b, a)| b != a);
        assert!(moved, "a non-zero amount must move at least one point");
        for (b, a) in before.iter().zip(after.iter()) {
            assert_eq!(a.x, b.x, "displacement is along N (+Y), not in-plane");
            assert_eq!(a.z, b.z);
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sway-geo`
Expected: FAIL — `Grid`, `Displace` do not exist.

- [ ] **Step 3: Implement `Grid`**

`crates/sway-geo/src/grid.rs`:

```rust
//! `Grid` — a CPU geometry source. Design §8.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use bevy_reflect::Reflect;
use sway_graph::{NoOutputs, NoSlots, NodeType, PortView, SlotView, TickCtx};

use crate::geometry::{Attribute, Geometry};

#[derive(Reflect, Component)]
pub struct GridParams {
    pub rows: u32,
    pub cols: u32,
    pub width: f32,
    pub height: f32,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            rows: 16,
            cols: 16,
            width: 4.0,
            height: 4.0,
        }
    }
}

#[derive(Component, Default)]
pub struct GridState;

pub struct Grid;

impl Grid {
    pub const ROWS: u16 = 0;
    pub const COLS: u16 = 1;
    pub const WIDTH: u16 = 2;
    pub const HEIGHT: u16 = 3;
}

impl NodeType for Grid {
    type Params = GridParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = Geometry;
    type State = GridState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("rows", Self::ROWS),
        ("cols", Self::COLS),
        ("width", Self::WIDTH),
        ("height", Self::HEIGHT),
    ];
    const COOKS: bool = true;

    fn register(_app: &mut App) {}

    /// Nothing per-tick: `Grid`'s whole product is its cook.
    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _slots: &SlotView) {
        let params = match world.get::<GridParams>(node) {
            Some(p) => (p.rows.max(2), p.cols.max(2), p.width, p.height),
            None => return,
        };
        let (rows, cols, width, height) = params;

        let count = (rows * cols) as usize;
        let mut positions = Vec::with_capacity(count);
        let mut normals = Vec::with_capacity(count);
        let mut uvs = Vec::with_capacity(count);
        for r in 0..rows {
            for c in 0..cols {
                let u = c as f32 / (cols - 1) as f32;
                let v = r as f32 / (rows - 1) as f32;
                positions.push(Vec3::new(
                    (u - 0.5) * width,
                    0.0,
                    (v - 0.5) * height,
                ));
                normals.push(Vec3::Y);
                uvs.push(Vec2::new(u, v));
            }
        }

        let mut indices = Vec::with_capacity(((rows - 1) * (cols - 1) * 6) as usize);
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let i = r * cols + c;
                indices.extend_from_slice(&[i, i + cols, i + 1]);
                indices.extend_from_slice(&[i + 1, i + cols, i + cols + 1]);
            }
        }

        let mut geo = Geometry::new(count);
        geo.set("P", Attribute::Vec3(Arc::new(positions)));
        geo.set("N", Attribute::Vec3(Arc::new(normals)));
        geo.set("uv", Attribute::Vec2(Arc::new(uvs)));
        geo.set_indices(Some(Arc::new(indices)));
        world.entity_mut(node).insert(geo);
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<Geometry>()
            .map(|t| t.changed)
    }
}
```

- [ ] **Step 4: Implement `Displace`**

`crates/sway-geo/src/displace.rs`:

```rust
//! `Displace` — element-wise displacement along `N`. Design §8.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::Vec3;
use bevy_reflect::Reflect;
use sway_graph::{
    NoOutputs, NodeType, PortView, Slot, SlotView, TickCtx, register_slot,
};

use crate::geometry::{Attribute, Geometry};

#[derive(Reflect, Default)]
pub struct DisplaceSlots {
    pub geo: Slot<Geometry>,
}

#[derive(Reflect, Component, Default)]
pub struct DisplaceParams {
    pub amount: f32,
    pub frequency: f32,
}

#[derive(Component, Default)]
pub struct DisplaceState;

pub struct Displace;

impl Displace {
    pub const AMOUNT: u16 = 0;
    pub const FREQUENCY: u16 = 1;
    pub const IN_GEO: u16 = 0;
}

impl NodeType for Displace {
    type Params = DisplaceParams;
    type Outputs = NoOutputs;
    type Slots = DisplaceSlots;
    type Produces = Geometry;
    type State = DisplaceState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] =
        &[("amount", Self::AMOUNT), ("frequency", Self::FREQUENCY)];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] = &[("geo", Self::IN_GEO)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_slot::<Geometry>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        let Some(source) = slots.source(Self::IN_GEO) else {
            return;
        };
        // Reads and writes touch different entities, so read through the
        // world, compute into a local, then insert into self (parent §2.11).
        let Some(input) = world.get::<Geometry>(source).cloned() else {
            return;
        };
        let (amount, frequency) = world
            .get::<DisplaceParams>(node)
            .map(|p| (p.amount, p.frequency))
            .unwrap_or((0.0, 1.0));

        let Some(positions) = input.get("P").and_then(|a| a.as_vec3()) else {
            return;
        };
        let normals = input.get("N").and_then(|a| a.as_vec3());

        let displaced: Vec<Vec3> = positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let n = normals.map(|n| n[i]).unwrap_or(Vec3::Y);
                let f = (p.x * frequency).sin() * (p.z * frequency).sin();
                *p + n * (amount * f)
            })
            .collect();

        // Cloning the input carries every other attribute through as a
        // refcount bump; only `P` becomes a new buffer (design §5).
        let mut out = input;
        out.set("P", Attribute::Vec3(Arc::new(displaced)));
        world.entity_mut(node).insert(out);
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<Geometry>()
            .map(|t| t.changed)
    }
}
```

- [ ] **Step 5: Add the plugin and re-exports**

Extend `crates/sway-geo/src/lib.rs`:

```rust
pub mod displace;
pub mod geometry;
pub mod grid;

pub use displace::{Displace, DisplaceParams, DisplaceState};
pub use geometry::{Attribute, Geometry};
pub use grid::{Grid, GridParams, GridState};

use bevy_app::{App, Plugin};

/// Registers the CPU geometry operators.
pub struct GeoNodesPlugin;

impl Plugin for GeoNodesPlugin {
    fn build(&self, app: &mut App) {
        sway_graph::register_node_type::<Grid>(app);
        sway_graph::register_node_type::<Displace>(app);
    }
}
```

`SlotSource` is already re-exported from `sway-graph`'s `lib.rs` (Task 2), which is what lets the displace test construct one.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-geo`
Expected: PASS, 8 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-geo crates/sway-graph/src/lib.rs
git commit -m "feat(geo): Grid and Displace operators"
```

---

## Task 9: `Group`, `Rgb`, and the material node

**Files:**
- Create: `crates/sway-nodes/src/scene.rs` (`Group`, `Rgb`)
- Create: `crates/sway-nodes/src/material.rs` (`StandardMaterialNode`, `MaterialOf<M>`)
- Modify: `crates/sway-nodes/Cargo.toml`, `crates/sway-nodes/src/lib.rs`

**Interfaces:**
- Consumes: `NodeType` and friends.
- Produces: `Group` with `Group::{TRANSLATION, ROTATION_X, ROTATION_Y, ROTATION_Z, SCALE}`, `GroupParams { translation: Vec3, rotation_x: f32, rotation_y: f32, rotation_z: f32, scale: Vec3 }`; `Rgb` with `Rgb::{R, G, B, OUT_COLOR}`, `RgbParams`, `RgbOutputs { color: Color }`; `MaterialOf<M>`; `StandardMaterialNode` with `::{BASE_COLOR, EMISSIVE, METALLIC, PERCEPTUAL_ROUGHNESS}`, `MaterialState { handle: Option<Handle<StandardMaterial>> }`; `SceneNodesPlugin`.

- [ ] **Step 1: Add the renderer-side dependency**

In `crates/sway-nodes/Cargo.toml` add `bevy.workspace = true` and replace the top comment with:

```toml
# Scene nodes need Transform, meshes, materials and Color, so this crate
# depends on the bevy facade from M2b onward. Parent §2.9's no-renderer rule
# constrains sway-graph, not the node crates (design §2). Signal-node tests
# still build a MinimalPlugins app — this costs compile time, not
# testability.
```

- [ ] **Step 2: Write the failing test**

`crates/sway-nodes/src/material.rs`, test module — these are `apply_level`'s three tests from `crates/sway-app/src/scene.rs:143-186`, moved to where the `get`/compare/`get_mut` rule now lives (§9):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use sway_graph::{PortArena, PortView, TickCtx};

    fn app_with_material() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<StandardMaterial>();
        let node = app
            .world_mut()
            .spawn((StandardMaterialParams::default(), MaterialState::default()))
            .id();
        (app, node)
    }

    /// Runs the node's tick with an arena holding the given base colour.
    fn tick_with(app: &mut App, node: Entity, colour: Color) {
        let mut arena = PortArena::new(4, 0);
        arena.continuous[StandardMaterialNode::BASE_COLOR as usize] = Box::new(colour);
        arena.continuous[StandardMaterialNode::EMISSIVE as usize] = Box::new(Color::BLACK);
        arena.continuous[StandardMaterialNode::METALLIC as usize] = Box::new(0.0_f32);
        arena.continuous[StandardMaterialNode::PERCEPTUAL_ROUGHNESS as usize] = Box::new(0.5_f32);
        let world = app.world_mut();
        let mut view = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 4]);
        StandardMaterialNode::tick(
            world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );
    }

    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<StandardMaterial>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn the_node_creates_and_drives_its_own_material() {
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));

        let handle = app
            .world()
            .get::<MaterialState>(node)
            .and_then(|s| s.handle.clone())
            .expect("the node owns a handle");
        let colour = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&handle)
            .unwrap()
            .base_color;
        assert_eq!(colour, Color::srgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn a_changed_colour_modifies_the_asset() {
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));
        let _ = count_modified(&mut app);

        tick_with(&mut app, node, Color::srgb(0.0, 1.0, 0.0));

        assert!(count_modified(&mut app) > 0, "a real change must write through");
    }

    #[test]
    fn an_unchanged_colour_does_not_touch_the_asset() {
        // Parent §2.11: `Assets::get_mut` marks the asset changed by the act
        // of being called, so an unconditional write re-uploads a material
        // that nothing moved.
        let (mut app, node) = app_with_material();
        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));
        let _ = count_modified(&mut app);

        tick_with(&mut app, node, Color::srgb(1.0, 0.0, 0.0));

        assert_eq!(count_modified(&mut app), 0, "an unchanged colour must not rewrite");
    }
}
```

`crates/sway-nodes/src/scene.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use sway_graph::{PortArena, PortView, TickCtx};

    /// Fills a five-slot arena with a Group's ports.
    fn group_arena(translation: Vec3) -> PortArena {
        let mut arena = PortArena::new(5, 0);
        arena.continuous[Group::TRANSLATION as usize] = Box::new(translation);
        arena.continuous[Group::ROTATION_X as usize] = Box::new(0.0_f32);
        arena.continuous[Group::ROTATION_Y as usize] = Box::new(0.0_f32);
        arena.continuous[Group::ROTATION_Z as usize] = Box::new(0.0_f32);
        arena.continuous[Group::SCALE as usize] = Box::new(Vec3::ONE);
        arena
    }

    #[test]
    fn a_group_writes_its_transform() {
        let mut world = World::new();
        let node = world.spawn((GroupParams::default(), GroupState)).id();
        let mut arena = group_arena(Vec3::new(1.0, 2.0, 3.0));
        let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);

        Group::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            world.get::<Transform>(node).map(|t| t.translation),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn an_unchanged_transform_is_not_rewritten() {
        // set_if_neq, per parent §2.11: an unconditional assignment sets the
        // change tick every tick, re-running propagation and making
        // `Changed<Transform>` worthless downstream.
        let mut world = World::new();
        let node = world.spawn((GroupParams::default(), GroupState)).id();
        let mut arena = group_arena(Vec3::ZERO);

        for _ in 0..2 {
            let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);
            Group::tick(
                &mut world,
                node,
                &mut view,
                &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
            );
        }
        let first = world.get_ref::<Transform>(node).unwrap().last_changed();

        let mut view = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 5]);
        Group::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            world.get_ref::<Transform>(node).unwrap().last_changed(),
            first,
            "an unchanged Transform must not be re-marked"
        );
    }

    #[test]
    fn rgb_writes_a_color_to_its_output_port() {
        // The first struct-typed value across a continuous edge (design §8).
        let mut world = World::new();
        let node = world.spawn((RgbParams::default(), RgbState)).id();
        let mut arena = PortArena::new(4, 0);
        arena.continuous[Rgb::R as usize] = Box::new(1.0_f32);
        arena.continuous[Rgb::G as usize] = Box::new(0.5_f32);
        arena.continuous[Rgb::B as usize] = Box::new(0.0_f32);
        arena.continuous[Rgb::OUT_COLOR as usize] = Box::new(Color::BLACK);
        let mut view = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 3]);

        Rgb::tick(
            &mut world,
            node,
            &mut view,
            &TickCtx { dt: 1.0 / 120.0, tick_start: 0.0, tick_index: 0 },
        );

        assert_eq!(
            arena.continuous[Rgb::OUT_COLOR as usize].try_downcast_ref::<Color>(),
            Some(&Color::srgb(1.0, 0.5, 0.0))
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p sway-nodes scene material`
Expected: FAIL — the modules do not exist.

- [ ] **Step 4: Implement `Group` and `Rgb`**

`crates/sway-nodes/src/scene.rs`:

```rust
//! Scene structure nodes: `Group` and `Rgb`. Design §8.

use bevy::prelude::*;
use sway_graph::{
    ContinuousIdx, NoSlots, NodeType, PortView, TickCtx,
};

/// Rotation is three scalar ports rather than one `Vec3`, because rotation is
/// the thing a signal actually drives and every M2a signal node outputs `f32`.
/// A `Vec3` port would need a vector-producing node that does not exist, and
/// §2.4's rule is that a node's ports are simply its fields. Translation and
/// scale stay `Vec3`: nothing drives them at M2b.
#[derive(Reflect, Component)]
pub struct GroupParams {
    pub translation: Vec3,
    /// Euler angles in radians, applied XYZ.
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

impl Default for GroupParams {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Reflect, Default)]
pub struct GroupOutputs {}

#[derive(Component, Default)]
pub struct GroupState;

pub struct Group;

impl Group {
    pub const TRANSLATION: u16 = 0;
    pub const ROTATION_X: u16 = 1;
    pub const ROTATION_Y: u16 = 2;
    pub const ROTATION_Z: u16 = 3;
    pub const SCALE: u16 = 4;
}

impl NodeType for Group {
    type Params = GroupParams;
    type Outputs = GroupOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = GroupState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("translation", Self::TRANSLATION),
        ("rotation_x", Self::ROTATION_X),
        ("rotation_y", Self::ROTATION_Y),
        ("rotation_z", Self::ROTATION_Z),
        ("scale", Self::SCALE),
    ];
    const SPATIAL: bool = true;

    fn register(app: &mut App) {
        app.register_type::<Vec3>();
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let translation: Vec3 = ports.read(ContinuousIdx(Self::TRANSLATION as u32));
        let rx: f32 = ports.read(ContinuousIdx(Self::ROTATION_X as u32));
        let ry: f32 = ports.read(ContinuousIdx(Self::ROTATION_Y as u32));
        let rz: f32 = ports.read(ContinuousIdx(Self::ROTATION_Z as u32));
        let scale: Vec3 = ports.read(ContinuousIdx(Self::SCALE as u32));
        let want = Transform {
            translation,
            rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
            scale,
        };
        // set_if_neq, per parent §2.11: an unconditional assignment re-runs
        // transform propagation for a scene that is not moving.
        match world.get_mut::<Transform>(node) {
            Some(mut transform) => {
                transform.set_if_neq(want);
            }
            None => {
                world.entity_mut(node).insert(want);
            }
        }
    }
}

#[derive(Reflect, Component, Default)]
pub struct RgbParams {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Reflect, Default)]
pub struct RgbOutputs {
    pub color: Color,
}

#[derive(Component, Default)]
pub struct RgbState;

/// Signal → `Color`. §2.4 fixes a material node's ports as the material's own
/// fields, so `base_color` is a `Color` port and something must produce one;
/// nothing in M2a's signal set does (design §8).
pub struct Rgb;

impl Rgb {
    pub const R: u16 = 0;
    pub const G: u16 = 1;
    pub const B: u16 = 2;
    pub const OUT_COLOR: u16 = 3;
}

impl NodeType for Rgb {
    type Params = RgbParams;
    type Outputs = RgbOutputs;
    type Slots = NoSlots;
    type Produces = ();
    type State = RgbState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("r", Self::R),
        ("g", Self::G),
        ("b", Self::B),
        ("color", Self::OUT_COLOR),
    ];

    fn register(app: &mut App) {
        app.register_type::<Color>();
    }

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let r: f32 = ports.read(ContinuousIdx(Self::R as u32));
        let g: f32 = ports.read(ContinuousIdx(Self::G as u32));
        let b: f32 = ports.read(ContinuousIdx(Self::B as u32));
        ports.write(ContinuousIdx(Self::OUT_COLOR as u32), Color::srgb(r, g, b));
    }
}
```

Add `use bevy_ecs::change_detection::DetectChangesMut;` if `set_if_neq` is not in `bevy::prelude`.

- [ ] **Step 5: Implement the material node**

`crates/sway-nodes/src/material.rs`:

```rust
//! `StandardMaterialNode` — one node per material type, per §2.4. Design §8.
//!
//! Named `StandardMaterialNode` rather than `StandardMaterial` because the
//! material type itself is in scope in every file that uses it. §2.4's
//! eventual `MaterialNode<M>` generalisation keeps this shape.

use core::marker::PhantomData;

use bevy::prelude::*;
use sway_graph::{ContinuousIdx, NoSlots, NodeType, PortView, TickCtx};

/// The capability a material node produces: "a handle to a material of type
/// `M`". A `Mesh` node's `material` slot accepts exactly this.
#[derive(TypePath)]
pub struct MaterialOf<M: TypePath + Send + Sync + 'static>(PhantomData<fn() -> M>);

#[derive(Reflect, Component)]
pub struct StandardMaterialParams {
    pub base_color: Color,
    pub emissive: Color,
    pub metallic: f32,
    pub perceptual_roughness: f32,
}

impl Default for StandardMaterialParams {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            emissive: Color::BLACK,
            metallic: 0.0,
            perceptual_roughness: 0.5,
        }
    }
}

#[derive(Reflect, Default)]
pub struct StandardMaterialOutputs {}

/// Owns the handle. `Option` rather than `Handle::default()` so "not created
/// yet" is representable without relying on what a default handle points at.
#[derive(Component, Default)]
pub struct MaterialState {
    pub handle: Option<Handle<StandardMaterial>>,
}

pub struct StandardMaterialNode;

impl StandardMaterialNode {
    pub const BASE_COLOR: u16 = 0;
    pub const EMISSIVE: u16 = 1;
    pub const METALLIC: u16 = 2;
    pub const PERCEPTUAL_ROUGHNESS: u16 = 3;
}

impl NodeType for StandardMaterialNode {
    type Params = StandardMaterialParams;
    type Outputs = StandardMaterialOutputs;
    type Slots = NoSlots;
    type Produces = MaterialOf<StandardMaterial>;
    type State = MaterialState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("base_color", Self::BASE_COLOR),
        ("emissive", Self::EMISSIVE),
        ("metallic", Self::METALLIC),
        ("perceptual_roughness", Self::PERCEPTUAL_ROUGHNESS),
    ];

    fn register(app: &mut App) {
        app.register_type::<Color>();
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let base_color: Color = ports.read(ContinuousIdx(Self::BASE_COLOR as u32));
        let emissive: Color = ports.read(ContinuousIdx(Self::EMISSIVE as u32));
        let metallic: f32 = ports.read(ContinuousIdx(Self::METALLIC as u32));
        let perceptual_roughness: f32 =
            ports.read(ContinuousIdx(Self::PERCEPTUAL_ROUGHNESS as u32));

        let handle = world
            .get::<MaterialState>(node)
            .and_then(|s| s.handle.clone());
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();

        let handle = match handle {
            Some(handle) => handle,
            None => {
                let handle = materials.add(StandardMaterial {
                    base_color,
                    emissive: emissive.into(),
                    metallic,
                    perceptual_roughness,
                    ..default()
                });
                let handle_for_state = handle.clone();
                drop(materials);
                if let Some(mut state) = world.get_mut::<MaterialState>(node) {
                    state.handle = Some(handle_for_state);
                }
                return;
            }
        };

        // Read, compare, and only then `get_mut` — `get_mut` marks the asset
        // changed by the act of being called (parent §2.11).
        let Some(current) = materials.get(&handle) else {
            return;
        };
        let unchanged = current.base_color == base_color
            && current.emissive == emissive.into()
            && current.metallic == metallic
            && current.perceptual_roughness == perceptual_roughness;
        if unchanged {
            return;
        }
        if let Some(material) = materials.get_mut(&handle) {
            material.base_color = base_color;
            material.emissive = emissive.into();
            material.metallic = metallic;
            material.perceptual_roughness = perceptual_roughness;
        }
    }
}
```

- [ ] **Step 6: Wire the modules**

In `crates/sway-nodes/src/lib.rs`, add `mod material; mod scene;` with `pub use material::*; pub use scene::*;`.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes`
Expected: PASS, six new tests plus every pre-existing one.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): Group, Rgb, and the StandardMaterial node"
```

---

## Task 10: The `Mesh` node

**Files:**
- Create: `crates/sway-nodes/src/mesh.rs`
- Modify: `crates/sway-nodes/Cargo.toml` (add `sway-geo`), `crates/sway-nodes/src/lib.rs` (module + `SceneNodesPlugin`)

**Interfaces:**
- Consumes: `Geometry`, `Attribute` (Task 1); `MaterialOf<StandardMaterial>`, `MaterialState` (Task 9); `SlotView` (Task 7).
- Produces: `MeshNode` with `MeshNode::{TRANSLATION, ROTATION_X, ROTATION_Y, ROTATION_Z, SCALE, IN_GEO, IN_MATERIAL}`, `MeshNodeParams`, `MeshNodeState { mesh: Option<Handle<Mesh>> }`; `SceneNodesPlugin`.

- [ ] **Step 1: Write the failing test**

`crates/sway-nodes/src/mesh.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use sway_geo::{Attribute, Geometry};
    use std::sync::Arc;
    use sway_graph::{SlotSource, SlotView};

    fn quad() -> Geometry {
        let mut g = Geometry::new(4);
        g.set(
            "P",
            Attribute::Vec3(Arc::new(vec![
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
            ])),
        );
        g.set("N", Attribute::Vec3(Arc::new(vec![Vec3::Y; 4])));
        g.set("uv", Attribute::Vec2(Arc::new(vec![Vec2::ZERO; 4])));
        g.set_indices(Some(Arc::new(vec![0, 2, 1, 1, 2, 3])));
        g
    }

    fn app_with_mesh() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
        let source = app.world_mut().spawn(quad()).id();
        let node = app
            .world_mut()
            .spawn((MeshNodeParams::default(), MeshNodeState::default()))
            .id();
        (app, source, node)
    }

    fn cook(app: &mut App, node: Entity, source: Entity) {
        let slots = [
            Some(SlotSource { entity: source, plan_index: 0 }),
            None,
        ];
        MeshNode::cook(app.world_mut(), node, &SlotView::new(&slots));
    }

    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<Mesh>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn cooking_uploads_the_geometry_as_a_mesh() {
        let (mut app, source, node) = app_with_mesh();
        cook(&mut app, node, source);

        let handle = app
            .world()
            .get::<Mesh3d>(node)
            .map(|m| m.0.clone())
            .expect("the node inserts Mesh3d");
        let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(6));
    }

    #[test]
    fn re_cooking_unchanged_geometry_does_not_modify_the_asset() {
        // The failure parent §2.11 names: re-uploading a mesh every tick for
        // a scene that is not moving. The gate normally prevents the second
        // cook from being called at all; this asserts the node is not
        // *itself* the thing that would churn if it were.
        let (mut app, source, node) = app_with_mesh();
        cook(&mut app, node, source);
        let _ = count_modified(&mut app);

        cook(&mut app, node, source);

        assert_eq!(count_modified(&mut app), 0);
    }

    #[test]
    fn a_material_slot_source_becomes_mesh_material_3d() {
        let (mut app, source, node) = app_with_mesh();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let material_node = app
            .world_mut()
            .spawn(MaterialState { handle: Some(handle.clone()) })
            .id();

        let slots = [
            Some(SlotSource { entity: source, plan_index: 0 }),
            Some(SlotSource { entity: material_node, plan_index: 1 }),
        ];
        MeshNode::cook(app.world_mut(), node, &SlotView::new(&slots));

        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(node).map(|m| m.0.clone()),
            Some(handle)
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p sway-nodes mesh`
Expected: FAIL — `MeshNode` does not exist.

- [ ] **Step 3: Add the `sway-geo` dependency**

In `crates/sway-nodes/Cargo.toml` add `sway-geo.workspace = true`.

- [ ] **Step 4: Implement the `Mesh` node**

`crates/sway-nodes/src/mesh.rs`:

```rust
//! `MeshNode` — where a `Feeds` chain enters the `ChildOf` tree. Design §8.
//!
//! Parent §2.10 calls this boundary most of what an author needs to
//! understand about the two chain kinds, and it is where the cook gate earns
//! its keep: an ungated version re-uploads a mesh asset every tick for a
//! scene that is not moving.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use sway_geo::Geometry;
use sway_graph::{
    ContinuousIdx, NodeType, PortView, Slot, SlotView, TickCtx, register_slot,
};

use crate::material::{MaterialOf, MaterialState};

#[derive(Reflect, Default)]
pub struct MeshNodeSlots {
    pub geo: Slot<Geometry>,
    pub material: Slot<MaterialOf<StandardMaterial>>,
}

/// Scalar rotation ports, for the reason `GroupParams` gives.
#[derive(Reflect, Component)]
pub struct MeshNodeParams {
    pub translation: Vec3,
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

impl Default for MeshNodeParams {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Reflect, Default)]
pub struct MeshNodeOutputs {}

#[derive(Component, Default)]
pub struct MeshNodeState {
    pub mesh: Option<Handle<Mesh>>,
}

pub struct MeshNode;

impl MeshNode {
    pub const TRANSLATION: u16 = 0;
    pub const ROTATION_X: u16 = 1;
    pub const ROTATION_Y: u16 = 2;
    pub const ROTATION_Z: u16 = 3;
    pub const SCALE: u16 = 4;
    pub const IN_GEO: u16 = 0;
    pub const IN_MATERIAL: u16 = 1;
}

impl NodeType for MeshNode {
    type Params = MeshNodeParams;
    type Outputs = MeshNodeOutputs;
    type Slots = MeshNodeSlots;
    type Produces = ();
    type State = MeshNodeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("translation", Self::TRANSLATION),
        ("rotation_x", Self::ROTATION_X),
        ("rotation_y", Self::ROTATION_Y),
        ("rotation_z", Self::ROTATION_Z),
        ("scale", Self::SCALE),
    ];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] =
        &[("geo", Self::IN_GEO), ("material", Self::IN_MATERIAL)];
    const SPATIAL: bool = true;
    const COOKS: bool = true;

    fn register(app: &mut App) {
        app.register_type::<Vec3>();
        register_slot::<Geometry>(app);
        register_slot::<MaterialOf<StandardMaterial>>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let translation: Vec3 = ports.read(ContinuousIdx(Self::TRANSLATION as u32));
        let rx: f32 = ports.read(ContinuousIdx(Self::ROTATION_X as u32));
        let ry: f32 = ports.read(ContinuousIdx(Self::ROTATION_Y as u32));
        let rz: f32 = ports.read(ContinuousIdx(Self::ROTATION_Z as u32));
        let scale: Vec3 = ports.read(ContinuousIdx(Self::SCALE as u32));
        let want = Transform {
            translation,
            rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
            scale,
        };
        match world.get_mut::<Transform>(node) {
            Some(mut transform) => {
                transform.set_if_neq(want);
            }
            None => {
                world.entity_mut(node).insert(want);
            }
        }
    }

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        if let Some(source) = slots.source(Self::IN_MATERIAL)
            && let Some(handle) = world.get::<MaterialState>(source).and_then(|s| s.handle.clone())
        {
            let current = world
                .get::<MeshMaterial3d<StandardMaterial>>(node)
                .map(|m| m.0.clone());
            if current.as_ref() != Some(&handle) {
                world.entity_mut(node).insert(MeshMaterial3d(handle));
            }
        }

        let Some(source) = slots.source(Self::IN_GEO) else {
            return;
        };
        let Some(geo) = world.get::<Geometry>(source).cloned() else {
            return;
        };
        let Some(mesh) = geometry_to_mesh(&geo) else {
            return;
        };

        let existing = world.get::<MeshNodeState>(node).and_then(|s| s.mesh.clone());
        match existing {
            Some(handle) => {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                if let Some(slot) = meshes.get_mut(&handle) {
                    *slot = mesh;
                }
            }
            None => {
                let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
                world
                    .entity_mut(node)
                    .insert((Mesh3d(handle.clone()), Visibility::default()));
                if let Some(mut state) = world.get_mut::<MeshNodeState>(node) {
                    state.mesh = Some(handle);
                }
            }
        }
    }
}

/// Planar attribute columns → a `bevy` mesh. `P` is required; `N` and `uv`
/// are filled with defaults when absent so a bare point set still draws.
fn geometry_to_mesh(geo: &Geometry) -> Option<Mesh> {
    let positions = geo.get("P")?.as_vec3()?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        positions.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
    );
    let normals: Vec<[f32; 3]> = match geo.get("N").and_then(|a| a.as_vec3()) {
        Some(n) => n.iter().map(|n| [n.x, n.y, n.z]).collect(),
        None => vec![[0.0, 1.0, 0.0]; positions.len()],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    let uvs: Vec<[f32; 2]> = match geo.get("uv").and_then(|a| a.as_vec2()) {
        Some(uv) => uv.iter().map(|uv| [uv.x, uv.y]).collect(),
        None => vec![[0.0, 0.0]; positions.len()],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(indices) = geo.indices() {
        mesh.insert_indices(Indices::U32(indices.as_ref().clone()));
    }
    Some(mesh)
}
```

The `re_cooking_unchanged_geometry_does_not_modify_the_asset` test will fail against the `Some(handle)` branch above, which calls `get_mut` unconditionally. Fix it by comparing first — read the existing mesh's vertex count and indices and skip when they match, or store a cheap fingerprint (`point_count` plus the `Arc::as_ptr` of `P`) in `MeshNodeState` and compare that. Prefer the fingerprint: `Arc` pointer identity is exactly the "did this buffer get rewritten" question, and it costs no comparison of the data.

- [ ] **Step 5: Add the plugin**

In `crates/sway-nodes/src/lib.rs`:

```rust
/// Registers the M2b scene node set.
pub struct SceneNodesPlugin;

impl bevy_app::Plugin for SceneNodesPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        sway_graph::register_node_type::<Group>(app);
        sway_graph::register_node_type::<Rgb>(app);
        sway_graph::register_node_type::<StandardMaterialNode>(app);
        sway_graph::register_node_type::<MeshNode>(app);
    }
}
```

with `mod mesh; pub use mesh::*;`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes && cargo clippy -p sway-nodes -p sway-geo -- -D warnings`
Expected: PASS and clean.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): the Mesh node and the SceneNodesPlugin"
```

---

## Task 11: The `sway-app` handover

**Files:**
- Delete: `crates/sway-app/src/bridge.rs`
- Create: `crates/sway-app/src/midi_feed.rs`, `crates/sway-app/src/demo_graph.rs`
- Modify: `crates/sway-app/src/scene.rs`, `crates/sway-app/src/main.rs`, `crates/sway-app/Cargo.toml`

**Interfaces:**
- Consumes: every node type from Tasks 8–10; `GeoNodesPlugin`, `SceneNodesPlugin`.
- Produces: `midi_feed::{MidiRx, MidiTimeEpoch, feed_midi}` (unchanged behaviour), `demo_graph::setup_demo_graph(world: &mut World)`.

- [ ] **Step 1: Move MIDI ingress out of the throwaway file**

Create `crates/sway-app/src/midi_feed.rs` holding `MidiRx`, `MidiTimeEpoch` and `feed_midi` copied verbatim from `bridge.rs:17-68`, together with its three tests (`bridge.rs:150-237`). Head the file:

```rust
//! MIDI ingress: the CoreMIDI channel into the graph's timestamped inbox.
//!
//! Moved out of the throwaway `bridge.rs` at M2b unchanged — this is ingress,
//! not the temporary cube graph (design §9). M2a's open finding travels with
//! it: the epoch is sampled at first drain, and long-session mach-versus-fixed
//! drift is uncorrected. That is M3's, with the transport.
```

- [ ] **Step 2: Write the failing test for the demo graph**

`crates/sway-app/src/demo_graph.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use sway_geo::{Geometry, GeoNodesPlugin};
    use sway_graph::{CompiledGraph, GraphPlugin};
    use sway_nodes::{SceneNodesPlugin, SignalNodesPlugin};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .add_plugins((GraphPlugin, SignalNodesPlugin, GeoNodesPlugin, SceneNodesPlugin));
        app
    }

    #[test]
    fn the_demo_graph_compiles_and_cooks_a_mesh() {
        let mut app = app();
        setup_demo_graph(app.world_mut());
        assert!(app.world().get_resource::<CompiledGraph>().is_some());

        app.world_mut()
            .insert_resource(Time::<Fixed>::from_hz(120.0));
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

        let mut app = app();
        setup_demo_graph(app.world_mut());

        let mut meshes = app.world_mut().query_filtered::<Entity, With<Mesh3d>>();
        // Mesh3d appears only after the first cook, so parenting is checked
        // through the ChildOf compile applied, not through the drawn mesh.
        let mut parented = app.world_mut().query::<&ChildOf>();
        assert!(
            parented.iter(app.world()).count() >= 1,
            "compile must have applied the hierarchy"
        );
        let _ = meshes;
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p sway-app demo_graph`
Expected: FAIL — the module does not exist.

- [ ] **Step 4: Build the demo graph**

`crates/sway-app/src/demo_graph.rs`, above the tests — §8's graph, built in Rust. M4's project loader replaces this and calls the same `compile`.

```rust
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
    LfoState, MeshNode, MeshNodeParams, MeshNodeState, MidiCC, MidiCCParams, MidiCCState,
    MidiNote, MidiNoteParams, MidiNoteState, Rgb, RgbParams, RgbState, StandardMaterialNode,
    StandardMaterialParams, MaterialState,
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
            LfoParams { hz: 0.1, shape: sway_nodes::Waveform::Saw, phase: 0.0, amplitude: 3.14 },
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
```

**Why `Group::ROTATION_Y` and not a `Vec3` rotation port:** `LFO` outputs `f32`, and a `Vec3` rotation port would need a vector-producing node that does not exist at M2b. Task 9 declares rotation as three scalar ports for exactly this edge; if you reach for a `Vec3` here the compiler rejects it as a type mismatch, which is the type system doing its job rather than a bug to work around.

- [ ] **Step 5: Trim `scene.rs` and rewire `main.rs`**

In `crates/sway-app/src/scene.rs`, delete `Cube`, `colour_for_level`, `apply_level` and the whole `mod tests` (its three tests now live on the material node — §9), leaving only the camera and light spawns. Update the module doc:

```rust
//! The M2b non-graph scene: one camera, one light. Everything else is
//! authored by the graph (design §8). `Camera` and light node types are M5's.
```

In `crates/sway-app/src/main.rs`:
- replace `mod bridge;` with `mod demo_graph; mod midi_feed;`
- replace the `bridge::` / `scene::apply_level` imports with `demo_graph::setup_demo_graph` and `midi_feed::{MidiRx, MidiTimeEpoch, feed_midi}`
- add `sway_geo::GeoNodesPlugin` and `sway_nodes::SceneNodesPlugin` to the `add_plugins` tuple
- replace `.add_systems(Startup, setup_cube_graph)` with `.add_systems(Startup, setup_demo_graph)`
- remove `apply_level` from the `Update` system tuple, leaving `(log_monitors, log_fps)`

Add `sway-geo.workspace = true` to `crates/sway-app/Cargo.toml`.

- [ ] **Step 6: Delete the throwaway**

```bash
git rm crates/sway-app/src/bridge.rs
```

- [ ] **Step 7: Run the whole suite**

Run: `cargo test -p sway-graph -p sway-geo -p sway-nodes -p sway-app`
Expected: PASS. Then `cargo clippy -p sway-graph -p sway-nodes -p sway-geo -- -D warnings` — clean.

- [ ] **Step 8: Verify by eye with real MIDI**

Run: `cargo run -p sway-app -- --windowed --midi ""`
Expected: a displaced grid on screen; CC 74 changes its displacement, notes change its colour, and it rotates slowly. M1's finding stands — two of that milestone's bugs were invisible to every test and only a GPU exposed them, so this step is not optional. If no hardware is available, say so plainly in the findings report rather than implying it was confirmed.

- [ ] **Step 9: Commit**

```bash
git add -A crates/sway-app Cargo.toml
git commit -m "feat(app): drive the scene from graph-authored nodes, retire bridge.rs"
```

---

## Task 12: The findings report

**Files:**
- Create: `docs/superpowers/reports/2026-08-01-m2b-scene-composition-findings.md`

- [ ] **Step 1: Write the report**

Answer §12's five questions, in this order and with evidence rather than impressions:

1. Did the sticky dirty flag (§6) hold, or did a case appear that it gets wrong — in particular any cook whose correctness depends on a value the gate does not observe?
2. Did two orders (§7) hold, or did a real graph want an ordering constraint spanning both DAGs?
3. What did `Geometry`'s `Arc` sharing actually save, measured rather than assumed?
4. How did slot typing (§4) read at the call site — is `Slots` plus `Produces` the right split, or does one want to be the other?
5. What the tick costs with cooks in it, recorded as a data point for §11's tick-rate question and explicitly not as its answer.

Also record, in a "what a later milestone would otherwise rediscover" section, every API surprise hit along the way — `#[derive(Reflect)]` bounds on `Slot<T>`, unit-struct `TypeInfo`, the `ChildOf` import path, and whether `Group` needed the scalar `rotation_y` port Task 11 anticipates.

- [ ] **Step 2: Add a Revision line to the design if anything was wrong**

If any of §3–§8's decisions turned out wrong, add a `**Revision:**` line at the top of `docs/superpowers/specs/2026-08-01-m2b-scene-composition-design.md` in the style the parent spec and the M2a design use, and correct the relevant sections. A design document that records what was believed beforehand and is never corrected afterwards is worse than none.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers
git commit -m "docs: M2b scene composition findings"
```
