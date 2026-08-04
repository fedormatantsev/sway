# Ports as Component Values Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove `FieldKind`, `EdgeKind`, `Product`, `Spatial`, and `PortArena` so port values live on `Inlets`/`Outlets` components, hierarchy uses plain `Entity` fields, and buffer flow uses handle values — with engine policy keyed only by `TypeId` / type data.

**Architecture:** Gather copies outlet fields into inlet fields on the node entities themselves. `Outlets` becomes an ECS component beside `Inlets`. Compile special-cases `TypeId::of::<Entity>()` for `ChildOf`, single-consumer fan-out, and sort exclusion. Events still clear via `ReflectEventList`. Heavy geometry moves over store-keyed `GeoHandle` values (index + generation into `GeometryStore` — **not** entities). Materials already have a small key — Bevy's `Handle<StandardMaterial>` travels on the port directly; there is no second material table. Cook-source change ticks register through a TypeId-keyed registry so `sway-graph` never names `GeoHandle`. `PortView` becomes a view over one node's taken `Inlets`/`Outlets` structs for the duration of tick+cook.

**Tech Stack:** Rust 2024, bevy 0.19 subcrates (`bevy_ecs`, `bevy_reflect`, `bevy_app`, `bevy_time`, `bevy_transform`, `bevy_math`), masonry (editor).

**Spec:** `docs/superpowers/specs/2026-08-04-ports-as-component-values-design.md`

## Global Constraints

- `sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform` only. **Not** the `bevy` facade, **not** `bevy_render`, **not** `sway-geo`. The manifest is the only place this is enforced. Therefore `GeoHandle` must not appear by name in `sway-graph`; cook-source policy reaches the engine only as the cook-source registry.
- `sway-editor` may depend on `sway-graph`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_transform`. **Not** `bevy`, `bevy_render`, `wgpu`, `vello`, `imaging_vello`.
- The graph model has **no** `FieldKind`, **no** `EdgeKind`, **no** stored "this is a product edge" flag. Editor colour may map `TypeId` → style locally; it must not persist a kind enum.
- The tick is infallible. All validation happens in `compile`.
- Use `reflect_clone()`, never `to_dynamic()`, for any gather that must later downcast to a concrete type.
- Entity ids appear on ports **only** for hierarchy (`Entity` / `Vec<Entity>`). Buffer / material connections carry handle values.
- Clippy gate for this work: `cargo clippy -p sway-graph -p sway-geo -p sway-nodes -p sway-editor --all-targets -- -D warnings`. `cargo clippy --workspace` was already red on `main` before this milestone; do not attribute pre-existing debt here.

## Design decisions this plan makes

These are implementation choices the spec leaves open. Record them here so every task agrees.

1. **`GeoHandle` is a store key, not an entity.** `{ index: u32, generation: u32 }` with `index == u32::MAX` meaning unset. It names a slot in `GeometryStore` (`sway-geo`). No `Entity` in the type or API. **`Geometry` stops being an ECS component on the producer** — Grid/Displace insert into the store and write the key on `Outlets`; Mesh reads via `store.get`. Per-slot change ticks drive consumer recooks. A later GPU residency layer can keep the same key shape (design §7).

2. **Material ports carry Bevy's `Handle<StandardMaterial>` directly.** That handle is already a small asset key — it is **not** heavy data and does **not** need a `MaterialStore` or a parallel `MaterialHandle` slab. `MaterialState` keeps owning the handle as today; tick also writes it onto `Outlets`. If `Handle<StandardMaterial>` is not `Reflect` out of the box in bevy 0.19, register it (or a one-field newtype that is still the Bevy handle, still not a second table).

3. **Cook-source registry** carries `fn(&World, &dyn PartialReflect) -> Option<Tick>`. `sway-geo` registers it for `GeoHandle` (look up the store slot's change tick). Material asset handles do **not** register (param edits do not require mesh consumers to re-cook — same as today's material `produced_change_tick` → `None`). Compile collects inlet slots that have a cook-source entry into `NodePlan.cook_sources`.

4. **`Outlets: Component`.** Both halves sit on the node entity. `insert_defaults` inserts both. Gather reads/writes component fields in place. Around each node's tick+cook, the runner `take`s both components into a `PortView`, then `insert`s them back so the next node in order can gather from this node's outlets.

5. **Prefill goes away.** Values live on the components. Unconnected inlets are whatever the component holds (editor-authored, or the last gathered value after a disconnect). Recompile no longer restores a shadowed authored copy — that dual store was the arena. Update disconnect tests to the new rule.

6. **`PortView::source` is deleted.** Nodes read `GeoHandle` / `Handle<StandardMaterial>` / `Entity` with `read`. Resolve geometry through `GeometryStore::get`; use the material handle as today with `Assets` / `MeshMaterial3d`; use `Entity` values directly for hierarchy.

7. **Error rename:** `SpatialFanOut` → `ParentFanOut`. Display text says parenting / `ChildOf`, not `Spatial`.

## Expected build state during the flip

**Tasks 1–2 are additive: the whole workspace builds and all tests pass.**

**Task 3 replaces the engine.** From the moment Task 3 lands until Task 7 completes, downstream crates may not compile. Each task states the scoped command that must pass:

| After task | Must pass |
|---|---|
| 1, 2 | `cargo test --workspace` |
| 3 | `cargo test -p sway-graph` |
| 4 | `cargo test -p sway-graph -p sway-geo` |
| 5 | `cargo test -p sway-graph -p sway-geo -p sway-nodes` |
| 6 | `cargo test --workspace --exclude sway-editor` |
| 7 | `cargo test --workspace` |
| 8 | `cargo test --workspace` + the clippy gate above |

## File structure

**`crates/sway-graph/src/`**

| File | Responsibility after this work |
|---|---|
| `ports.rs` | `Occurrence<T>`, `Events<T>`, `clear_events_of` — **no** `PortArena`, `Product`, or `Spatial` |
| `schema.rs` | `FieldSpec` without `kind`; `derive_fields`; `ReflectEventList` + `register_events`; **new** `CookSourceRegistry` + `register_cook_source`; **deleted** `FieldKind`, `ReflectProduct`, `ProductAccess`, `register_product` |
| `field_ops.rs` | **new** — reflect helpers: read/write a struct field or `Vec` element by index; used by gather, clear, seed, and `PortView` |
| `edges.rs` | unchanged edge shape; `NodeRuntime.last_product_ticks` → `last_source_ticks` |
| `registry.rs` | `Outlets: Component`; drop `prefill` / arena `seed_outlets`; `with_ports` take/insert; drop product outlet limit |
| `compile.rs` | no arena layout; copies are field addresses; Entity TypeId policies; cook_sources from `ReflectCookSource`; seed Entity outlets |
| `tick.rs` | clear Events on components; gather field→field; take ports; tick; cook gate via `cook_sources`; no `PortArena` resource |
| `view.rs` | `PortView` over `&mut dyn Struct` inlets + outlets |
| `test_nodes.rs` | Entity hierarchy fixtures; handle-free cook fixtures use a local `Blob` component + optional test handle type data if needed |

**Other crates:** `sway-geo` (`GeoHandle`, `GeometryStore`, `grid`, `displace`, `geometry`), `sway-nodes` (`scene`, `mesh`, `material`, signals), `sway-app` (`demo_graph`), `sway-editor` (`snapshot`, `canvas`, `test_graph`).

---

### Task 1: `GeoHandle` and `GeometryStore`

Adds the geometry store key and the CPU table it names. Nothing in the graph consumes them yet. **No `Entity` in the handle.** Materials are out of scope here — they already have Bevy's asset `Handle`.

**Files:**
- Create: `crates/sway-geo/src/handle.rs` — `GeoHandle`
- Create: `crates/sway-geo/src/store.rs` — `GeometryStore` resource
- Modify: `crates/sway-geo/src/lib.rs`

**Interfaces:**
- Produces: `GeoHandle { index: u32, generation: u32 }` with `UNSET` (`index == u32::MAX`), `is_unset`, `Default`/`Reflect`/`Copy`/`Clone`/`PartialEq`/`Eq`/`Hash`. **No** `from_node` / `node` / `Entity`.
- Produces: `GeometryStore` resource with `insert(Geometry) -> GeoHandle`, `update(GeoHandle, Geometry) -> Option<()>`, `get(GeoHandle) -> Option<&Geometry>`, `change_tick(GeoHandle) -> Option<Tick>`.
- Consumes: nothing from later tasks.

- [ ] **Step 1: Write the failing `GeoHandle` / store tests**

Add `crates/sway-geo/src/handle.rs`:

```rust
//! `GeoHandle` — a small value naming a slot in [`GeometryStore`](crate::store::GeometryStore).
//!
//! The graph matches this by `TypeId` like `f32`. It does not carry an
//! `Entity`: high-cardinality geometry lives in the store; only this key
//! travels on edges (design §7).

use bevy_reflect::Reflect;

/// Unset / unconnected handle (`index == u32::MAX`).
pub const GEO_HANDLE_UNSET_INDEX: u32 = u32::MAX;

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeoHandle {
    pub index: u32,
    pub generation: u32,
}

impl Default for GeoHandle {
    fn default() -> Self {
        Self { index: GEO_HANDLE_UNSET_INDEX, generation: 0 }
    }
}

impl GeoHandle {
    pub fn is_unset(self) -> bool {
        self.index == GEO_HANDLE_UNSET_INDEX
    }
}
```

Add `crates/sway-geo/src/store.rs`:

```rust
//! CPU geometry table. Keys are [`GeoHandle`]s; values are [`Geometry`].

use bevy_ecs::change_detection::Tick;
use bevy_ecs::resource::Resource;

use crate::geometry::Geometry;
use crate::handle::{GeoHandle, GEO_HANDLE_UNSET_INDEX};

struct Slot {
    generation: u32,
    geo: Geometry,
    /// Advanced on every `insert`/`update` that writes this slot.
    changed: Tick,
}

#[derive(Resource, Default)]
pub struct GeometryStore {
    slots: Vec<Option<Slot>>,
    free: Vec<u32>,
}

impl GeometryStore {
    pub fn insert(&mut self, geo: Geometry, now: Tick) -> GeoHandle {
        let slot = Slot { generation: 0, geo, changed: now };
        if let Some(index) = self.free.pop() {
            let handle = GeoHandle { index, generation: 0 };
            self.slots[index as usize] = Some(slot);
            handle
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Some(slot));
            GeoHandle { index, generation: 0 }
        }
    }

    /// Overwrite an existing slot in place (same index, generation unchanged).
    /// Returns `None` if the handle is stale or unset. Advances `changed`.
    pub fn update(&mut self, handle: GeoHandle, geo: Geometry, now: Tick) -> Option<()> {
        if handle.is_unset() {
            return None;
        }
        let slot = self.slots.get_mut(handle.index as usize)?.as_mut()?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.geo = geo;
        slot.changed = now;
        Some(())
    }

    pub fn get(&self, handle: GeoHandle) -> Option<&Geometry> {
        if handle.is_unset() {
            return None;
        }
        let slot = self.slots.get(handle.index as usize)?.as_ref()?;
        (slot.generation == handle.generation).then_some(&slot.geo)
    }

    pub fn change_tick(&self, handle: GeoHandle) -> Option<Tick> {
        if handle.is_unset() {
            return None;
        }
        let slot = self.slots.get(handle.index as usize)?.as_ref()?;
        (slot.generation == handle.generation).then_some(slot.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_reflect::PartialReflect;

    #[test]
    fn a_geo_handle_survives_reflect_clone() {
        let original = GeoHandle { index: 3, generation: 1 };
        let cloned = original
            .reflect_clone()
            .expect("GeoHandle must reflect_clone")
            .into_partial_reflect();
        let cloned = cloned
            .try_downcast_ref::<GeoHandle>()
            .expect("reflect_clone must preserve the concrete type");
        assert_eq!(*cloned, original);
    }

    #[test]
    fn store_insert_get_and_update_advance_change_tick() {
        let mut store = GeometryStore::default();
        let t0 = Tick::new(1);
        let t1 = Tick::new(2);
        let handle = store.insert(Geometry::new(1), t0);
        assert!(!handle.is_unset());
        assert_eq!(store.get(handle).map(|g| g.point_count()), Some(1));
        assert_eq!(store.change_tick(handle), Some(t0));

        store.update(handle, Geometry::new(4), t1).expect("live handle");
        assert_eq!(store.get(handle).map(|g| g.point_count()), Some(4));
        assert_eq!(store.change_tick(handle), Some(t1));
    }

    #[test]
    fn an_unset_handle_misses_the_store() {
        let store = GeometryStore::default();
        assert!(GeoHandle::default().is_unset());
        assert!(store.get(GeoHandle::default()).is_none());
        assert_eq!(GEO_HANDLE_UNSET_INDEX, u32::MAX);
    }
}
```

Wire in `lib.rs`: `mod handle; mod store; pub use handle::GeoHandle; pub use store::GeometryStore;`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p sway-geo --lib store::`
Expected: FAIL until wired, then PASS.

- [ ] **Step 3: Ensure workspace still green**

Run: `cargo test --workspace`
Expected: PASS (store unused by nodes yet; `GeoNodesPlugin` may `init_resource::<GeometryStore>()` now or in Task 4).

- [ ] **Step 4: Commit**

```bash
git add crates/sway-geo/src/handle.rs crates/sway-geo/src/store.rs crates/sway-geo/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(geo): store-keyed GeoHandle and GeometryStore

Geometry buffers live in a CPU table; the port carries index+generation,
not an Entity and not a second wrapper around Bevy asset handles.
EOF
)"
```

---

### Task 2: Cook-source registry and field reflect helpers

Additive engine support: a TypeId-keyed cook-source change-tick registry, and pure reflect field get/set used by the later gather. `sway-geo` registers `GeoHandle` → `GeometryStore::change_tick`. Nothing switches node storage yet.

**Files:**
- Create: `crates/sway-graph/src/field_ops.rs`
- Modify: `crates/sway-graph/src/schema.rs`
- Modify: `crates/sway-graph/src/lib.rs`
- Modify: `crates/sway-geo/src/lib.rs` (or `handle.rs`) to register type data from a small `register_geo_handle(app)` helper
- Modify: `crates/sway-geo/src/grid.rs` / plugin to call registration (or `GeoNodesPlugin`)

**Interfaces:**
- Produces: `CookSourceRegistry` resource mapping `TypeId → fn(&World, &dyn PartialReflect) -> Option<Tick>`; `register_cook_source::<T>(app, change_tick)`.
- Produces: `field_ops::{read_slot, write_slot, clone_slot_value}` operating on `&dyn Struct` / `&mut dyn Struct` with `(field_index, element_index, variadic)`.
- Consumes: `GeoHandle` + `GeometryStore` from Task 1.

- [ ] **Step 1: Write the failing field_ops tests**

Create `crates/sway-graph/src/field_ops.rs`:

```rust
//! Reflect get/set for one Inlets/Outlets field slot (scalar or Vec element).

use bevy_reflect::structs::Struct;
use bevy_reflect::PartialReflect;

pub fn clone_slot_value(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|e| {
            panic!(
                "could not reflect_clone a `{}` port value ({e:?})",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// Reads one slot from a reflected struct field.
pub fn read_slot<'a>(
    root: &'a dyn Struct,
    field_index: usize,
    index: usize,
    variadic: bool,
) -> &'a dyn PartialReflect {
    let field = root
        .field_at(field_index)
        .unwrap_or_else(|| panic!("field_index {field_index} out of range"));
    if variadic {
        field
            .reflect_ref()
            .as_list()
            .expect("variadic field must reflect as a list")
            .get(index)
            .unwrap_or_else(|| panic!("element {index} out of range"))
    } else {
        field
    }
}

/// Overwrites one slot on a reflected struct field.
pub fn write_slot(
    root: &mut dyn Struct,
    field_index: usize,
    index: usize,
    variadic: bool,
    value: Box<dyn PartialReflect>,
) {
    if variadic {
        let field = root
            .field_at_mut(field_index)
            .expect("field_index in range");
        let list = field
            .reflect_mut()
            .as_list_mut()
            .expect("variadic field must reflect as a list");
        *list.get_mut(index).expect("element in range") = value;
    } else {
        let field = root
            .field_at_mut(field_index)
            .expect("field_index in range");
        *field = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_reflect::Reflect;

    #[derive(Reflect, Default)]
    struct Sample {
        gain: f32,
        terms: Vec<f32>,
    }

    #[test]
    fn read_and_write_a_scalar_and_a_vec_element() {
        let mut sample = Sample { gain: 1.0, terms: vec![2.0, 3.0] };
        let root: &dyn Struct = sample.reflect_ref().as_struct().unwrap();
        assert_eq!(
            read_slot(root, 0, 0, false).try_downcast_ref::<f32>(),
            Some(&1.0)
        );
        assert_eq!(
            read_slot(root, 1, 1, true).try_downcast_ref::<f32>(),
            Some(&3.0)
        );

        let root = sample.reflect_mut().as_struct_mut().unwrap();
        write_slot(root, 0, 0, false, Box::new(9.0_f32));
        write_slot(root, 1, 0, true, Box::new(8.0_f32));
        assert_eq!(sample.gain, 9.0);
        assert_eq!(sample.terms[0], 8.0);
    }
}
```

Add `pub mod field_ops;` and re-export nothing public yet (or `pub use field_ops::{read_slot, write_slot}` if useful).

- [ ] **Step 2: Add `ReflectCookSource`**

In `schema.rs`, after `ReflectEventList`:

```rust
use bevy_ecs::change_detection::Tick;
use bevy_ecs::world::World;

/// Type data: this slot value names a cook upstream whose change tick the
/// cook gate must watch (design §3 — policy via type data, not a field kind).
#[derive(Clone, Copy)]
pub struct ReflectCookSource {
    pub change_tick: fn(&World, &dyn PartialReflect) -> Option<Tick>,
}

/// Registers `T` and attaches [`ReflectCookSource`].
pub fn register_cook_source<T>(
    app: &mut App,
    change_tick: fn(&World, &dyn PartialReflect) -> Option<Tick>,
) where
    T: Reflect + TypePath + Typed + GetTypeRegistration,
{
    let registry = app.world().resource::<bevy_ecs::reflect::AppTypeRegistry>().clone();
    let mut registry = registry.write();
    registry.register::<T>();
    registry.register_type_data::<T, ReflectCookSource>();
    // Bevy's register_type_data uses FromType; for a free fn pointer we set it manually:
    let registration = registry.get_mut(core::any::TypeId::of::<T>())
        .expect("just registered");
    registration.insert(ReflectCookSource { change_tick });
}
```

**Important:** Bevy's `register_type_data::<T, D>()` requires `D: FromType<T>`. Prefer implementing that with a stored fn via a registration helper that does NOT use `FromType` — use `TypeRegistration::insert` after `register::<T>()` as above, **or** keep a thin wrapper:

If `insert` on the registration is awkward with the bevy version in tree, use this pattern instead (matches existing `register_events`):

```rust
impl FromType<GeoHandle> for ReflectCookSource {
    // cannot — GeoHandle is not in sway-graph
}
```

So the helper must manually `registration.insert(ReflectCookSource { change_tick })`. Verify against bevy 0.19 `TypeRegistration` API in-tree; if `insert` is private, store the fn on a resource map `HashMap<TypeId, CookSourceFn>` in `NodeTypeRegistry` filled by `register_cook_source`. **Prefer the resource map if type-data insert fights the API** — same external behaviour.

Concrete fallback (use if type-data insert fails):

```rust
// schema.rs or registry.rs
#[derive(Resource, Default)]
pub struct CookSourceRegistry {
    pub by_type: HashMap<TypeId, fn(&World, &dyn PartialReflect) -> Option<Tick>>,
}

pub fn register_cook_source<T>(app: &mut App, change_tick: fn(&World, &dyn PartialReflect) -> Option<Tick>)
where
    T: 'static,
{
    app.init_resource::<CookSourceRegistry>();
    app.world_mut()
        .resource_mut::<CookSourceRegistry>()
        .by_type
        .insert(TypeId::of::<T>(), change_tick);
}
```

Pick **one** approach in this task and use it consistently in Task 3. The resource map is YAGNI-friendlier with bevy 0.19; prefer it unless type data inserts cleanly in five minutes.

- [ ] **Step 3: Register GeoHandle's cook source from sway-geo**

```rust
// sway-geo — e.g. register_geo_handle(app) called from GeoNodesPlugin
use sway_graph::register_cook_source;
use crate::handle::GeoHandle;
use crate::store::GeometryStore;

fn geo_handle_change_tick(world: &World, value: &dyn PartialReflect) -> Option<Tick> {
    let handle = *value.try_downcast_ref::<GeoHandle>()?;
    world.resource::<GeometryStore>().change_tick(handle)
}

pub fn register_geo_handle(app: &mut App) {
    app.init_resource::<GeometryStore>();
    app.register_type::<GeoHandle>();
    register_cook_source::<GeoHandle>(app, geo_handle_change_tick);
}
```

Call `register_geo_handle` from the geo plugin before registering `Grid`/`Displace`. **Do not** look up any `Entity` or `Geometry` component.

- [ ] **Step 4: Run tests**

Run: `cargo test -p sway-graph --lib field_ops::`
Expected: PASS

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/field_ops.rs crates/sway-graph/src/schema.rs crates/sway-graph/src/lib.rs \
  crates/sway-graph/src/registry.rs crates/sway-geo/src/lib.rs crates/sway-geo/src/store.rs
git commit -m "$(cat <<'EOF'
feat(graph,geo): field reflect helpers and GeoHandle cook-source registry

Cook gate watches GeometryStore slot ticks via TypeId, without naming
GeoHandle inside sway-graph and without Entity on the handle.
EOF
)"
```

---

### Task 3: Engine flip — schema, ports, view, compile, tick, registry

Replaces the arena model inside `sway-graph`. Downstream crates break until Tasks 4–7 migrate.

**Files:**
- Modify: `crates/sway-graph/src/ports.rs` — delete `PortArena`, `Product`, `Spatial`; keep `Events`/`Occurrence`/`clear_events_of`
- Modify: `crates/sway-graph/src/schema.rs` — `FieldSpec` without `kind`; delete `FieldKind`/`ReflectProduct`/`ProductAccess`/`register_product`; `derive_fields` only walks types + Events diagnostic
- Modify: `crates/sway-graph/src/view.rs` — `PortView` over inlets/outlets structs
- Modify: `crates/sway-graph/src/compile.rs` — Entity policies; field-address copies; no arena bases
- Modify: `crates/sway-graph/src/tick.rs` — component gather/clear; take/insert ports; no `PortArena` resource
- Modify: `crates/sway-graph/src/registry.rs` — `Outlets: Component`; drop prefill; seed Entity outlets on the component
- Modify: `crates/sway-graph/src/edges.rs` — rename `last_product_ticks` → `last_source_ticks`
- Modify: `crates/sway-graph/src/lib.rs` — exports
- Modify: `crates/sway-graph/src/test_nodes.rs` — rewrite fixtures

**Interfaces:**
- Consumes: `field_ops`, `CookSourceRegistry` / `ReflectCookSource` from Task 2.
- Produces:
  - `FieldSpec { name, field_index, slot_type, slot_type_path, variadic }`
  - `NodePlan { entity, node_type, fields, inlet_field_count, field_lens, connected: Vec<bool> /* per inlet slot flat */, copies: Vec<CopyEdge>, cook_sources: Vec<CookSourceRef>, entity_outlet: Option<usize /* field_index in Outlets */> }`
  - `CopyEdge { source: Entity, from_field: u16, from_index: u16, to_field: u16, to_index: u16 }`
  - `ClearRef { entity: Entity, outlets: bool, field_index: usize, index: usize, variadic: bool, clear: fn(&mut dyn PartialReflect) }`
  - `NodeType::Outlets: Component + Default + Reflect + …`
  - `CompileError::ParentFanOut` (renamed)
  - No `PortArena`, `Product`, `Spatial`, `FieldKind`, `register_product`, `PortView::source`

- [ ] **Step 1: Rewrite schema tests first (TDD)**

Replace `schema.rs` tests with:

```rust
#[derive(Reflect, Default)]
struct SampleInlets {
    children: Vec<Entity>,
    triggers: Vec<Events<NoteMsg>>,
    gain: f32,
    terms: Vec<f32>,
}

fn fields_registry() -> TypeRegistry {
    let mut r = TypeRegistry::new();
    r.register::<NoteMsg>();
    r.register::<SampleInlets>();
    r.register::<Entity>();
    r.register::<Events<NoteMsg>>();
    r.register_type_data::<Events<NoteMsg>, ReflectEventList>();
    r
}

#[test]
fn derive_fields_has_no_kind_and_reports_slot_types() {
    let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");
    assert_eq!(
        fields.iter().map(|f| f.name).collect::<Vec<_>>(),
        vec!["children", "triggers", "gain", "terms"]
    );
    assert!(fields[0].variadic);
    assert_eq!(fields[0].slot_type, TypeId::of::<Entity>());
    assert!(fields[1].variadic);
    assert_eq!(fields[1].slot_type, TypeId::of::<Events<NoteMsg>>());
    assert!(!fields[2].variadic);
    assert_eq!(fields[2].slot_type, TypeId::of::<f32>());
}

#[test]
fn an_unregistered_events_field_is_still_an_error() {
    let mut r = TypeRegistry::new();
    r.register::<NoteMsg>();
    r.register::<SampleInlets>();
    r.register::<Events<NoteMsg>>();
    // no ReflectEventList
    let msg = derive_fields::<SampleInlets>(&r).unwrap_err().to_string();
    assert!(msg.contains("triggers"));
    assert!(msg.contains("register_events"));
}
```

Delete tests that mention `FieldKind`, `Product`, `Spatial`, `ProductAccess`.

- [ ] **Step 2: Implement kindless `derive_fields`**

```rust
pub struct FieldSpec {
    pub name: &'static str,
    pub field_index: usize,
    pub slot_type: TypeId,
    pub slot_type_path: &'static str,
    pub variadic: bool,
}

pub fn derive_fields<T: Typed>(registry: &TypeRegistry) -> Result<Vec<FieldSpec>, SchemaError> {
    let s = struct_info::<T>()?;
    let mut fields = Vec::with_capacity(s.field_len());
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");
        let (slot_type, slot_type_path, variadic) = match field.type_info() {
            Some(TypeInfo::List(list)) => {
                let item = list.item_ty();
                (item.id(), item.path(), true)
            }
            _ => (field.type_id(), field.type_path(), false),
        };
        if registry.get_type_data::<ReflectEventList>(slot_type).is_none()
            && is_events_marker_path(slot_type_path)
        {
            return Err(SchemaError::UnregisteredEventsField {
                type_path: s.type_path(),
                field: field.name(),
            });
        }
        fields.push(FieldSpec {
            name: field.name(),
            field_index: i,
            slot_type,
            slot_type_path,
            variadic,
        });
    }
    Ok(fields)
}
```

Delete `FieldKind`, `ReflectProduct`, `ProductAccess`, `register_product`, `UnregisteredProductField`, `is_product_marker_path`.

- [ ] **Step 3: Slim `ports.rs`**

Delete `PortArena`, `Product`, `Spatial`, and their tests. Keep `Events`, `Occurrence`, `clear_events_of`, and the Events reflect_clone / clear-in-place tests (adjust clear test to use a concrete `Events<u8>` mut ref, not only a box — both are fine).

- [ ] **Step 4: Rewrite `PortView`**

```rust
pub struct PortView<'a> {
    inlets: &'a mut dyn Struct,
    outlets: &'a mut dyn Struct,
    fields: &'a [FieldSpec],
    inlet_field_count: usize,
    field_lens: &'a [usize],
    connected: &'a [bool], // length = sum of inlet field_lens
}

impl<'a> PortView<'a> {
    fn root_and_local(&mut self, field: u16) -> (&mut dyn Struct, usize, bool /*is_inlet*/) {
        let f = field as usize;
        if f < self.inlet_field_count {
            (self.inlets, f, true)
        } else {
            (self.outlets, f - self.inlet_field_count, false)
        }
    }

    pub fn read<T: Reflect + Clone>(&self, field: u16) -> T { self.read_at(field, 0) }

    pub fn read_at<T: Reflect + Clone>(&self, field: u16, index: u16) -> T {
        let f = field as usize;
        let (root, spec_index, _) = if f < self.inlet_field_count {
            (&*self.inlets as &dyn Struct, f, true)
        } else {
            (&*self.outlets as &dyn Struct, f - self.inlet_field_count, false)
        };
        let spec = &self.fields[f];
        let value = crate::field_ops::read_slot(root, spec.field_index, index as usize, spec.variadic);
        value.try_downcast_ref::<T>().expect("compiled type").clone()
    }

    pub fn write<T: Reflect>(&mut self, field: u16, value: T) { self.write_at(field, 0, value); }

    pub fn write_at<T: Reflect>(&mut self, field: u16, index: u16, value: T) {
        let f = field as usize;
        let spec = &self.fields[f].clone();
        let variadic = spec.variadic;
        let field_index = spec.field_index;
        let root: &mut dyn Struct = if f < self.inlet_field_count {
            self.inlets
        } else {
            self.outlets
        };
        crate::field_ops::write_slot(root, field_index, index as usize, variadic, Box::new(value));
    }

    // events / emit: downcast Events<T> via read_slot / write path on the outlet field
    // is_connected: map (field,index) to flat inlet slot index
}
```

Delete `source`. Update boundary panic tests to construct a `PortView` over local structs instead of an arena.

- [ ] **Step 5: Rewrite compile policies**

Key changes in `compile`:

1. Layout no longer assigns `base` / arena offsets. Keep `field_lens` from `inlet_lens` + outlet 1s. Build flat `connected` sized to inlet slots only (or total — pick inlet-only and document).
2. Validate TypeId equality as today.
3. **Entity policy:** `target_spec.slot_type == TypeId::of::<Entity>()` (and for variadic children, element type is `Entity`) ⇒ single-consumer on the **source** outlet (ParentFanOut), record `parent_of`, exclude from topo, still include in gather copies.
4. Build `copies: Vec<CopyEdge>` with entity + field + index endpoints (not absolute slots).
5. Build `clears: Vec<ClearRef>` by walking fields: if `CookSourceRegistry`/`ReflectEventList` for `slot_type`, push clear refs. Use `registry.get_type_data::<ReflectEventList>(slot_type).clear`.
6. Build `cook_sources: Vec<CookSourceRef { field, index, change_tick }>` from cook-source registry for **inlet** slots.
7. Detect `entity_outlet: Option<usize>` — the outlets field whose `slot_type == TypeId::of::<Entity>()` (at most one; panic at registration if more than one Entity outlet, analogous to old product limit — **or** allow multiple and seed all; prefer seed all Entity-typed outlet fields, drop the "at most one product" check).
8. Apply `ChildOf` as today.
9. Seed Entity outlets **here or in tick once:** write `plan.entity` into each Entity-typed outlet field on the `Outlets` component.

Rename `SpatialFanOut` → `ParentFanOut` and update Display strings to say parenting / `ChildOf`.

- [ ] **Step 6: Rewrite registry**

```rust
type Outlets: Reflect + Typed + GetTypeRegistration + Component + Default;

// Remove PrefillFn, Prefill from NodeTypeEntry
// SeedOutletsFn becomes optional / only Entity seeding done in compile

fn insert_defaults_of<N: NodeType>(world: &mut World, node: Entity) {
    if world.get::<N::Inlets>(node).is_none() {
        world.entity_mut(node).insert(N::Inlets::default());
    }
    if world.get::<N::Outlets>(node).is_none() {
        world.entity_mut(node).insert(N::Outlets::default());
    }
    if world.get::<N::State>(node).is_none() {
        world.entity_mut(node).insert(N::State::default());
    }
}

fn check_outlets<N: NodeType>(outlets: &[FieldSpec]) {
    // only: no Vec outlets
    if let Some(variadic) = outlets.iter().find(|f| f.variadic) {
        panic!(...);
    }
}

pub fn with_ports_of<N: NodeType>(
    world: &mut World,
    entity: Entity,
    plan: &NodePlan,
    f: impl FnOnce(&mut World, &mut PortView),
) {
    let mut inlets = world
        .entity_mut(entity)
        .take::<N::Inlets>()
        .unwrap_or_default();
    let mut outlets = world
        .entity_mut(entity)
        .take::<N::Outlets>()
        .unwrap_or_default();
    {
        let inlets_s = inlets.reflect_mut().as_struct_mut().expect("struct");
        let outlets_s = outlets.reflect_mut().as_struct_mut().expect("struct");
        let mut view = PortView::new(inlets_s, outlets_s, &plan.fields, plan.inlet_field_count, &plan.field_lens, &plan.connected);
        f(world, &mut view);
    }
    world.entity_mut(entity).insert((inlets, outlets));
}
```

Store `with_ports: fn(&mut World, Entity, &NodePlan, &mut dyn FnMut(&mut World, &mut PortView))` on the entry — use a concrete fn pointer wrapping `with_ports_of::<N>` that takes a `TickFn` style. Practical pattern:

```rust
pub type WithPortsFn = fn(&mut World, Entity, &NodePlan, TickFn, &TickCtx);
// and a second for cook
```

Or keep tick/cook as today but change them to receive ports only after the runner does:

```rust
// entry still has tick: TickFn
// runner:
(entry.take_ports)(world, plan.entity, plan, &mut |world, view| {
    (entry.tick)(world, plan.entity, view, &ctx);
    // cook...
});
```

Implement `TakePortsFn` accordingly.

- [ ] **Step 7: Rewrite `graph_tick`**

```text
remove CompiledGraph
build dispatch + ctx
for each ClearRef: get mut Inlets or Outlets, clear Events field in place via field_ops
for each plan in order:
  dirty = false
  for copy in plan.copies:
    read source Outlets slot (world)
    compare/write target Inlets slot (world)
    dirty |= changed
  if inlets_changed_tick moved: dirty = true; record tick  // no prefill
  sticky cook_dirty
  take_ports → tick → cook gate (cook_sources change ticks vs last_source_ticks) → insert ports
reinsert CompiledGraph
```

Delete `PortArena` from `GraphPlugin`.

- [ ] **Step 8: Rewrite `test_nodes.rs` and compile/tick tests**

- `GroupInlets { children: Vec<Entity>, ... }`, `GroupOutlets { entity: Entity }` with `#[derive(Component)]` on outlets.
- `Producer`/`Consumer`: replace `Product<Blob>` with a test-local `BlobHandle` **or** keep cook tests using a `GeoHandle`-free path: e.g. `Consumer` reads a plain `u32` generation outlet. Simplest path that preserves cook-gate coverage: define `TestHandle` in `test_nodes.rs`, register cook source against a `Blob` component on the producer.
- Helpers `port_value` / `recompile` read from components via `field_ops`, not `PortArena`.
- Rename Spatial tests to parenting/Entity tests; expect `ParentFanOut` and `ChildOf` as today.
- Product outlet seeding test → Entity outlet seeding test.
- Disconnect/prefill tests → assert unconnected inlet retains component value after disconnect (last gathered or authored — write the test to set an authored value, connect, tick, disconnect, recompile, assert **last gathered remains** OR reset — **choose last gathered remains** and document).

- [ ] **Step 9: Run sway-graph tests**

Run: `cargo test -p sway-graph`
Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add crates/sway-graph
git commit -m "$(cat <<'EOF'
feat(graph): ports as Inlets/Outlets component values

Drop PortArena, Product, Spatial, and FieldKind. Gather copies fields on
components; Entity-typed edges drive ChildOf; Events still clear via type data.
EOF
)"
```

---

### Task 4: Migrate `sway-geo` (`Grid`, `Displace`)

**Files:**
- Modify: `crates/sway-geo/src/grid.rs`
- Modify: `crates/sway-geo/src/displace.rs`
- Modify: `crates/sway-geo/src/geometry.rs` — drop `Component` derive from `Geometry` if nothing else needs it as a component (it becomes a plain store value). Keep `TypePath` only if still useful; otherwise leave as a normal struct.
- Modify: tests in those files

**Interfaces:**
- Consumes: `GeoHandle`, `GeometryStore`, `register_geo_handle`, component `Outlets`, kindless graph.
- Produces: cooks that `insert`/`update` the store and `ports.write` the `GeoHandle`. **No** `world.entity_mut(node).insert(geo)`. `produced_change_tick` on Grid/Displace can return `None` — consumers watch the handle via cook-source registry. Optionally keep a `GeoHandle` on node state to reuse the same store slot across cooks (`update` in place).

- [ ] **Step 1: Update Grid**

```rust
#[derive(Reflect, Component, Default)]
pub struct GridOutlets {
    pub geo: GeoHandle,
}

#[derive(Component, Default)]
pub struct GridState {
    /// Slot this node owns in GeometryStore; unset until first cook.
    pub handle: GeoHandle,
}

fn register(app: &mut App) {
    register_geo_handle(app);
}

pub type CookFn = fn(&mut World, Entity, &mut PortView);

fn cook(world: &mut World, node: Entity, ports: &mut PortView) {
    // ... build `geo: Geometry` as today ...
    let now = /* current change tick: world.change_tick() or equivalent bevy 0.19 API */;
    let prior = world.get::<GridState>(node).map(|s| s.handle).unwrap_or_default();
    let handle = {
        let mut store = world.resource_mut::<GeometryStore>();
        if prior.is_unset() {
            store.insert(geo, now)
        } else {
            store.update(prior, geo, now).expect("node owns this slot");
            prior
        }
    };
    if let Some(mut state) = world.get_mut::<GridState>(node) {
        state.handle = handle;
    }
    ports.write(Self::OUT_GEO, handle);
}

fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> {
    // Consumers watch the GeoHandle via CookSourceRegistry, not this node.
    None
}
```

Use the real bevy 0.19 API for "now" (`ChangeTick` / `World::change_tick`). If `Tick` cannot be manufactured in tests, store a `u32` revision on the slot and have `change_tick` synthesize or compare revisions inside the cook gate — keep one approach and use it in Task 1's store.

Displace:

```rust
fn cook(world: &mut World, node: Entity, ports: &mut PortView) {
    let input_handle: GeoHandle = ports.read(Self::IN_GEO);
    let input = {
        let store = world.resource::<GeometryStore>();
        store.get(input_handle).cloned()?
    };
    // ... displace into `out: Geometry` ...
    let now = /* change tick */;
    let prior = world.get::<DisplaceState>(node).map(|s| s.handle).unwrap_or_default();
    let handle = {
        let mut store = world.resource_mut::<GeometryStore>();
        if prior.is_unset() {
            store.insert(out, now)
        } else {
            store.update(prior, out, now).expect("node owns this slot");
            prior
        }
    };
    if let Some(mut state) = world.get_mut::<DisplaceState>(node) {
        state.handle = handle;
    }
    ports.write(Self::OUT_GEO, handle);
}
```

- [ ] **Step 2: Fix geo unit tests** that used `Product` / `PortArena` / `world.get::<Geometry>(entity)` — drive the store and mutable `PortView` instead.

- [ ] **Step 3: Run tests**

Run: `cargo test -p sway-graph -p sway-geo`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/sway-geo
git commit -m "$(cat <<'EOF'
feat(geo): Grid and Displace write GeometryStore keys on outlets

Geometry leaves the ECS component path; cooks own store slots and emit GeoHandle.
EOF
)"
```

---

### Task 5: Migrate `sway-nodes` (scene, mesh, material, signals)

**Files:**
- Modify: `crates/sway-nodes/src/scene.rs` — `children: Vec<Entity>`, `GroupOutlets { entity: Entity }` as Component; drop `register_product`
- Modify: `crates/sway-nodes/src/mesh.rs` — `geo: GeoHandle`, `material: Handle<StandardMaterial>` (or `Option<Handle<…>>` if unset must be representable without a default handle), `MeshNodeOutlets { entity: Entity }`; cook reads `GeometryStore` + uses the material handle; `CookFn` mut ports
- Modify: `crates/sway-nodes/src/material.rs` — `StandardMaterialOutlets { material: Handle<StandardMaterial> }`; tick writes the same handle it already keeps in `MaterialState` onto outlets. Drop `MaterialOf` / `register_product`. **No** `MaterialStore` / `MaterialHandle`.
- Modify: signal nodes only as needed for `Outlets: Component` + `Default` derives
- Modify: `crates/sway-nodes/tests/traces.rs` and per-node tests

**Interfaces:**
- Consumes: Task 3 graph + Task 4 geo store + Task 1 `GeoHandle`.
- Produces: all production nodes on the new port model.

- [ ] **Step 1: Group**

```rust
#[derive(Reflect, Component)]
pub struct GroupInlets {
    pub children: Vec<Entity>,
    pub translation: Vec3,
    // rotations, scale unchanged
}

#[derive(Reflect, Component, Default)]
pub struct GroupOutlets {
    pub entity: Entity,
}

fn register(app: &mut App) {
    app.register_type::<Vec3>();
    app.register_type::<Entity>();
}
// ORDINALS: rename "spatial" → "entity" and update const OUT_ENTITY
```

Entity outlet seeding in compile sets `entity` to the node — Group tick does not write it.

- [ ] **Step 2: Material**

```rust
#[derive(Reflect, Component, Default)]
pub struct StandardMaterialOutlets {
    pub material: Handle<StandardMaterial>,
}

#[derive(Component, Default)]
pub struct MaterialState {
    pub handle: Option<Handle<StandardMaterial>>,
}

// in tick, after Assets<StandardMaterial> add/update — same as today —
// also publish onto the outlet:
ports.write(Self::OUT_MATERIAL, handle.clone());
```

Register `Handle<StandardMaterial>` for reflect if needed. Drop `MaterialOf` and every `register_product` call.

If an unconnected material inlet must be distinguishable from a live handle and `Handle::default()` is unsafe to treat as "none", use `Option<Handle<StandardMaterial>>` on both the inlet and the outlet (still no store).

- [ ] **Step 3: MeshNode**

```rust
pub struct MeshNodeInlets {
    pub geo: GeoHandle,
    pub material: Handle<StandardMaterial>, // or Option<Handle<_>>
    // transforms...
}

#[derive(Reflect, Component, Default)]
pub struct MeshNodeOutlets {
    pub entity: Entity,
}

fn cook(world: &mut World, node: Entity, ports: &mut PortView) {
    let material: Handle<StandardMaterial> = ports.read(Self::IN_MATERIAL);
    // MeshMaterial3d as today — compare/insert the asset handle directly

    let geo_handle: GeoHandle = ports.read(Self::IN_GEO);
    let Some(geo) = world.resource::<GeometryStore>().get(geo_handle).cloned() else {
        return;
    };
    // fingerprint / upload as today
}
```

- [ ] **Step 4: Every other node**

Add `#[derive(Component)]` to each `*Outlets` struct. Remove any `register_product`. Keep `register_events`. Ensure `Default` on outlets (Entity defaults to placeholder — seeding overwrites hierarchy outlets; for Events outlets `Default` is empty list).

**Entity::default()** is `Entity::PLACEHOLDER` in bevy 0.19 — fine until seed.

- [ ] **Step 5: Run tests**

Run: `cargo test -p sway-graph -p sway-geo -p sway-nodes`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes
git commit -m "$(cat <<'EOF'
feat(nodes): Entity hierarchy ports; GeoHandle and asset Handle on mesh

Group/Mesh seed Entity outlets. Geometry uses GeometryStore keys;
materials put Bevy's Handle on the wire — no MaterialStore.
EOF
)"
```

---

### Task 6: Migrate `sway-app` demo graph

**Files:**
- Modify: `crates/sway-app/src/demo_graph.rs`
- Modify: any tests spawning Product/Spatial/PortArena

- [ ] **Step 1: Update edge wiring**

Parenting edges: child `OUT_ENTITY` → parent `CHILDREN[i]` with `Entity` types. Geometry edges: `Grid::OUT_GEO` → `Displace::IN_GEO` → `MeshNode::IN_GEO` with `GeoHandle`. Material: material outlet → mesh material inlet.

Remove `PortArena` resize after compile if present — compile no longer returns `slots_len` for an arena. Delete `slots_len` from `CompiledGraph` if it only served the arena.

- [ ] **Step 2: Run**

Run: `cargo test --workspace --exclude sway-editor`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/sway-app
git commit -m "$(cat <<'EOF'
feat(app): demo graph wires Entity and handle ports

Match the component-value port model in the walking scene.
EOF
)"
```

---

### Task 7: Editor — drop `EdgeKind`, read activity from components

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`
- Modify: `crates/sway-editor/src/canvas.rs` (if it matches on `EdgeKind`)
- Modify: `crates/sway-editor/src/test_graph.rs`
- Modify: `crates/sway-editor/src/node_box.rs` only if kinds affect drawing

**Interfaces:**
- Consumes: kindless `FieldSpec`; values on components.
- Produces: `EdgeView` without `kind` (or with a non-persisted local style derived from `slot_type` if canvas needs a colour — prefer computing colour from `TypeId` at draw time without storing `EdgeKind`).

- [ ] **Step 1: Delete `EdgeKind` enum and `edge_kind` helper**

Remove `kind` from `EdgeView`. Update tests `edge_kinds_distinguish_what_an_edge_carries` → delete or replace with "activity is some only for f32" and parenting still nests via `ChildOf`.

- [ ] **Step 2: Activity sampling**

Replace arena slot lookup with reading the source node's `Outlets` component field via `field_ops` + plan metadata:

```rust
fn source_f32(world: &World, plans: &HashMap<Entity, &NodePlan>, source: Entity, field: u16, index: u16) -> Option<f32> {
    let plan = plans.get(&source)?;
    let spec = plan.fields.get(field as usize)?;
    // Outlets start at inlet_field_count
    if (field as usize) < plan.inlet_field_count { return None; }
    // get Outlets as dyn Reflect — needs type erasure: store on NodeTypeEntry a
    // read_outlet_f32 fn, OR use reflect components via AppTypeRegistry.
}
```

Practical approach already in registry spirit: add `OutletF32Fn` only if needed; simpler: keep a small `read_outlet_value` fn pointer on `NodeTypeEntry`:

```rust
pub type ReadOutletFn = fn(&World, Entity, usize /*field_index in Outlets*/, usize /*index*/) -> Option<Box<dyn PartialReflect>>;
```

Or for activity only: `fn try_outlet_f32(...) -> Option<f32>` per node type.

Minimal: in `capture`, for each edge, if target inlet `slot_type == TypeId::of::<f32>()` (from plan.fields), call a registry helper `entry.read_outlet_f32(world, source, outlet_field_index)`. Implement via `world.get::<N::Outlets>` in `read_outlet_f32_of::<N>`.

Canvas: remove `EdgeKind` match arms; use one edge stroke, or map `slot_type` to colour with a local `fn stroke_for(type_id: TypeId) -> Color` that special-cases `TypeId::of::<f32>()`, Events path prefix, `Entity`, else default — **not** an enum stored on the snapshot.

- [ ] **Step 3: Update editor fixtures** (`test_graph.rs`) to Component outlets + Entity/handles as needed.

- [ ] **Step 4: Run**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/sway-editor
git commit -m "$(cat <<'EOF'
feat(editor): drop EdgeKind; sample f32 activity from Outlets

Snapshot no longer classifies edges; styling may key off TypeId locally.
EOF
)"
```

---

### Task 8: Success criteria sweep and clippy

**Files:**
- Grep-driven cleanup across the workspace
- Modify: docs only if a comment still describes Product/PortArena as current

- [ ] **Step 1: Grep for deleted API**

Run:

```bash
rg -n 'FieldKind|EdgeKind|PortArena|register_product|ReflectProduct|ProductAccess|Product<|struct Spatial|PortView::source|last_product_ticks' \
  crates docs/superpowers/specs/2026-08-04-ports-as-component-values-design.md
```

Expected: no matches under `crates/`. Spec may mention the deleted names historically — that is fine. Plans/reports may mention them historically — fine. **No** matches in `crates/**/*.rs`.

Also: `rg -n 'SpatialFanOut|slots_len' crates`

- [ ] **Step 2: Clippy gate**

Run: `cargo clippy -p sway-graph -p sway-geo -p sway-nodes -p sway-editor --all-targets -- -D warnings`
Expected: PASS

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 3: Manual success checklist (from spec §10)**

- [ ] No `FieldKind`, `EdgeKind`, `Product`, `Spatial`, or `PortArena` in the graph crate (or editor snapshot kinds enum)
- [ ] Hierarchy expressible as ordinary graph edges, backed by `ChildOf`
- [ ] Geometry / material flow via handle values with TypeId matching (`GeoHandle` store keys for geometry; Bevy `Handle<StandardMaterial>` on the wire for materials — **no** `MaterialStore`)
- [ ] `Events<T>` ordinary reflected value with type-data clearing
- [ ] Policies only as TypeId / type-data branches in compile and tick
- [ ] `rg 'struct GeoHandle|fn from_node|fn node\(' -n crates/sway-geo` — GeoHandle has no Entity API; `rg MaterialStore|MaterialHandle crates` is empty

- [ ] **Step 4: Commit if any cleanup landed**

```bash
git add -A crates
git commit -m "$(cat <<'EOF'
chore: sweep Product/PortArena leftovers after ports-as-values

Confirm spec success criteria and the milestone clippy gate.
EOF
)"
```

---

## Self-review (plan author)

**Spec coverage**

| Spec section | Task |
|---|---|
| §2 Ports on Inlets/Outlets; no kinds | 3, 7 |
| §2 Delete Product/Spatial/PortArena/FieldKind/EdgeKind/ReflectProduct/… | 3, 7, 8 |
| §3 TypeId policies (Events, Entity, else gather) | 3 |
| §4 FieldSpec without kind; derive_fields | 3 |
| §5 Compile expand/validate/parent/order/emit | 3 (passes renamed to match code) |
| §6 Tick clear/gather/tick/cook; PortView over components | 3 |
| §7 Handles | 1, 4, 5 — `GeoHandle` store keys; material = Bevy `Handle` on the port |
| §9 Out of scope (GPU, RON, MIDI, Events clear still required) | not scheduled |
| §10 Success criteria | 8 |

**Placeholder scan:** none intentional — `CookFn` mutability and cook-source registry vs type-data are decided in Task 2/4 notes.

**Type consistency:** `GeoHandle` index+generation; material ports use `Handle<StandardMaterial>` (or `Option<Handle<_>>`); `ParentFanOut`; `last_source_ticks`; `CopyEdge`; `Outlets: Component`; `CookFn(... &mut PortView)`.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-04-ports-as-component-values.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
