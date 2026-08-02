# Unified Edges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace three edge kinds and five node-declaration mechanisms with one `Edge`, two structs (`Inlets`/`Outlets`), one arena, and one compiled order.

**Architecture:** Every inlet is a typed value slot taking exactly one edge. A field's type decides what it carries: a plain reflect field is a value, `Events<T>` is a list of timestamped occurrences, `Product<T>` holds the source node's `Entity`. Variable fan-in is a `Vec` field, so a node's inlet *count* varies while every inlet's arity stays one. `Product` edges are ordinary value edges, so the tick and cook orders merge into a single topological sort — except `Product<Spatial>` edges, which emit Bevy `ChildOf` and are excluded from ordering.

**Tech Stack:** Rust 2024, bevy 0.19 subcrates (`bevy_ecs`, `bevy_reflect`, `bevy_app`, `bevy_time`, `bevy_transform`, `bevy_math`), masonry (editor).

**Spec:** `docs/superpowers/specs/2026-08-03-unified-edges-design.md`

## Global Constraints

- `sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform` only. **Not** the `bevy` facade, **not** `bevy_render`. The manifest is the only place this is enforced.
- `sway-editor` may depend on `sway-graph`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_transform`. **Not** `bevy`, `bevy_render`, `wgpu`, `vello`, `imaging_vello`.
- Every load failure must produce a clear, node-attributed error message in the vocabulary of what failed (parent spec §2.5). This is asserted by tests, not by inspection.
- The tick is infallible. All validation happens in `compile`.
- Use `reflect_clone()`, never `to_dynamic()`, for any arena value that must later downcast to its concrete type.
- `PhantomData<fn() -> T>` (not `PhantomData<T>`) for generic markers, so they stay `Send + Sync` regardless of `T`.
- Clippy gate for this work: `cargo clippy -p sway-graph -p sway-geo -p sway-nodes -p sway-editor --all-targets -- -D warnings`. `cargo clippy --workspace` was already red on `main` before this milestone; do not attribute pre-existing debt here.
- Measurements, if any are taken, run with `--test-threads=1` and time `graph_tick` directly, never `App::update()`.

## Expected build state during the flip

**Tasks 1–3 are additive: the whole workspace builds and all tests pass.**

**Task 4 replaces the engine.** From the moment Task 4 lands until Task 7 completes, `sway-geo`, `sway-nodes`, `sway-app` and `sway-editor` do not compile. This is expected and bounded. Each task in that window states the exact scoped command that must pass at its end:

| After task | Must pass |
|---|---|
| 1, 2, 3 | `cargo test --workspace` |
| 4 | `cargo test -p sway-graph` |
| 5 | `cargo test -p sway-graph -p sway-geo` |
| 6 | `cargo test -p sway-graph -p sway-geo -p sway-nodes` |
| 7 | `cargo test --workspace --exclude sway-editor` |
| 8, 9 | `cargo test --workspace` |
| 10 | `cargo test --workspace` + the clippy gate above |

Do not try to keep the whole workspace green inside the window; it costs a compatibility shim the spec explicitly rejects.

## File structure

**`crates/sway-graph/src/`**

| File | Responsibility after this work |
|---|---|
| `ports.rs` | `PortArena` (one `Vec<Box<dyn PartialReflect>>`), the three slot value types `Occurrence<T>` / `Events<T>` / `Product<T>`, the `Spatial` capability, `SlotIdx` |
| `schema.rs` | field derivation: `FieldKind`, `FieldSpec`, `derive_fields`, the `ReflectEventList` / `ReflectProduct` type data and their registration helpers |
| `edges.rs` | `NodeId`, `GraphNode`, `NodeRuntime`, `Endpoint`, `Edge`, `EdgeFrom`/`EdgeTo`, `EditorPos` |
| `registry.rs` | the `NodeType` contract, `NodeTypeEntry`, `register_node_type`, the ordinal guard |
| `compile.rs` | one validation pass, one topological sort, `NodePlan`, `CompiledGraph`, `CompileError` |
| `tick.rs` | one loop: clear, gather, prefill, tick, cook |
| `view.rs` | `PortView` — scoped access to values, events and products |
| `test_nodes.rs` | engine-only fixtures |
| ~~`slots.rs`~~ | **deleted** — `Slot<T>`, `ReflectSlot`, `SlotField`, `derive_slots`, `SlotSource` all go |
| ~~`structure.rs`~~ | **deleted** — its validation folds into `compile.rs` |

**Other crates:** `sway-geo` (`grid.rs`, `displace.rs`), `sway-nodes` (all seven node files), `sway-app` (`demo_graph.rs`), `sway-editor` (`snapshot.rs`, `canvas.rs`, `node_box.rs`, `test_graph.rs`).

---

### Task 1: The slot value types

Adds the three types an arena slot can hold, plus the `Spatial` capability marker. Purely additive — nothing consumes them yet, and the old `Event<T>` marker stays until Task 4.

The point of doing this first and alone is that these are the only types in the design whose derive behaviour is unproven. M2a proved a `PhantomData<fn() -> T>` marker derives `Reflect`; it did **not** prove that a struct with an ignored field survives `reflect_clone`, which is what the gather does to every slot on every tick.

**Files:**
- Modify: `crates/sway-graph/src/ports.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Produces: `Occurrence<T> { offset: f32, value: T }`, `Events<T> { occurrences: Vec<Occurrence<T>> }`, `Product<T> { source: Option<Entity> }`, `Spatial`, `SlotIdx(pub u32)`. Task 2 derives field specs from these; Task 4's arena stores them.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `crates/sway-graph/src/ports.rs`:

```rust
    #[test]
    fn a_product_survives_reflect_clone_with_its_source() {
        // The gather clones every slot every tick via reflect_clone. A
        // Product whose `source` did not survive that would silently
        // disconnect every structural edge on the first tick.
        use bevy_ecs::entity::Entity;
        use bevy_reflect::PartialReflect;

        let original = Product::<Spatial>::from_source(Entity::from_raw_u32(7).unwrap());
        let cloned = original
            .reflect_clone()
            .expect("Product must reflect_clone")
            .into_partial_reflect();

        let cloned = cloned
            .try_downcast_ref::<Product<Spatial>>()
            .expect("reflect_clone must preserve the concrete type, not produce a proxy");
        assert_eq!(cloned.source, original.source);
        assert_eq!(cloned.source, Entity::from_raw_u32(7).ok());
    }

    #[test]
    fn events_survive_reflect_clone_with_their_occurrences() {
        let mut original = Events::<u8>::default();
        original.occurrences.push(Occurrence { offset: 0.25, value: 9 });

        let cloned = original
            .reflect_clone()
            .expect("Events must reflect_clone")
            .into_partial_reflect();
        let cloned = cloned
            .try_downcast_ref::<Events<u8>>()
            .expect("reflect_clone must preserve the concrete type");

        assert_eq!(cloned.occurrences.len(), 1);
        assert_eq!(cloned.occurrences[0].offset, 0.25);
        assert_eq!(cloned.occurrences[0].value, 9);
    }

    #[test]
    fn clearing_events_in_place_retains_the_allocation() {
        // Spec §8: this is the one axis where a merged arena can be worse
        // than the split one. Clearing must empty the existing Vec, never
        // replace the value with a fresh Events::default().
        let mut events = Events::<u8>::default();
        for i in 0..16 {
            events.occurrences.push(Occurrence { offset: i as f32, value: i });
        }
        let capacity = events.occurrences.capacity();

        let mut boxed: Box<dyn bevy_reflect::PartialReflect> = Box::new(events);
        clear_events_of::<u8>(&mut *boxed);

        let cleared = boxed.try_downcast_ref::<Events<u8>>().expect("still Events<u8>");
        assert!(cleared.occurrences.is_empty());
        assert!(
            cleared.occurrences.capacity() >= capacity,
            "clear must retain the buffer, not reallocate"
        );
    }

    #[test]
    fn an_unset_product_is_none() {
        assert_eq!(Product::<Spatial>::default().source, None);
    }
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p sway-graph --lib ports::`
Expected: FAIL to compile — `cannot find type Product`, `cannot find type Events`, `cannot find function clear_events_of`.

- [ ] **Step 3: Rename the existing `Occurrence` out of the way**

`ports.rs` already has a non-generic `Occurrence { offset: f32, value: Box<dyn PartialReflect> }`, which the new generic `Occurrence<T>` would collide with. The old one is still in use until Task 4 deletes it, so rename it rather than removing it.

Rename `Occurrence` to `BoxedOccurrence` at every site. There are three files and the rename is mechanical:

```bash
sed -i '' 's/\bOccurrence\b/BoxedOccurrence/g' \
  crates/sway-graph/src/ports.rs \
  crates/sway-graph/src/view.rs \
  crates/sway-graph/src/tick.rs
```

Then in `crates/sway-graph/src/lib.rs`, change `Occurrence` to `BoxedOccurrence` in the `pub use ports::{...}` list if it appears there.

Update the doc comment on the renamed struct to say what it is now:

```rust
/// One event occurrence in the pre-unification arena, with a boxed payload.
///
/// Deleted in the same change that replaces `PortArena::events`; the typed
/// `Occurrence<T>` below is its replacement.
pub struct BoxedOccurrence {
```

Run: `cargo test -p sway-graph --lib` and expect the pre-existing tests to still pass (the new ones from step 1 still fail to compile — that is the next step's job).

- [ ] **Step 4: Add the types**

In `crates/sway-graph/src/ports.rs`, add these imports at the top of the file alongside the existing ones:

```rust
use bevy_ecs::entity::Entity;
use bevy_reflect::{FromReflect, GetTypeRegistration, Typed};
```

Then add the types, after the existing `Event<T>` definition:

```rust
/// The capability a scene node produces and a `children` inlet accepts.
///
/// The engine knows this one capability by name, because Bevy owns the scene
/// hierarchy: an edge into a `Product<Spatial>` inlet also emits `ChildOf`,
/// a `Product<Spatial>` outlet may feed at most one inlet, and `Spatial`
/// edges are excluded from the compiled order (design §3).
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct Spatial;

/// One event occurrence, stamped with its offset inside the tick window.
///
/// `offset` is seconds from the tick's start, so it is bounded by the
/// timestep (~8.3ms at 120Hz) and f32 has precision to spare.
///
/// Typed rather than boxed: one allocation for the whole [`Events`] list
/// replaces one box per occurrence, which is the allocation M2a identified as
/// the tick's dominant cost.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct Occurrence<T> {
    pub offset: f32,
    pub value: T,
}

/// An event port's value: the occurrences that landed this tick.
///
/// Empty means "nothing arrived", which is what distinguishes it from a
/// continuous value of zero (parent §2.4). Emptied before every tick by the
/// runner, in place, through [`clear_events_of`].
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct Events<T> {
    pub occurrences: Vec<Occurrence<T>>,
}

impl<T> Default for Events<T> {
    // Not derived: `#[derive(Default)]` would demand `T: Default`, and an
    // empty list needs nothing from `T`.
    fn default() -> Self {
        Self { occurrences: Vec::new() }
    }
}

/// A structural port's value: the entity that produces capability `T`.
///
/// The produced data itself never enters the arena — only this reference does
/// — so parent §2.1's rule that high-cardinality data lives in the ECS is
/// untouched. `None` is an unconnected inlet, which is also its authored
/// value, so the shadowing rule of parent §2.11 needs no special case here.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Product<T: TypePath + Send + Sync + 'static> {
    pub source: Option<Entity>,
    #[reflect(ignore, clone)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: TypePath + Send + Sync + 'static> Default for Product<T> {
    fn default() -> Self {
        Self { source: None, _marker: PhantomData }
    }
}

impl<T: TypePath + Send + Sync + 'static> Product<T> {
    pub fn from_source(source: Entity) -> Self {
        Self { source: Some(source), _marker: PhantomData }
    }
}

/// Empties an `Events<T>` slot **in place**, keeping its allocation.
///
/// Registered per payload type as a fn pointer (Task 2's `ReflectEventList`)
/// so the runner can clear a slot without knowing `T`. Replacing the value
/// with a fresh `Events::default()` would be correct and would also throw
/// away the buffer every tick — see the test above.
pub fn clear_events_of<T>(value: &mut dyn PartialReflect)
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    if let Some(events) = value.try_downcast_mut::<Events<T>>() {
        events.occurrences.clear();
    }
}

/// Absolute index into [`PortArena`]'s slots.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct SlotIdx(pub u32);
```

If `#[reflect(ignore, clone)]` is rejected by the derive, the requirement the
test states still stands: `Product<T>` must `reflect_clone` into a real
`Product<T>` carrying its `source`. The next thing to try is dropping
`_marker` entirely and making `Product<T>` a unit-like struct with the
capability tracked only by its `TypePath` — but do not do that speculatively;
only if the attribute fails.

- [ ] **Step 5: Export the new types**

In `crates/sway-graph/src/lib.rs`, find the `pub use ports::{...}` line and add the new names to it:

```rust
pub use ports::{
    clear_events_of, BoxedOccurrence, ContinuousIdx, Event, EventIdx, Events, Occurrence,
    PortArena, Product, SlotIdx, Spatial,
};
```

Keep every name that is already exported; this list adds `clear_events_of`, `Events`, `Occurrence`, `Product`, `SlotIdx` and `Spatial`, and carries the step 3 rename.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-graph --lib ports::`
Expected: PASS, including the three existing arena tests.

- [ ] **Step 7: Verify nothing else regressed**

Run: `cargo test --workspace`
Expected: PASS. Task 1 is purely additive.

- [ ] **Step 8: Commit**

```bash
git add crates/sway-graph/src/ports.rs crates/sway-graph/src/view.rs \
  crates/sway-graph/src/tick.rs crates/sway-graph/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(graph): the three slot value types

Occurrence<T> is typed rather than boxed, so one allocation for a whole
Events<T> list replaces one box per occurrence. Product<T> carries the source
entity and nothing else -- the produced data stays on the source's components.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Field derivation

Derives a node's connectivity from its `Inlets`/`Outlets` types: which fields exist, what each carries, whether it is variadic, and the fn pointers the engine needs to touch a slot without knowing its payload type.

Still additive — `derive_schema` and `derive_slots` stay until Task 4 deletes them.

**Files:**
- Modify: `crates/sway-graph/src/schema.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `Events<T>`, `Product<T>`, `clear_events_of` from Task 1.
- Produces: `FieldKind`, `ProductAccess`, `FieldSpec`, `derive_fields(&TypeRegistry) -> Result<Vec<FieldSpec>, SchemaError>`, `register_events::<T>(app)`, `register_product::<T>(app)`. Task 4's registry stores the `Vec<FieldSpec>`; Task 4's compiler reads `kind`, `variadic` and `slot_type`.

- [ ] **Step 1: Write the failing test**

Replace the entire `#[cfg(test)] mod tests` block at the bottom of `crates/sway-graph/src/schema.rs` with this — it keeps the four existing `derive_schema` tests and adds the new ones:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{Event, Events, Product, Spatial};
    use bevy_reflect::{Reflect, TypeRegistry};

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg {
        note: u8,
        velocity: u8,
    }

    #[derive(Reflect, TypePath)]
    struct Geometry;

    #[derive(Reflect, Default)]
    struct MixedParams {
        hz: f32,
        trigger: Event<NoteMsg>,
        amplitude: f32,
    }

    fn registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<MixedParams>();
        r.register::<Event<NoteMsg>>();
        r.register_type_data::<Event<NoteMsg>, ReflectEventPort>();
        r
    }

    #[test]
    fn field_type_decides_port_kind_and_ordinals_are_per_kind() {
        let s = derive_schema::<MixedParams>(&registry()).expect("schema");

        assert_eq!(
            s.continuous.iter().map(|f| f.name).collect::<Vec<_>>(),
            vec!["hz", "amplitude"]
        );
        assert_eq!(s.continuous[1].field_index, 2);
        assert_eq!(s.events.iter().map(|f| f.name).collect::<Vec<_>>(), vec!["trigger"]);
    }

    #[test]
    fn event_port_carries_its_payload_type_not_the_marker_type() {
        let s = derive_schema::<MixedParams>(&registry()).expect("schema");
        assert_eq!(s.events[0].type_id, core::any::TypeId::of::<NoteMsg>());
        assert_ne!(s.events[0].type_id, core::any::TypeId::of::<Event<NoteMsg>>());
    }

    #[test]
    fn an_unregistered_event_field_is_an_error_not_a_continuous_port() {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<MixedParams>();
        r.register::<Event<NoteMsg>>();

        let err = derive_schema::<MixedParams>(&r).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("trigger"), "message must name the field: {msg}");
        assert!(msg.contains("register_event_port"), "message must say the fix: {msg}");
    }

    #[test]
    fn a_non_struct_params_type_is_rejected() {
        let mut r = TypeRegistry::new();
        r.register::<f32>();
        let err = derive_schema::<f32>(&r).unwrap_err();
        assert!(err.to_string().contains("struct"), "{err}");
    }

    // --- derive_fields ------------------------------------------------

    #[derive(Reflect, Default)]
    struct SampleInlets {
        children: Vec<Product<Spatial>>,
        geo: Product<Geometry>,
        triggers: Vec<Events<NoteMsg>>,
        gain: f32,
        terms: Vec<f32>,
    }

    fn fields_registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register_type_data::<Events<NoteMsg>, ReflectEventList>();
        r.register::<Product<Spatial>>();
        r.register_type_data::<Product<Spatial>, ReflectProduct>();
        r.register::<Product<Geometry>>();
        r.register_type_data::<Product<Geometry>, ReflectProduct>();
        r
    }

    #[test]
    fn each_field_kind_is_derived_from_its_type() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        assert_eq!(
            fields.iter().map(|f| f.name).collect::<Vec<_>>(),
            vec!["children", "geo", "triggers", "gain", "terms"]
        );
        assert!(matches!(fields[0].kind, FieldKind::Product { .. }));
        assert!(matches!(fields[1].kind, FieldKind::Product { .. }));
        assert!(matches!(fields[2].kind, FieldKind::Events { .. }));
        assert!(matches!(fields[3].kind, FieldKind::Value));
        assert!(matches!(fields[4].kind, FieldKind::Value));
    }

    #[test]
    fn a_vec_field_is_variadic_and_reports_its_element_type() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        // The slot type is the ELEMENT type for a Vec field. Edge validation
        // compares slot types, and an edge connects to one element.
        assert!(fields[0].variadic, "children is Vec<_>");
        assert_eq!(fields[0].slot_type, core::any::TypeId::of::<Product<Spatial>>());
        assert!(!fields[1].variadic, "geo is a bare Product<_>");
        assert_eq!(fields[1].slot_type, core::any::TypeId::of::<Product<Geometry>>());
        assert!(fields[4].variadic, "terms is Vec<f32>");
        assert_eq!(fields[4].slot_type, core::any::TypeId::of::<f32>());
    }

    #[test]
    fn a_product_field_carries_its_capability_and_its_spatial_flag() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        let FieldKind::Product { capability, spatial, .. } = fields[0].kind else {
            panic!("children must be a Product field");
        };
        assert_eq!(capability, core::any::TypeId::of::<Spatial>());
        assert!(spatial, "Spatial is the one capability the engine acts on");

        let FieldKind::Product { capability, spatial, .. } = fields[1].kind else {
            panic!("geo must be a Product field");
        };
        assert_eq!(capability, core::any::TypeId::of::<Geometry>());
        assert!(!spatial);
    }

    #[test]
    fn product_access_reads_and_writes_a_source_without_knowing_the_capability() {
        // This is what lets the runner seed a Product outlet and the cook
        // read a Product inlet through `dyn PartialReflect` alone.
        use bevy_ecs::entity::Entity;
        use bevy_reflect::PartialReflect;

        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");
        let FieldKind::Product { access, .. } = fields[1].kind else {
            panic!("geo must be a Product field");
        };

        let mut slot: Box<dyn PartialReflect> = Box::new(Product::<Geometry>::default());
        assert_eq!((access.get)(&*slot), None);

        let entity = Entity::from_raw_u32(11).unwrap();
        (access.set)(&mut *slot, Some(entity));
        assert_eq!((access.get)(&*slot), Some(entity));
    }

    #[test]
    fn an_events_field_carries_its_clear_fn() {
        use bevy_reflect::PartialReflect;

        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");
        let FieldKind::Events { clear, .. } = fields[2].kind else {
            panic!("triggers must be an Events field");
        };

        let mut events = Events::<NoteMsg>::default();
        events.occurrences.push(Occurrence {
            offset: 0.0,
            value: NoteMsg { note: 60, velocity: 100 },
        });
        let mut slot: Box<dyn PartialReflect> = Box::new(events);

        clear(&mut *slot);

        assert!(slot
            .try_downcast_ref::<Events<NoteMsg>>()
            .expect("still Events")
            .occurrences
            .is_empty());
    }

    #[test]
    fn an_unregistered_events_field_is_an_error_not_a_value_port() {
        // The failure this prevents: a node author adds an Events<T> field,
        // forgets register_events, and it silently becomes a value port that
        // is never cleared -- so an occurrence fires forever.
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register::<Product<Spatial>>();
        r.register_type_data::<Product<Spatial>, ReflectProduct>();
        r.register::<Product<Geometry>>();
        r.register_type_data::<Product<Geometry>, ReflectProduct>();

        let msg = derive_fields::<SampleInlets>(&r).unwrap_err().to_string();
        assert!(msg.contains("triggers"), "must name the field: {msg}");
        assert!(msg.contains("register_events"), "must say the fix: {msg}");
    }

    #[test]
    fn an_unregistered_product_field_is_an_error_not_a_value_port() {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register_type_data::<Events<NoteMsg>, ReflectEventList>();
        r.register::<Product<Spatial>>();
        r.register::<Product<Geometry>>();

        let msg = derive_fields::<SampleInlets>(&r).unwrap_err().to_string();
        assert!(msg.contains("children"), "must name the field: {msg}");
        assert!(msg.contains("register_product"), "must say the fix: {msg}");
    }
}
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p sway-graph --lib schema::`
Expected: FAIL to compile — `cannot find type FieldKind`, `ReflectEventList`, `ReflectProduct`, `derive_fields`.

- [ ] **Step 3: Add the type data and their registration helpers**

In `crates/sway-graph/src/schema.rs`, replace the import block at the top with:

```rust
use core::any::TypeId;
use core::fmt;

use bevy_app::App;
use bevy_ecs::entity::Entity;
use bevy_reflect::structs::StructInfo;
use bevy_reflect::{
    FromReflect, FromType, GetTypeRegistration, PartialReflect, Reflect, TypeInfo, TypePath,
    TypeRegistry, Typed,
};

use crate::ports::{clear_events_of, Event, Events, Occurrence, Product, Spatial};
```

Then add, immediately after the existing `register_event_port` function:

```rust
/// Type data marking a type as an `Events<T>` slot value, carrying the payload
/// type and the fn that empties one in place.
#[derive(Clone)]
pub struct ReflectEventList {
    pub payload: TypeId,
    pub payload_path: &'static str,
    pub clear: fn(&mut dyn PartialReflect),
}

impl<T> FromType<Events<T>> for ReflectEventList
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    fn from_type() -> Self {
        Self {
            payload: TypeId::of::<T>(),
            payload_path: T::type_path(),
            clear: clear_events_of::<T>,
        }
    }
}

/// Reads and writes a `Product<T>`'s source through `dyn PartialReflect`, so
/// the engine can seed an outlet and a cook can read an inlet without knowing
/// the capability.
#[derive(Clone, Copy)]
pub struct ProductAccess {
    pub get: fn(&dyn PartialReflect) -> Option<Entity>,
    pub set: fn(&mut dyn PartialReflect, Option<Entity>),
}

/// Type data marking a type as a `Product<T>` slot value.
#[derive(Clone)]
pub struct ReflectProduct {
    pub capability: TypeId,
    pub capability_path: &'static str,
    pub access: ProductAccess,
}

impl<T: TypePath + Send + Sync + 'static> FromType<Product<T>> for ReflectProduct {
    fn from_type() -> Self {
        Self {
            capability: TypeId::of::<T>(),
            capability_path: T::type_path(),
            access: ProductAccess {
                get: |value| {
                    value
                        .try_downcast_ref::<Product<T>>()
                        .and_then(|product| product.source)
                },
                set: |value, source| {
                    if let Some(product) = value.try_downcast_mut::<Product<T>>() {
                        product.source = source;
                    }
                },
            },
        }
    }
}

/// Registers `Events<T>` and its `ReflectEventList` data. A node type with an
/// `Events<T>` field must call this in its `register`.
pub fn register_events<T>(app: &mut App)
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    let mut registry = registry.write();
    registry.register::<T>();
    registry.register::<Occurrence<T>>();
    registry.register::<Events<T>>();
    registry.register_type_data::<Events<T>, ReflectEventList>();
}

/// Registers `Product<T>` and its `ReflectProduct` data. A node type with a
/// `Product<T>` field must call this in its `register`.
pub fn register_product<T>(app: &mut App)
where
    T: TypePath + Send + Sync + 'static,
{
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    let mut registry = registry.write();
    registry.register::<Product<T>>();
    registry.register_type_data::<Product<T>, ReflectProduct>();
}
```

- [ ] **Step 4: Add `FieldKind`, `FieldSpec` and `derive_fields`**

Still in `crates/sway-graph/src/schema.rs`, add after the code from step 3:

```rust
/// What a field's slots carry. Derived from the field's type — for a `Vec`
/// field, from its element type.
#[derive(Clone, Copy)]
pub enum FieldKind {
    /// A plain reflect value: the slot holds it directly.
    Value,
    /// `Events<T>`: the slot holds this tick's occurrences, and is emptied
    /// before every tick through `clear`.
    Events {
        payload: TypeId,
        clear: fn(&mut dyn PartialReflect),
    },
    /// `Product<T>`: the slot holds the producing entity. `spatial` is true
    /// for the one capability the engine acts on (design §3).
    Product {
        capability: TypeId,
        spatial: bool,
        access: ProductAccess,
    },
}

/// One field of an `Inlets` or `Outlets` struct.
#[derive(Clone)]
pub struct FieldSpec {
    pub name: &'static str,
    /// Index of the field in the reflect struct — what `Struct::field_at`
    /// takes. Equal to the field ordinal within its own struct.
    pub field_index: usize,
    pub kind: FieldKind,
    /// The type one *slot* holds: the element type for a `Vec` field, the
    /// field's own type otherwise. Edge validation compares these directly.
    pub slot_type: TypeId,
    pub slot_type_path: &'static str,
    /// A `Vec<_>` field, whose slot count comes from the instance.
    pub variadic: bool,
}

/// Derives one struct's fields. The struct is a node's `Inlets` or `Outlets`;
/// direction comes from which of the two it is, never from the field.
pub fn derive_fields<T: Typed>(registry: &TypeRegistry) -> Result<Vec<FieldSpec>, SchemaError> {
    let s = struct_info::<T>()?;

    let mut fields = Vec::with_capacity(s.field_len());
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");

        // A Vec field is variadic and its slots hold the element type.
        // Anything else holds the field's own type.
        let (slot_type, slot_type_path, variadic) = match field.type_info() {
            Some(TypeInfo::List(list)) => {
                let item = list.item_ty();
                (item.id(), item.path(), true)
            }
            _ => (field.type_id(), field.type_path(), false),
        };

        let kind = if let Some(events) = registry.get_type_data::<ReflectEventList>(slot_type) {
            FieldKind::Events { payload: events.payload, clear: events.clear }
        } else if let Some(product) = registry.get_type_data::<ReflectProduct>(slot_type) {
            FieldKind::Product {
                capability: product.capability,
                spatial: product.capability == TypeId::of::<Spatial>(),
                access: product.access,
            }
        } else {
            // A marker field whose type data is missing would otherwise
            // become a value port of a type no edge can usefully drive —
            // and, for Events, one that is never cleared. Catch it by path.
            if is_events_marker_path(slot_type_path) {
                return Err(SchemaError::UnregisteredEventsField {
                    type_path: s.type_path(),
                    field: field.name(),
                });
            }
            if is_product_marker_path(slot_type_path) {
                return Err(SchemaError::UnregisteredProductField {
                    type_path: s.type_path(),
                    field: field.name(),
                });
            }
            FieldKind::Value
        };

        fields.push(FieldSpec {
            name: field.name(),
            field_index: i,
            kind,
            slot_type,
            slot_type_path,
            variadic,
        });
    }
    Ok(fields)
}

/// Recognises `sway_graph::ports::Events<..>` by path. The authoritative test
/// is the `ReflectEventList` type data; this is the diagnostic for its absence.
fn is_events_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Events<")
}

/// The same, for `Product<..>`.
fn is_product_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Product<")
}
```

- [ ] **Step 5: Add the two new error variants**

In `crates/sway-graph/src/schema.rs`, add these variants to `enum SchemaError`:

```rust
    UnregisteredEventsField {
        type_path: &'static str,
        field: &'static str,
    },
    UnregisteredProductField {
        type_path: &'static str,
        field: &'static str,
    },
```

and these arms to its `Display` impl:

```rust
            Self::UnregisteredEventsField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like an event list but its type is not \
                 registered as one — call `register_events::<Payload>(app)` in this node \
                 type's `register`"
            ),
            Self::UnregisteredProductField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like a product but its type is not \
                 registered as one — call `register_product::<Capability>(app)` in this \
                 node type's `register`"
            ),
```

- [ ] **Step 6: Export the new items**

In `crates/sway-graph/src/lib.rs`, find the `pub use schema::{...}` line and add the new names:

```rust
pub use schema::{
    derive_fields, derive_schema, register_event_port, register_events, register_product,
    FieldKind, FieldSpec, PortField, ProductAccess, ReflectEventList, ReflectEventPort,
    ReflectProduct, SchemaError, SchemaHalf,
};
```

Keep every name already exported; this adds `derive_fields`, `register_events`, `register_product`, `FieldKind`, `FieldSpec`, `ProductAccess`, `ReflectEventList` and `ReflectProduct`.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p sway-graph --lib schema::`
Expected: PASS — four pre-existing tests plus seven new ones.

If `field.type_info()` returns `None` for `Vec<f32>`, the type is not registered in the *reflect* sense the call needs; the fix is to keep the `match` as written and register the field's type in the test registry, not to reach for `TypeRegistry::get_type_info`.

- [ ] **Step 8: Verify nothing else regressed**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/sway-graph/src/schema.rs crates/sway-graph/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(graph): derive a node's fields, kinds and arity from its types

One derivation for what used to be two: ports came from Params/Outputs and
slots from a separate Slots struct with its own capability system. A field's
type now decides what it carries, and a Vec field decides that its slot count
comes from the instance.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `Endpoint` and `Edge`

Adds the one edge component. Still additive: `ParamEdge`, `FeedsEdge` and `ParentEdge` stay until Task 4 deletes them.

**Files:**
- Modify: `crates/sway-graph/src/edges.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Produces: `Endpoint { field: u16, index: u16 }`, `Endpoint::field(u16)`, `Edge { from: Endpoint, to: Endpoint }`. Task 4's compiler queries `(&Edge, &EdgeFrom, &EdgeTo)`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `crates/sway-graph/src/edges.rs`:

```rust
    #[test]
    fn an_edge_addresses_an_element_of_a_field() {
        use super::{Edge, Endpoint};

        let edge = Edge {
            from: Endpoint::field(2),
            to: Endpoint { field: 0, index: 3 },
        };

        // A non-Vec field is element 0 of itself, so one addressing scheme
        // covers both cases and callers never branch on variadic-ness.
        assert_eq!(edge.from.index, 0);
        assert_eq!(edge.to.field, 0);
        assert_eq!(edge.to.index, 3);
    }
```

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p sway-graph --lib edges::`
Expected: FAIL to compile — `cannot find type Endpoint`.

- [ ] **Step 3: Add the types**

In `crates/sway-graph/src/edges.rs`, add after the `NodeRuntime` definition:

```rust
/// One end of an edge: a field ordinal and, for a `Vec` field, which element.
///
/// Addressing by `(field, index)` rather than a flat ordinal is what makes a
/// `Vec` resize local — inserting a child renumbers nothing outside that
/// field, so authored edges and editor widget identity survive it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Endpoint {
    /// Ordinal within the node's fields: inlets first, then outlets.
    pub field: u16,
    /// Element within a `Vec` field. Always 0 for a non-`Vec` field.
    pub index: u16,
}

impl Endpoint {
    /// The single slot of a non-`Vec` field.
    pub fn field(field: u16) -> Self {
        Self { field, index: 0 }
    }
}

/// The one edge. Carries nothing but its two endpoints — what an edge *does*
/// is decided by the type of the inlet it lands on (design §2).
///
/// An entity, so Bevy maintains the reverse index and `linked_spawn` on
/// `EdgeFrom`/`EdgeTo` makes despawning a node despawn its edges.
#[derive(Component, Clone, Copy, Debug)]
pub struct Edge {
    pub from: Endpoint,
    pub to: Endpoint,
}
```

- [ ] **Step 4: Export the new types**

In `crates/sway-graph/src/lib.rs`, add `Edge` and `Endpoint` to the `pub use edges::{...}` list, keeping every existing name.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p sway-graph --lib edges::`
Expected: PASS.

- [ ] **Step 6: Verify nothing else regressed**

Run: `cargo test --workspace`
Expected: PASS. This is the last task before the flip; the workspace must be green here.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-graph/src/edges.rs crates/sway-graph/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(graph): Endpoint and the one Edge component

(field, index) addressing keeps a Vec resize local: inserting an element
renumbers nothing outside its own field.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: The engine flip

Replaces the node contract, the arena, the view, the compiler and the runner in one change, and deletes `slots.rs` and `structure.rs`. This is atomic on purpose: `NodeType` is implemented by every node, the arena shape is what `PortView` reads, and the compiler is what fills it. Splitting it would require a compatibility shim the spec rejects.

**After this task `cargo test -p sway-graph` passes and nothing else in the workspace compiles.** Tasks 5–7 close that.

**Files:**
- Rewrite: `crates/sway-graph/src/ports.rs` (arena only — the Task 1 types stay)
- Rewrite: `crates/sway-graph/src/view.rs`
- Rewrite: `crates/sway-graph/src/registry.rs`
- Rewrite: `crates/sway-graph/src/compile.rs`
- Rewrite: `crates/sway-graph/src/tick.rs`
- Rewrite: `crates/sway-graph/src/test_nodes.rs`
- Delete: `crates/sway-graph/src/slots.rs`, `crates/sway-graph/src/structure.rs`
- Modify: `crates/sway-graph/src/edges.rs`, `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `Events<T>`, `Product<T>`, `Spatial`, `clear_events_of` (Task 1); `FieldKind`, `FieldSpec`, `ProductAccess`, `derive_fields`, `register_events`, `register_product` (Task 2); `Edge`, `Endpoint` (Task 3).
- Produces, for Tasks 5–9:
  - `NodeType { type Inlets; type Outlets; type State; const ORDINALS; const COOKS; fn register; fn tick; fn cook; fn produced_change_tick }`
  - `PortView::read<T>(field: u16) -> T`, `read_at<T>(field, index) -> T`, `write<T>(field, value)`, `write_at<T>(field, index, value)`, `len(field) -> usize`, `events<T>(field) -> &[Occurrence<T>]`, `events_at<T>(field, index) -> &[Occurrence<T>]`, `emit<T>(field, offset, value)`, `source(field, index) -> Option<Entity>`
  - `NodePlan { entity, node_type, fields, inlet_field_count, base, field_offsets, field_lens, inlet_slots, connected, copies, product_inlets }`
  - `CompiledGraph { plans, slots_len, clears, plan_index_of }`
  - `CompileError` with variants `UnknownNodeType`, `MissingEndpoint`, `FieldOutOfRange`, `ElementOutOfRange`, `WrongDirection`, `TypeMismatch`, `InletAlreadyConnected`, `SpatialFanOut`, `ParentCycle`, `Cycle`

- [ ] **Step 1: Rewrite the engine test fixtures**

These define what the rest of the task must satisfy, so they come first. Replace `crates/sway-graph/src/test_nodes.rs` entirely:

```rust
//! Engine-only node fixtures. Deliberately not real nodes: these exist to
//! exercise the contract, not to do anything musical.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::{Reflect, TypePath};

use crate::compile::{compile, CompiledGraph};
use crate::edges::{Edge, EdgeFrom, EdgeTo, Endpoint, GraphNode, NodeId};
use crate::ports::{Events, PortArena, Product, Spatial};
use crate::registry::{register_node_type, NodeType, NodeTypeId, NodeTypeRegistry};
use crate::schema::{register_events, register_product};
use crate::tick::GraphPlugin;
use crate::view::{PortView, TickCtx};

/// A capability no real node uses, for slot-typing tests.
#[derive(Reflect, TypePath, Default)]
pub struct Blob;

/// A second one, so a mismatch names two real capabilities.
#[derive(Reflect, TypePath, Default)]
pub struct Sludge;

#[derive(Reflect, Default, Debug, Clone, PartialEq)]
pub struct Ping {
    pub seq: u32,
}

// --- Gain: two value inlets, one value outlet -------------------------

#[derive(Reflect, Component, Default)]
pub struct GainInlets {
    pub gain: f32,
    pub bias: f32,
}

#[derive(Reflect, Default)]
pub struct GainOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct GainState;

pub struct Gain;

impl Gain {
    pub const GAIN: u16 = 0;
    pub const BIAS: u16 = 1;
    pub const OUT_VALUE: u16 = 2; // outlets follow inlets in one field space
}

impl NodeType for Gain {
    type Inlets = GainInlets;
    type Outlets = GainOutlets;
    type State = GainState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("gain", Gain::GAIN), ("bias", Gain::BIAS), ("value", Gain::OUT_VALUE)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let gain: f32 = ports.read(Gain::GAIN);
        let bias: f32 = ports.read(Gain::BIAS);
        ports.write(Gain::OUT_VALUE, gain * bias);
    }
}

// --- Sum: one variadic value inlet ------------------------------------

#[derive(Reflect, Component, Default)]
pub struct SumInlets {
    pub terms: Vec<f32>,
}

#[derive(Reflect, Default)]
pub struct SumOutlets {
    pub total: f32,
}

#[derive(Component, Default)]
pub struct SumState;

pub struct Sum;

impl Sum {
    pub const TERMS: u16 = 0;
    pub const OUT_TOTAL: u16 = 1;
}

impl NodeType for Sum {
    type Inlets = SumInlets;
    type Outlets = SumOutlets;
    type State = SumState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("terms", Sum::TERMS), ("total", Sum::OUT_TOTAL)];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _t: &TickCtx) {
        // The combining rule lives here, in the node, not in the engine.
        let mut total = 0.0;
        for i in 0..ports.len(Sum::TERMS) {
            total += ports.read_at::<f32>(Sum::TERMS, i as u16);
        }
        ports.write(Sum::OUT_TOTAL, total);
    }
}

// --- Emitter / Sink: event out, event in ------------------------------

#[derive(Reflect, Component, Default)]
pub struct EmitterInlets {
    pub period: f32,
}

#[derive(Reflect, Default)]
pub struct EmitterOutlets {
    pub pulse: Events<Ping>,
}

#[derive(Component, Default)]
pub struct EmitterState {
    pub seq: u32,
}

pub struct Emitter;

impl Emitter {
    pub const PERIOD: u16 = 0;
    pub const OUT_PULSE: u16 = 1;
}

impl NodeType for Emitter {
    type Inlets = EmitterInlets;
    type Outlets = EmitterOutlets;
    type State = EmitterState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("period", Emitter::PERIOD), ("pulse", Emitter::OUT_PULSE)];

    fn register(app: &mut App) {
        register_events::<Ping>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let offset = ports.read::<f32>(Emitter::PERIOD);
        let seq = {
            let mut state = world.get_mut::<EmitterState>(node).expect("state");
            state.seq += 1;
            state.seq
        };
        ports.emit(Emitter::OUT_PULSE, offset, Ping { seq });
    }
}

#[derive(Reflect, Component, Default)]
pub struct SinkInlets {
    pub pulse: Events<Ping>,
}

#[derive(Reflect, Default)]
pub struct SinkOutlets {}

#[derive(Component, Default)]
pub struct SinkState;

pub struct Sink;

impl Sink {
    pub const PULSE: u16 = 0;
}

impl NodeType for Sink {
    type Inlets = SinkInlets;
    type Outlets = SinkOutlets;
    type State = SinkState;

    const ORDINALS: &'static [(&'static str, u16)] = &[("pulse", Sink::PULSE)];

    fn register(app: &mut App) {
        register_events::<Ping>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

// --- Producer / Consumer: Product edges and the cook ------------------

#[derive(Reflect, Component, Default)]
pub struct ProducerInlets {
    pub scale: f32,
}

#[derive(Reflect, Default)]
pub struct ProducerOutlets {
    pub blob: Product<Blob>,
}

#[derive(Component, Default)]
pub struct ProducerState {
    pub cooks: u32,
}

pub struct Producer;

impl Producer {
    pub const SCALE: u16 = 0;
    pub const OUT_BLOB: u16 = 1;
}

impl NodeType for Producer {
    type Inlets = ProducerInlets;
    type Outlets = ProducerOutlets;
    type State = ProducerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("scale", Producer::SCALE), ("blob", Producer::OUT_BLOB)];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _ports: &PortView) {
        world.get_mut::<ProducerState>(node).expect("state").cooks += 1;
    }
}

/// Produces `Sludge`, so a mismatch names two real capabilities.
#[derive(Reflect, Default)]
pub struct SludgeOutlets {
    pub sludge: Product<Sludge>,
}

pub struct SludgeSource;

impl SludgeSource {
    pub const SCALE: u16 = 0;
    pub const OUT_SLUDGE: u16 = 1;
}

impl NodeType for SludgeSource {
    type Inlets = ProducerInlets;
    type Outlets = SludgeOutlets;
    type State = ProducerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("scale", SludgeSource::SCALE), ("sludge", SludgeSource::OUT_SLUDGE)];

    fn register(app: &mut App) {
        register_product::<Sludge>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

#[derive(Reflect, Component, Default)]
pub struct ConsumerInlets {
    pub input: Product<Blob>,
    pub scale: f32,
}

#[derive(Reflect, Default)]
pub struct ConsumerOutlets {
    pub blob: Product<Blob>,
}

#[derive(Component, Default)]
pub struct ConsumerState {
    pub cooks: u32,
}

pub struct Consumer;

impl Consumer {
    pub const INPUT: u16 = 0;
    pub const SCALE: u16 = 1;
    pub const OUT_BLOB: u16 = 2;
}

impl NodeType for Consumer {
    type Inlets = ConsumerInlets;
    type Outlets = ConsumerOutlets;
    type State = ConsumerState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("input", Consumer::INPUT),
        ("scale", Consumer::SCALE),
        ("blob", Consumer::OUT_BLOB),
    ];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Blob>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _ports: &PortView) {
        world.get_mut::<ConsumerState>(node).expect("state").cooks += 1;
    }
}

// --- Group: a variadic Spatial inlet and a Spatial outlet -------------

#[derive(Reflect, Component, Default)]
pub struct GroupInlets {
    pub children: Vec<Product<Spatial>>,
    pub rotation_y: f32,
}

#[derive(Reflect, Default)]
pub struct GroupOutlets {
    pub spatial: Product<Spatial>,
}

#[derive(Component, Default)]
pub struct GroupState;

pub struct Group;

impl Group {
    pub const CHILDREN: u16 = 0;
    pub const ROTATION_Y: u16 = 1;
    pub const OUT_SPATIAL: u16 = 2;
}

impl NodeType for Group {
    type Inlets = GroupInlets;
    type Outlets = GroupOutlets;
    type State = GroupState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("children", Group::CHILDREN),
        ("rotation_y", Group::ROTATION_Y),
        ("spatial", Group::OUT_SPATIAL),
    ];

    fn register(app: &mut App) {
        register_product::<Spatial>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}
}

// --- Helpers ----------------------------------------------------------

pub fn engine_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.add_plugins(GraphPlugin);
    register_node_type::<Gain>(&mut app);
    register_node_type::<Sum>(&mut app);
    register_node_type::<Emitter>(&mut app);
    register_node_type::<Sink>(&mut app);
    register_node_type::<Producer>(&mut app);
    register_node_type::<SludgeSource>(&mut app);
    register_node_type::<Consumer>(&mut app);
    register_node_type::<Group>(&mut app);
    app
}

fn type_id_of<N: NodeType>(world: &World) -> NodeTypeId {
    world
        .resource::<NodeTypeRegistry>()
        .id_of(core::any::type_name::<N>())
        .expect("node type registered")
}

fn next_id(world: &mut World) -> NodeId {
    let mut query = world.query::<&GraphNode>();
    NodeId(query.iter(world).count() as u32)
}

pub fn spawn_gain(world: &mut World, gain: f32, bias: f32) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Gain>(world);
    world
        .spawn((GraphNode { id, node_type }, GainInlets { gain, bias }, GainState))
        .id()
}

pub fn spawn_sum(world: &mut World, terms: Vec<f32>) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Sum>(world);
    world
        .spawn((GraphNode { id, node_type }, SumInlets { terms }, SumState))
        .id()
}

pub fn spawn_emitter(world: &mut World, period: f32) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Emitter>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            EmitterInlets { period },
            EmitterState::default(),
        ))
        .id()
}

pub fn spawn_sink(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Sink>(world);
    world
        .spawn((GraphNode { id, node_type }, SinkInlets::default(), SinkState))
        .id()
}

pub fn spawn_producer(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Producer>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ProducerInlets::default(),
            ProducerState::default(),
        ))
        .id()
}

pub fn spawn_sludge_source(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<SludgeSource>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ProducerInlets::default(),
            ProducerState::default(),
        ))
        .id()
}

pub fn spawn_consumer(world: &mut World) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Consumer>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            ConsumerInlets::default(),
            ConsumerState::default(),
        ))
        .id()
}

/// `children` is sized here, because a variadic field's slot count comes
/// from the instance.
pub fn spawn_group(world: &mut World, children: usize) -> Entity {
    let id = next_id(world);
    let node_type = type_id_of::<Group>(world);
    world
        .spawn((
            GraphNode { id, node_type },
            GroupInlets {
                children: vec![Product::<Spatial>::default(); children],
                rotation_y: 0.0,
            },
            GroupState,
        ))
        .id()
}

pub fn connect(world: &mut World, from: Entity, from_field: u16, to: Entity, to_field: u16) -> Entity {
    connect_at(world, from, from_field, to, to_field, 0)
}

pub fn connect_at(
    world: &mut World,
    from: Entity,
    from_field: u16,
    to: Entity,
    to_field: u16,
    to_index: u16,
) -> Entity {
    world
        .spawn((
            Edge {
                from: Endpoint::field(from_field),
                to: Endpoint { field: to_field, index: to_index },
            },
            EdgeFrom(from),
            EdgeTo(to),
        ))
        .id()
}

pub fn recompile(app: &mut App) {
    let compiled = compile(app.world_mut()).expect("compiles");
    let slots_len = compiled.slots_len;
    app.world_mut().resource_mut::<PortArena>().resize(slots_len);
    app.world_mut().insert_resource(compiled);
}

/// Reads a node's value slot out of the arena, by field ordinal.
pub fn port_value(app: &App, node: Entity, field: u16) -> f32 {
    let compiled = app.world().resource::<CompiledGraph>();
    let plan = compiled
        .plans
        .iter()
        .find(|p| p.entity == node)
        .expect("node is compiled");
    let slot = plan.base + plan.field_offsets[field as usize];
    app.world().resource::<PortArena>().values[slot]
        .try_downcast_ref::<f32>()
        .copied()
        .expect("slot holds an f32")
}

/// The occurrences on a node's event slot this tick.
pub fn event_offsets(app: &App, node: Entity, field: u16) -> Vec<f32> {
    let compiled = app.world().resource::<CompiledGraph>();
    let plan = compiled
        .plans
        .iter()
        .find(|p| p.entity == node)
        .expect("node is compiled");
    let slot = plan.base + plan.field_offsets[field as usize];
    app.world().resource::<PortArena>().values[slot]
        .try_downcast_ref::<Events<Ping>>()
        .expect("slot holds Events<Ping>")
        .occurrences
        .iter()
        .map(|o| o.offset)
        .collect()
}
```

- [ ] **Step 2: Replace the arena**

In `crates/sway-graph/src/ports.rs`, delete `BoxedOccurrence`, `ContinuousIdx`, `EventIdx`, `Event<T>` and the whole existing `PortArena` (including `clear_events`, `resize` and the three arena tests that reference them). Keep the Task 1 types. Add:

```rust
/// Where every port value lives between nodes.
///
/// One collection, because every slot now holds a value: a plain reflect
/// value, an `Events<T>` list, or a `Product<T>` reference. The pre-unification
/// arena had a second collection for events, which is no longer a different
/// kind of thing.
#[derive(Resource)]
pub struct PortArena {
    pub values: Vec<Box<dyn PartialReflect>>,
}

impl PortArena {
    pub fn new(len: usize) -> Self {
        Self {
            // `()` rather than a zero: an unwritten read is then visibly
            // wrong rather than plausibly 0.0.
            values: (0..len).map(|_| Box::new(()) as Box<dyn PartialReflect>).collect(),
        }
    }

    /// Grows or shrinks to a new compiled layout, keeping the values that
    /// still have a slot. Recompilation calls this.
    pub fn resize(&mut self, len: usize) {
        self.values
            .resize_with(len, || Box::new(()) as Box<dyn PartialReflect>);
    }
}
```

Add a test to the `tests` module:

```rust
    #[test]
    fn resize_preserves_existing_values() {
        let mut arena = PortArena::new(1);
        arena.values[0] = Box::new(3.5_f32);

        arena.resize(3);

        assert_eq!(arena.values[0].try_downcast_ref::<f32>().copied(), Some(3.5));
        assert_eq!(arena.values.len(), 3);
    }
```

- [ ] **Step 3: Replace `PortView`**

Replace `crates/sway-graph/src/view.rs` entirely:

```rust
//! `PortView` — a node's scoped window onto the arena.
//!
//! Indices are the node's own **field ordinals** (`Gain::GAIN`,
//! `Emitter::OUT_PULSE`, ...) and, for a `Vec` field, an element index.
//! `PortView` resolves them against the node's own base internally, which is
//! what stops a node reaching another node's slots by arithmetic.

use bevy_ecs::entity::Entity;
use bevy_reflect::Reflect;

use crate::ports::{Events, Occurrence, PortArena};
use crate::schema::{FieldKind, FieldSpec};

/// Context shared by every node ticked this frame.
pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds.
    pub tick_start: f64,
    /// Monotonically increasing tick counter, starting at 0.
    pub tick_index: u64,
}

/// Scoped to one node: field ordinals are resolved against its base here.
pub struct PortView<'a> {
    arena: &'a mut PortArena,
    base: usize,
    fields: &'a [FieldSpec],
    field_offsets: &'a [usize],
    field_lens: &'a [usize],
    connected: &'a [bool],
}

impl<'a> PortView<'a> {
    pub fn new(
        arena: &'a mut PortArena,
        base: usize,
        fields: &'a [FieldSpec],
        field_offsets: &'a [usize],
        field_lens: &'a [usize],
        connected: &'a [bool],
    ) -> Self {
        Self { arena, base, fields, field_offsets, field_lens, connected }
    }

    fn slot(&self, field: u16, index: u16) -> usize {
        let f = field as usize;
        assert!(
            f < self.field_lens.len(),
            "PortView: field ordinal {field} is out of range for this node's {} fields",
            self.field_lens.len()
        );
        let len = self.field_lens[f];
        assert!(
            (index as usize) < len,
            "PortView: element {index} is out of range for field `{}`, which has {len} slot(s)",
            self.fields[f].name
        );
        self.base + self.field_offsets[f] + index as usize
    }

    /// How many slots a field has: 1, or the instance's `Vec` length.
    pub fn len(&self, field: u16) -> usize {
        self.field_lens[field as usize]
    }

    pub fn is_empty(&self, field: u16) -> bool {
        self.len(field) == 0
    }

    /// Whether an edge drives this slot. False means it holds its authored
    /// value.
    pub fn is_connected(&self, field: u16, index: u16) -> bool {
        let slot = self.slot(field, index) - self.base;
        self.connected.get(slot).copied().unwrap_or(false)
    }

    /// Reads a non-`Vec` field's value.
    ///
    /// A compiled graph guarantees the slot holds exactly `T`, so a downcast
    /// failure here means the compiler failed to catch a type mismatch. The
    /// panic is deliberate: the tick is documented infallible for valid
    /// graphs.
    pub fn read<T: Reflect + Clone>(&self, field: u16) -> T {
        self.read_at(field, 0)
    }

    pub fn read_at<T: Reflect + Clone>(&self, field: u16, index: u16) -> T {
        let slot = self.slot(field, index);
        self.arena.values[slot]
            .try_downcast_ref::<T>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::read: field `{}`[{index}] does not hold a `{}` — the compiler \
                     should have caught this type mismatch before the tick ran",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .clone()
    }

    /// Overwrites a non-`Vec` field's slot. Immediate — a node later in
    /// compiled order sees this within the same tick.
    pub fn write<T: Reflect>(&mut self, field: u16, value: T) {
        self.write_at(field, 0, value);
    }

    pub fn write_at<T: Reflect>(&mut self, field: u16, index: u16, value: T) {
        let slot = self.slot(field, index);
        self.arena.values[slot] = Box::new(value);
    }

    /// This tick's occurrences on an event field. Empty if nothing arrived.
    pub fn events<T: Reflect>(&self, field: u16) -> &[Occurrence<T>] {
        self.events_at(field, 0)
    }

    pub fn events_at<T: Reflect>(&self, field: u16, index: u16) -> &[Occurrence<T>] {
        let slot = self.slot(field, index);
        &self.arena.values[slot]
            .try_downcast_ref::<Events<T>>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::events: field `{}`[{index}] does not hold an `Events<{}>`",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .occurrences
    }

    /// Appends an occurrence to an event field's slot for this tick.
    pub fn emit<T: Reflect>(&mut self, field: u16, offset: f32, value: T) {
        let slot = self.slot(field, 0);
        self.arena.values[slot]
            .try_downcast_mut::<Events<T>>()
            .unwrap_or_else(|| {
                panic!(
                    "PortView::emit: field `{}` does not hold an `Events<{}>`",
                    self.fields[field as usize].name,
                    core::any::type_name::<T>()
                )
            })
            .occurrences
            .push(Occurrence { offset, value });
    }

    /// The entity feeding a `Product` field's slot, or `None` if unconnected.
    pub fn source(&self, field: u16, index: u16) -> Option<Entity> {
        let slot = self.slot(field, index);
        let FieldKind::Product { access, .. } = self.fields[field as usize].kind else {
            panic!(
                "PortView::source: field `{}` is not a product",
                self.fields[field as usize].name
            );
        };
        (access.get)(&*self.arena.values[slot])
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::*;
    use crate::ports::PortArena;
    use crate::schema::derive_fields;
    use crate::test_nodes::GainInlets;
    use bevy_reflect::TypeRegistry;

    fn gain_fields() -> Vec<FieldSpec> {
        let mut registry = TypeRegistry::new();
        registry.register::<GainInlets>();
        derive_fields::<GainInlets>(&registry).expect("fields")
    }

    #[test]
    fn an_out_of_range_field_cannot_cross_a_node_boundary() {
        let mut arena = PortArena::new(4);
        arena.values[3] = Box::new(41.0_f32);
        let fields = gain_fields();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut view = PortView::new(&mut arena, 0, &fields, &[0, 1], &[1, 1], &[false, false]);
            view.write(9, 99.0_f32);
        }));

        assert!(result.is_err(), "a field outside the node must panic");
        assert_eq!(
            arena.values[3].try_downcast_ref::<f32>(),
            Some(&41.0),
            "another node's slot must remain untouched"
        );
    }

    #[test]
    fn an_out_of_range_element_cannot_cross_a_node_boundary() {
        let mut arena = PortArena::new(4);
        arena.values[3] = Box::new(41.0_f32);
        let fields = gain_fields();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut view = PortView::new(&mut arena, 0, &fields, &[0, 1], &[1, 1], &[false, false]);
            // `gain` has one slot; element 2 is past it and into `bias`.
            view.write_at(0, 2, 99.0_f32);
        }));

        assert!(result.is_err(), "an element past a field's length must panic");
        assert_eq!(arena.values[3].try_downcast_ref::<f32>(), Some(&41.0));
    }
}
```

- [ ] **Step 4: Replace the registry**

Replace the non-test portion of `crates/sway-graph/src/registry.rs`. The new `NodeType`, entry and registration:

```rust
//! The node type registry.
//!
//! A node type is plugin-shaped; a node instance is an entity. `register`
//! erases `tick` to a bare fn stored here, and the tick loop dispatches
//! through it — there is no `NodeInstance` trait object.

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::structs::Struct;
use bevy_reflect::{GetTypeRegistration, PartialReflect, Reflect, Typed};
use std::collections::HashMap;

use crate::compile::NodePlan;
use crate::ports::PortArena;
use crate::schema::{derive_fields, FieldKind, FieldSpec};
use crate::view::{PortView, TickCtx};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeTypeId(pub u32);

pub type TickFn = fn(&mut World, Entity, &mut PortView, &TickCtx);
pub type CookFn = fn(&mut World, Entity, &PortView);
pub type PrefillFn = fn(&World, Entity, &mut PortArena, &NodePlan);
pub type SeedOutletsFn = fn(&mut PortArena, &NodePlan);
pub type InsertDefaultsFn = fn(&mut World, Entity);
pub type TickOfFn = fn(&World, Entity) -> Option<Tick>;
pub type ProducedTickFn = fn(&World, Entity) -> Option<Tick>;
/// Per inlet field: how many slots this instance has. 1 for a non-`Vec`
/// field, the instance's `Vec` length otherwise.
pub type InletLensFn = fn(&World, Entity) -> Vec<usize>;

pub trait NodeType: 'static {
    /// Everything the node consumes: authored values, event lists and
    /// products, in one struct. Also the component holding authored values.
    type Inlets: Reflect + Typed + GetTypeRegistration + Component + Default;
    /// Everything the node offers. Direction comes from which struct a field
    /// is in, so the same marker types are legal in both.
    type Outlets: Reflect + Typed + GetTypeRegistration + Default;
    type State: Component + Default;

    /// `(field name, the ordinal the node's index const uses)` for every
    /// field, inlets first then outlets. Verified against the derived fields
    /// at registration, so a field reorder fails at startup instead of
    /// silently swapping two ports.
    const ORDINALS: &'static [(&'static str, u16)];

    /// Whether `cook` is meaningful. Rust cannot distinguish a defaulted
    /// trait method from an overridden one, and the gate needs to know
    /// whether a node has a cook at all.
    const COOKS: bool = false;

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);

    /// Reads this node's product inlets and writes its own product. Runs
    /// immediately after `tick`, only when the gate says the node is dirty.
    fn cook(_world: &mut World, _node: Entity, _ports: &PortView) {}

    /// The change tick of whatever this node's product consumers depend on.
    ///
    /// `None` — the default — means "changes to what I produce do not require
    /// my consumers to re-cook", which is correct for a material node: its
    /// consumers hold its `Handle`, and editing the material's params does
    /// not change the handle.
    fn produced_change_tick(_world: &World, _node: Entity) -> Option<Tick> {
        None
    }
}

pub struct NodeTypeEntry {
    pub name: &'static str,
    pub inlets: Vec<FieldSpec>,
    pub outlets: Vec<FieldSpec>,
    pub tick: TickFn,
    pub prefill: PrefillFn,
    pub seed_outlets: SeedOutletsFn,
    pub insert_defaults: InsertDefaultsFn,
    pub inlets_changed_tick: TickOfFn,
    pub inlet_lens: InletLensFn,
    /// `Some` iff `N::COOKS`.
    pub cook: Option<CookFn>,
    pub produced_change_tick: ProducedTickFn,
}

impl NodeTypeEntry {
    /// Resolves a field ordinal in the node's one flat space: inlets first,
    /// then outlets. Returns the spec and whether it is an inlet.
    pub fn field(&self, ordinal: u16) -> Option<(&FieldSpec, bool)> {
        let o = ordinal as usize;
        if o < self.inlets.len() {
            Some((&self.inlets[o], true))
        } else {
            self.outlets.get(o - self.inlets.len()).map(|f| (f, false))
        }
    }

    pub fn field_count(&self) -> usize {
        self.inlets.len() + self.outlets.len()
    }
}

#[derive(Resource, Default)]
pub struct NodeTypeRegistry {
    entries: Vec<NodeTypeEntry>,
    by_name: HashMap<&'static str, NodeTypeId>,
}

impl NodeTypeRegistry {
    pub fn get(&self, id: NodeTypeId) -> Option<&NodeTypeEntry> {
        self.entries.get(id.0 as usize)
    }
    pub fn id_of(&self, name: &str) -> Option<NodeTypeId> {
        self.by_name.get(name).copied()
    }
}

pub fn register_node_type<N: NodeType>(app: &mut App) -> NodeTypeId {
    N::register(app);

    app.init_resource::<AppTypeRegistry>();
    {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let mut w = registry.write();
        w.register::<N::Inlets>();
        w.register::<N::Outlets>();
    }

    let (inlets, outlets) = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        (
            derive_fields::<N::Inlets>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
            derive_fields::<N::Outlets>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
        )
    };

    check_ordinals::<N>(&inlets, &outlets);
    check_outlets::<N>(&outlets);

    let entry = NodeTypeEntry {
        name: core::any::type_name::<N>(),
        inlets,
        outlets,
        tick: N::tick,
        prefill: prefill_of::<N>,
        seed_outlets: seed_outlets_of::<N>,
        insert_defaults: insert_defaults_of::<N>,
        inlets_changed_tick: inlets_changed_tick_of::<N>,
        inlet_lens: inlet_lens_of::<N>,
        cook: if N::COOKS { Some(N::cook as CookFn) } else { None },
        produced_change_tick: N::produced_change_tick,
    };

    app.init_resource::<NodeTypeRegistry>();
    let mut reg = app.world_mut().resource_mut::<NodeTypeRegistry>();
    let id = NodeTypeId(reg.entries.len() as u32);
    reg.by_name.insert(entry.name, id);
    reg.entries.push(entry);
    id
}

/// The startup guard: the node's index consts must agree with the derived
/// field ordinals. Inlets occupy 0..inlets.len(); outlets follow.
///
/// Element indices are *not* declared — they are positional within their
/// field, and a `Vec` field's length is per-instance.
fn check_ordinals<N: NodeType>(inlets: &[FieldSpec], outlets: &[FieldSpec]) {
    let node = core::any::type_name::<N>();
    let expected: Vec<(&'static str, u16)> = inlets
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name, i as u16))
        .chain(
            outlets
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name, (inlets.len() + i) as u16)),
        )
        .collect();

    let mut matched = vec![false; N::ORDINALS.len()];
    for &(name, want) in &expected {
        if let Some((index, _)) = N::ORDINALS
            .iter()
            .enumerate()
            .find(|(index, entry)| !matched[*index] && **entry == (name, want))
        {
            matched[index] = true;
            continue;
        }
        match N::ORDINALS
            .iter()
            .enumerate()
            .find(|(index, (declared, _))| !matched[*index] && declared == &name)
        {
            Some((_, (_, got))) => panic!(
                "{node}: field `{name}` is ordinal {want}, but ORDINALS declares {got} \
                 — a field was reordered, or the const is stale"
            ),
            None => panic!(
                "{node}: field `{name}` is undeclared in ORDINALS (expected ordinal {want})"
            ),
        }
    }
    for (index, (name, _)) in N::ORDINALS.iter().enumerate() {
        if !matched[index] {
            panic!("{node}: ORDINALS declares `{name}`, which is not a field");
        }
    }
}

/// Two limits on `Outlets`, both deliberate (design §12):
///
/// - at most one product outlet, which is what keeps `produced_change_tick` a
///   per-node function rather than a per-outlet table;
/// - no `Vec` outlets, because nothing needs a variable number of outputs and
///   allowing it would put a per-instance count on the source side of every
///   edge.
fn check_outlets<N: NodeType>(outlets: &[FieldSpec]) {
    let node = core::any::type_name::<N>();
    let mut products = outlets
        .iter()
        .filter(|f| matches!(f.kind, FieldKind::Product { .. }));
    if let (Some(first), Some(second)) = (products.next(), products.next()) {
        panic!(
            "{node}: outlets `{}` and `{}` are both products, and a node may have at most one \
             — `produced_change_tick` is per node, not per outlet",
            first.name, second.name
        );
    }
    if let Some(variadic) = outlets.iter().find(|f| f.variadic) {
        panic!(
            "{node}: outlet `{}` is a Vec, which is not supported — a node's outlet count is \
             fixed by its type",
            variadic.name
        );
    }
}

/// Copies every **unconnected** inlet slot from the node's `Inlets` component
/// into the arena. The authored-versus-driven rule: `Inlets` is never written
/// by the graph, so a disconnect returns the slot to its authored value.
fn prefill_of<N: NodeType>(world: &World, node: Entity, arena: &mut PortArena, plan: &NodePlan) {
    let Some(inlets) = world.get::<N::Inlets>(node) else {
        return;
    };
    let inlets: &dyn Struct = inlets.reflect_ref().as_struct().expect("Inlets is a struct");

    for (ordinal, spec) in plan.fields[..plan.inlet_field_count].iter().enumerate() {
        let field = inlets
            .field_at(spec.field_index)
            .expect("field_index came from this type's own fields");
        let offset = plan.field_offsets[ordinal];
        let len = plan.field_lens[ordinal];

        for index in 0..len {
            if plan.connected[offset + index] {
                continue;
            }
            let value = if spec.variadic {
                field
                    .reflect_ref()
                    .as_list()
                    .expect("a variadic field reflects as a list")
                    .get(index)
                    .expect("compile sized this field from the same list")
            } else {
                field
            };
            arena.values[plan.base + offset + index] = clone_authored(value);
        }
    }
}

fn clone_authored(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|error| {
            panic!(
                "could not clone `{}` while prefilling an inlet: {error:?}",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// Writes each outlet's default into its slot, and seeds a product outlet
/// with the node's own entity — which is constant for the life of a compiled
/// graph, so the node's `tick` never has to write it.
fn seed_outlets_of<N: NodeType>(arena: &mut PortArena, plan: &NodePlan) {
    let outlets = N::Outlets::default();
    let outlets: &dyn Struct = outlets.reflect_ref().as_struct().expect("Outlets is a struct");

    for (offset_index, spec) in plan.fields[plan.inlet_field_count..].iter().enumerate() {
        let ordinal = plan.inlet_field_count + offset_index;
        let slot = plan.base + plan.field_offsets[ordinal];
        let value = outlets
            .field_at(spec.field_index)
            .expect("field_index came from this type's own fields");
        arena.values[slot] = clone_authored(value);

        if let FieldKind::Product { access, .. } = spec.kind {
            (access.set)(&mut *arena.values[slot], Some(plan.entity));
        }
    }
}

fn insert_defaults_of<N: NodeType>(world: &mut World, node: Entity) {
    if world.get::<N::Inlets>(node).is_none() {
        world.entity_mut(node).insert(N::Inlets::default());
    }
    if world.get::<N::State>(node).is_none() {
        world.entity_mut(node).insert(N::State::default());
    }
}

fn inlets_changed_tick_of<N: NodeType>(world: &World, node: Entity) -> Option<Tick> {
    world
        .get_entity(node)
        .ok()?
        .get_change_ticks::<N::Inlets>()
        .map(|t| t.changed)
}

fn inlet_lens_of<N: NodeType>(world: &World, node: Entity) -> Vec<usize> {
    let Some(inlets) = world.get::<N::Inlets>(node) else {
        return Vec::new();
    };
    let inlets: &dyn Struct = inlets.reflect_ref().as_struct().expect("Inlets is a struct");
    (0..inlets.field_len())
        .map(|i| {
            let field = inlets.field_at(i).expect("index below field_len");
            match field.reflect_ref().as_list() {
                Ok(list) => list.len(),
                Err(_) => 1,
            }
        })
        .collect()
}
```

Keep the existing `#[cfg(test)] mod tests` block's *structure* but rewrite its node types to the new contract. The four ordinal-guard tests carry over unchanged in intent: a wrong ordinal, a missing declaration, an ordinal naming a nonexistent field, and matching inlet/outlet names. Add two:

```rust
    #[test]
    fn two_product_outlets_fail_registration() {
        // design §12: at most one product outlet per node, because
        // produced_change_tick is per node.
        use crate::ports::{Product, Spatial};
        use crate::schema::register_product;

        #[derive(Reflect, Default)]
        struct TwoProducts {
            a: Product<Spatial>,
            b: Product<Spatial>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Inlets = ProbeInlets;
            type Outlets = TwoProducts;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] =
                &[("gain", 0), ("a", 1), ("b", 2)];
            fn register(app: &mut App) {
                register_product::<Spatial>(app);
            }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("at most one"), "{msg}");
    }

    #[test]
    fn a_vec_outlet_fails_registration() {
        #[derive(Reflect, Default)]
        struct VecOutlet {
            values: Vec<f32>,
        }

        struct Bad;
        impl NodeType for Bad {
            type Inlets = ProbeInlets;
            type Outlets = VecOutlet;
            type State = ProbeState;
            const ORDINALS: &'static [(&'static str, u16)] = &[("gain", 0), ("values", 1)];
            fn register(_app: &mut App) {}
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = panic_message(&*err);
        assert!(msg.contains("values"), "must name the outlet: {msg}");
    }
```

where `ProbeInlets` is `#[derive(Reflect, Component, Default)] struct ProbeInlets { gain: f32 }` and `ProbeState` is `#[derive(Component, Default)] struct ProbeState;`, both declared at the top of the test module, and `panic_message` is carried over from the existing test module unchanged.

- [ ] **Step 5: Replace the compiler**

Replace the non-test portion of `crates/sway-graph/src/compile.rs`:

```rust
//! Graph compilation: one validation pass, one topological sort, one plan per
//! node.
//!
//! All failure happens here — the tick is infallible, and only walks the plans
//! this produces.

use core::any::TypeId;
use core::fmt;
use std::collections::{HashMap, VecDeque};

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;

use crate::edges::{Edge, EdgeFrom, EdgeTo, GraphNode, NodeId, NodeRuntime};
use crate::ports::Spatial;
use crate::registry::{NodeTypeId, NodeTypeRegistry};
use crate::schema::{FieldKind, FieldSpec, ProductAccess};

/// The compiled, per-node-instance plan the runner reads.
pub struct NodePlan {
    pub entity: Entity,
    pub node_type: NodeTypeId,
    /// This node's fields: inlets first, then outlets. Cloned from the
    /// registry so the runner can hold it while `world` is borrowed mutably.
    pub fields: Vec<FieldSpec>,
    /// How many of `fields` are inlets.
    pub inlet_field_count: usize,
    /// Absolute base of this node's slots in the arena.
    pub base: usize,
    /// Per field ordinal: offset from `base` of that field's first slot.
    pub field_offsets: Vec<usize>,
    /// Per field ordinal: how many slots it has — 1, or the instance's `Vec`
    /// length.
    pub field_lens: Vec<usize>,
    /// How many slots this node's inlets occupy, so `base..base + inlet_slots`
    /// is exactly the prefillable range.
    pub inlet_slots: usize,
    /// Per slot, relative to `base`: whether an edge drives it. Sized to the
    /// node's total slots so `PortView` can index it uniformly.
    pub connected: Vec<bool>,
    /// Absolute `(source slot, dest slot)` for every edge into this node.
    pub copies: Vec<(usize, usize)>,
    /// Absolute slot and accessor for every product inlet, whether filled or
    /// not — the cook gate walks these to find its sources.
    pub product_inlets: Vec<(usize, ProductAccess)>,
}

/// The output of [`compile`].
#[derive(Resource)]
pub struct CompiledGraph {
    /// One entry per node, in topological order.
    pub plans: Vec<NodePlan>,
    pub slots_len: usize,
    pub(crate) outlets_seeded: bool,
    /// Every `Events` slot in the graph, with the fn that empties it in
    /// place. Run once at the top of each tick.
    pub clears: Vec<(usize, fn(&mut dyn PartialReflect))>,
    /// Entity → index into `plans`, for the cook gate's source lookup.
    pub plan_index_of: HashMap<Entity, usize>,
}

/// Everything that can go wrong at compile time. Every `Display` arm names
/// the offending node(s).
#[derive(Debug)]
pub enum CompileError {
    UnknownNodeType { node: Entity, id: NodeTypeId },
    MissingEndpoint { edge: Entity, missing: Entity },
    FieldOutOfRange { node: Entity, field: u16, arity: usize },
    ElementOutOfRange { node: Entity, field: &'static str, index: u16, len: usize },
    WrongDirection { node: Entity, field: &'static str, expected: &'static str },
    TypeMismatch {
        source: Entity,
        source_field: &'static str,
        source_type: &'static str,
        target: Entity,
        target_field: &'static str,
        target_type: &'static str,
    },
    InletAlreadyConnected {
        target: Entity,
        field: &'static str,
        index: u16,
        first: Entity,
        second: Entity,
    },
    SpatialFanOut { child: Entity, first: Entity, second: Entity },
    ParentCycle { nodes: Vec<Entity> },
    Cycle { nodes: Vec<Entity> },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNodeType { node, id } => {
                write!(f, "node {node} has unregistered node type {id:?}")
            }
            Self::MissingEndpoint { edge, missing } => write!(
                f,
                "edge {edge} names {missing}, which is not a node in this graph"
            ),
            Self::FieldOutOfRange { node, field, arity } => write!(
                f,
                "node {node}: field ordinal {field} is out of range — the node has {arity} field(s)"
            ),
            Self::ElementOutOfRange { node, field, index, len } => write!(
                f,
                "node {node}: field `{field}` has {len} slot(s), so element {index} does not exist \
                 — resize the Vec on the node's Inlets, or edit the edge"
            ),
            Self::WrongDirection { node, field, expected } => write!(
                f,
                "node {node}: field `{field}` is not {expected} — an edge runs from an outlet to \
                 an inlet"
            ),
            Self::TypeMismatch {
                source,
                source_field,
                source_type,
                target,
                target_field,
                target_type,
            } => write!(
                f,
                "type mismatch: node {source} outlet `{source_field}` produces `{source_type}`, \
                 but node {target} inlet `{target_field}` expects `{target_type}`"
            ),
            Self::InletAlreadyConnected { target, field, index, first, second } => write!(
                f,
                "node {target}: inlet `{field}`[{index}] is already connected to node {first}; a \
                 second edge from node {second} is illegal — every inlet takes exactly one edge"
            ),
            Self::SpatialFanOut { child, first, second } => write!(
                f,
                "node {child} already has parent {first}; a second parent edge to {second} is \
                 illegal — a scene node has one parent"
            ),
            Self::ParentCycle { nodes } => write!(
                f,
                "parenting cycle: {}",
                nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" → ")
            ),
            Self::Cycle { nodes } => write!(
                f,
                "cycle: these nodes could not be ordered: {}",
                nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

impl core::error::Error for CompileError {}

/// One node's slot layout, computed before validation because edge
/// resolution needs it.
struct Layout {
    entity: Entity,
    node_type: NodeTypeId,
    fields: Vec<FieldSpec>,
    inlet_field_count: usize,
    base: usize,
    field_offsets: Vec<usize>,
    field_lens: Vec<usize>,
    inlet_slots: usize,
    slot_count: usize,
}

impl Layout {
    fn slot(&self, field: u16, index: u16) -> usize {
        self.base + self.field_offsets[field as usize] + index as usize
    }
}

struct ValidEdge {
    source_idx: usize,
    target_idx: usize,
    source_slot: usize,
    target_slot: usize,
    /// Relative to the target's base — indexes `connected`.
    target_local: usize,
    spatial: bool,
}

pub fn compile(world: &mut World) -> Result<CompiledGraph, CompileError> {
    // --- Pass 1: collect nodes, sorted by NodeId for determinism -------
    let mut node_query = world.query::<(Entity, &GraphNode)>();
    let mut raw_nodes: Vec<(Entity, NodeId, NodeTypeId)> = node_query
        .iter(world)
        .map(|(entity, node)| (entity, node.id, node.node_type))
        .collect();
    raw_nodes.sort_by_key(|(_, id, _)| *id);

    for &(entity, _, node_type) in &raw_nodes {
        let insert_defaults = {
            let registry = world.resource::<NodeTypeRegistry>();
            registry
                .get(node_type)
                .ok_or(CompileError::UnknownNodeType { node: entity, id: node_type })?
                .insert_defaults
        };
        insert_defaults(world, entity);
    }

    // --- Pass 2: lay out slots -----------------------------------------
    //
    // A variadic field's length comes from the instance, so this reads the
    // Inlets component. That one number is the only per-instance input to
    // what is otherwise a per-type schema.
    let mut layouts: Vec<Layout> = Vec::with_capacity(raw_nodes.len());
    let mut cursor = 0usize;
    for &(entity, _, node_type) in &raw_nodes {
        let (fields, inlet_field_count, inlet_lens) = {
            let inlet_lens_fn = {
                let registry = world.resource::<NodeTypeRegistry>();
                let entry = registry
                    .get(node_type)
                    .ok_or(CompileError::UnknownNodeType { node: entity, id: node_type })?;
                entry.inlet_lens
            };
            let lens = inlet_lens_fn(world, entity);
            let registry = world.resource::<NodeTypeRegistry>();
            let entry = registry.get(node_type).expect("resolved above");
            let mut fields = entry.inlets.clone();
            let inlet_field_count = fields.len();
            fields.extend(entry.outlets.iter().cloned());
            (fields, inlet_field_count, lens)
        };

        let base = cursor;
        let mut field_offsets = Vec::with_capacity(fields.len());
        let mut field_lens = Vec::with_capacity(fields.len());
        let mut offset = 0usize;
        for (ordinal, spec) in fields.iter().enumerate() {
            let len = if ordinal < inlet_field_count {
                // `inlet_lens` reports 1 for a non-Vec field already.
                inlet_lens.get(spec.field_index).copied().unwrap_or(1)
            } else {
                1 // outlets cannot be Vec — enforced at registration
            };
            field_offsets.push(offset);
            field_lens.push(len);
            offset += len;
            if ordinal + 1 == inlet_field_count {
                // remember where inlets end
            }
        }
        let inlet_slots: usize = field_lens[..inlet_field_count].iter().sum();
        let slot_count = offset;
        cursor += slot_count;

        layouts.push(Layout {
            entity,
            node_type,
            fields,
            inlet_field_count,
            base,
            field_offsets,
            field_lens,
            inlet_slots,
            slot_count,
        });
    }
    let slots_len = cursor;

    let index_of: HashMap<Entity, usize> =
        layouts.iter().enumerate().map(|(i, l)| (l.entity, i)).collect();

    // --- Pass 3: validate every edge ------------------------------------
    struct RawEdge {
        edge: Entity,
        from: Entity,
        to: Entity,
        from_field: u16,
        from_index: u16,
        to_field: u16,
        to_index: u16,
    }

    let mut edge_query = world.query::<(Entity, &Edge, &EdgeFrom, &EdgeTo)>();
    let raw_edges: Vec<RawEdge> = edge_query
        .iter(world)
        .map(|(edge, e, from, to)| RawEdge {
            edge,
            from: from.0,
            to: to.0,
            from_field: e.from.field,
            from_index: e.from.index,
            to_field: e.to.field,
            to_index: e.to.index,
        })
        .collect();

    let mut valid: Vec<ValidEdge> = Vec::with_capacity(raw_edges.len());
    let mut filled: HashMap<usize, Entity> = HashMap::new();
    // Spatial outlets are single-consumer: keyed by source node index.
    let mut spatial_consumer: HashMap<usize, Entity> = HashMap::new();
    let mut parent_of: Vec<Option<usize>> = vec![None; layouts.len()];

    for raw in raw_edges {
        let &source_idx = index_of
            .get(&raw.from)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.from })?;
        let &target_idx = index_of
            .get(&raw.to)
            .ok_or(CompileError::MissingEndpoint { edge: raw.edge, missing: raw.to })?;

        let source = &layouts[source_idx];
        let target = &layouts[target_idx];

        let source_spec = source.fields.get(raw.from_field as usize).ok_or(
            CompileError::FieldOutOfRange {
                node: source.entity,
                field: raw.from_field,
                arity: source.fields.len(),
            },
        )?;
        let target_spec = target.fields.get(raw.to_field as usize).ok_or(
            CompileError::FieldOutOfRange {
                node: target.entity,
                field: raw.to_field,
                arity: target.fields.len(),
            },
        )?;

        // An edge runs outlet → inlet. Direction is which half of the field
        // space the ordinal lands in.
        if (raw.from_field as usize) < source.inlet_field_count {
            return Err(CompileError::WrongDirection {
                node: source.entity,
                field: source_spec.name,
                expected: "an outlet",
            });
        }
        if (raw.to_field as usize) >= target.inlet_field_count {
            return Err(CompileError::WrongDirection {
                node: target.entity,
                field: target_spec.name,
                expected: "an inlet",
            });
        }

        let source_len = source.field_lens[raw.from_field as usize];
        if (raw.from_index as usize) >= source_len {
            return Err(CompileError::ElementOutOfRange {
                node: source.entity,
                field: source_spec.name,
                index: raw.from_index,
                len: source_len,
            });
        }
        let target_len = target.field_lens[raw.to_field as usize];
        if (raw.to_index as usize) >= target_len {
            return Err(CompileError::ElementOutOfRange {
                node: target.entity,
                field: target_spec.name,
                index: raw.to_index,
                len: target_len,
            });
        }

        // One type check for every carrier: a slot type is a slot type.
        if source_spec.slot_type != target_spec.slot_type {
            return Err(CompileError::TypeMismatch {
                source: source.entity,
                source_field: source_spec.name,
                source_type: source_spec.slot_type_path,
                target: target.entity,
                target_field: target_spec.name,
                target_type: target_spec.slot_type_path,
            });
        }

        let target_slot = target.slot(raw.to_field, raw.to_index);
        if let Some(&first) = filled.get(&target_slot) {
            return Err(CompileError::InletAlreadyConnected {
                target: target.entity,
                field: target_spec.name,
                index: raw.to_index,
                first,
                second: source.entity,
            });
        }
        filled.insert(target_slot, source.entity);

        let spatial = matches!(
            target_spec.kind,
            FieldKind::Product { capability, .. } if capability == TypeId::of::<Spatial>()
        );
        if spatial {
            // Bevy's ChildOf is a one-parent relationship, so a Spatial
            // outlet may feed at most one inlet.
            if let Some(&first) = spatial_consumer.get(&source_idx) {
                return Err(CompileError::SpatialFanOut {
                    child: source.entity,
                    first,
                    second: target.entity,
                });
            }
            spatial_consumer.insert(source_idx, target.entity);
            parent_of[source_idx] = Some(target_idx);
        }

        valid.push(ValidEdge {
            source_idx,
            target_idx,
            source_slot: source.slot(raw.from_field, raw.from_index),
            target_slot,
            target_local: target_slot - target.base,
            spatial,
        });
    }

    // --- Pass 4: parenting acyclicity ------------------------------------
    //
    // Checked separately from the sort, because Spatial edges are excluded
    // from it — a parent reads nothing from its child, and including them
    // would reject a child that drives a param on its own parent.
    for start in 0..layouts.len() {
        let mut cursor = parent_of[start];
        let mut chain = vec![layouts[start].entity];
        let mut seen = 0usize;
        while let Some(idx) = cursor {
            if idx == start {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            chain.push(layouts[idx].entity);
            seen += 1;
            if seen > layouts.len() {
                return Err(CompileError::ParentCycle { nodes: chain });
            }
            cursor = parent_of[idx];
        }
    }

    // --- Pass 5: one topological sort, Spatial excluded -------------------
    let n = layouts.len();
    let mut in_degree = vec![0u32; n];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in valid.iter().filter(|e| !e.spatial) {
        in_degree[edge.target_idx] += 1;
        adjacency[edge.source_idx].push(edge.target_idx);
    }
    for adj in &mut adjacency {
        adj.sort_unstable();
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while let Some(idx) = queue.pop_front() {
        order.push(idx);
        placed[idx] = true;
        for &next in &adjacency[idx] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push_back(next);
            }
        }
    }
    if order.len() != n {
        let remaining: Vec<Entity> =
            (0..n).filter(|&i| !placed[i]).map(|i| layouts[i].entity).collect();
        return Err(CompileError::Cycle { nodes: remaining });
    }

    // --- Pass 6: build plans, in compiled order ---------------------------
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, edge) in valid.iter().enumerate() {
        incoming[edge.target_idx].push(i);
    }

    let mut plans: Vec<NodePlan> = Vec::with_capacity(n);
    let mut clears: Vec<(usize, fn(&mut dyn PartialReflect))> = Vec::new();

    for &idx in &order {
        let layout = &layouts[idx];
        let mut connected = vec![false; layout.slot_count];
        let mut copies: Vec<(usize, usize)> = Vec::new();

        for &edge_idx in &incoming[idx] {
            let edge = &valid[edge_idx];
            connected[edge.target_local] = true;
            copies.push((edge.source_slot, edge.target_slot));
        }
        copies.sort_unstable_by_key(|&(_, dest)| dest);

        let mut product_inlets: Vec<(usize, ProductAccess)> = Vec::new();
        for (ordinal, spec) in layout.fields.iter().enumerate() {
            let offset = layout.field_offsets[ordinal];
            for index in 0..layout.field_lens[ordinal] {
                let slot = layout.base + offset + index;
                match spec.kind {
                    FieldKind::Events { clear, .. } => clears.push((slot, clear)),
                    FieldKind::Product { access, .. } if ordinal < layout.inlet_field_count => {
                        product_inlets.push((slot, access));
                    }
                    _ => {}
                }
            }
        }

        plans.push(NodePlan {
            entity: layout.entity,
            node_type: layout.node_type,
            fields: layout.fields.clone(),
            inlet_field_count: layout.inlet_field_count,
            base: layout.base,
            field_offsets: layout.field_offsets.clone(),
            field_lens: layout.field_lens.clone(),
            inlet_slots: layout.inlet_slots,
            connected,
            copies,
            product_inlets,
        });
    }

    clears.sort_unstable_by_key(|&(slot, _)| slot);

    let plan_index_of: HashMap<Entity, usize> =
        plans.iter().enumerate().map(|(i, p)| (p.entity, i)).collect();

    // --- Pass 7: apply ChildOf, write NodeRuntime -------------------------
    for (idx, layout) in layouts.iter().enumerate() {
        match parent_of[idx] {
            Some(parent_idx) => {
                let parent = layouts[parent_idx].entity;
                world
                    .entity_mut(layout.entity)
                    .insert(bevy_ecs::hierarchy::ChildOf(parent));
            }
            None => {
                world
                    .entity_mut(layout.entity)
                    .remove::<bevy_ecs::hierarchy::ChildOf>();
            }
        }
    }
    for plan in &plans {
        world.entity_mut(plan.entity).insert(NodeRuntime {
            last_inlets_tick: None,
            cook_dirty: true,
            last_product_ticks: vec![None; plan.product_inlets.len()],
        });
    }

    Ok(CompiledGraph {
        plans,
        slots_len,
        outlets_seeded: false,
        clears,
        plan_index_of,
    })
}
```

Delete the now-unused `if ordinal + 1 == inlet_field_count {}` block if the compiler warns about it — it is a no-op left from computing `inlet_slots` two ways; `inlet_slots` is summed directly below it.

- [ ] **Step 6: Update `NodeRuntime`**

In `crates/sway-graph/src/edges.rs`, replace the `NodeRuntime` fields (`continuous_base`, `event_base`, `last_params_tick`, `last_slot_ticks`) with:

```rust
#[derive(Component, Default)]
pub struct NodeRuntime {
    /// The `Inlets` change tick this node last prefilled against. `None`
    /// forces a prefill, which is how a recompile makes a disconnect take
    /// effect.
    pub last_inlets_tick: Option<Tick>,
    /// The cook gate. Sticky: set when a driven inlet changes, when prefill
    /// fires, or when an upstream product's change tick moves; cleared only
    /// by a cook that actually ran. Stickiness is what makes it survive a
    /// skipped cadence, which a `Changed<T>` filter cannot.
    pub cook_dirty: bool,
    /// Per product inlet: the source's `produced_change_tick` at this node's
    /// last cook.
    pub last_product_ticks: Vec<Option<Tick>>,
}
```

Delete `ParamEdge`, `FeedsEdge`, `ParentEdge` and `PortKind` from the same file.

- [ ] **Step 7: Replace the runner**

Replace the non-test portion of `crates/sway-graph/src/tick.rs`:

```rust
//! The tick runner: one exclusive system in `FixedUpdate` that walks the
//! compiled plan.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::change_detection::{Mut, Tick};
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use bevy_time::{Fixed, Time};

use crate::compile::CompiledGraph;
use crate::edges::NodeRuntime;
use crate::ports::PortArena;
use crate::registry::{
    CookFn, InletLensFn, NodeTypeRegistry, PrefillFn, ProducedTickFn, SeedOutletsFn, TickFn,
    TickOfFn,
};
use crate::view::{PortView, TickCtx};

/// Ticks since the graph started running. Exposed as `TickCtx::tick_index`.
#[derive(Resource, Default)]
pub struct GraphTickCount(pub u64);

/// Clones a slot's value while preserving its concrete type.
///
/// `to_dynamic` is the wrong tool: for a struct it returns a `Dynamic*`
/// proxy that can no longer downcast to the concrete type. `reflect_clone`
/// produces a real `T`.
fn clone_slot(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|e| {
            panic!(
                "graph_tick: could not clone a `{}` slot value while gathering an edge ({e:?})",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// The graph tick: one exclusive system in `FixedUpdate`.
///
/// One order, one pass: each node gathers, ticks, and cooks if dirty when its
/// turn comes. A product edge is an ordinary value edge, so the order that
/// puts a producer before its consumer is the same order that puts an LFO
/// before the node it drives.
pub fn graph_tick(world: &mut World) {
    let Some(mut compiled) = world.remove_resource::<CompiledGraph>() else {
        return;
    };

    let (dt, tick_start) = {
        let time = world.resource::<Time<Fixed>>();
        let dt = time.delta_secs();
        (dt, time.elapsed_secs_f64() - dt as f64)
    };
    let tick_index = {
        let mut count = world.resource_mut::<GraphTickCount>();
        let idx = count.0;
        count.0 += 1;
        idx
    };
    let ctx = TickCtx { dt, tick_start, tick_index };

    // Fn pointers copied out before the loop: `world` is borrowed mutably
    // inside it, so a `&NodeTypeEntry` cannot be held across it.
    struct Dispatch {
        tick: TickFn,
        prefill: PrefillFn,
        seed_outlets: SeedOutletsFn,
        inlets_changed_tick: TickOfFn,
        cook: Option<CookFn>,
        produced_change_tick: ProducedTickFn,
    }
    let _ = std::marker::PhantomData::<InletLensFn>; // compile-time only

    let dispatch: Vec<Dispatch> = {
        let registry = world.resource::<NodeTypeRegistry>();
        compiled
            .plans
            .iter()
            .map(|plan| {
                let entry = registry.get(plan.node_type).unwrap_or_else(|| {
                    panic!(
                        "graph_tick: node {:?}'s node type {:?} is not in the registry",
                        plan.entity, plan.node_type
                    )
                });
                Dispatch {
                    tick: entry.tick,
                    prefill: entry.prefill,
                    seed_outlets: entry.seed_outlets,
                    inlets_changed_tick: entry.inlets_changed_tick,
                    cook: entry.cook,
                    produced_change_tick: entry.produced_change_tick,
                }
            })
            .collect()
    };

    world.resource_scope(|world: &mut World, mut arena: Mut<PortArena>| {
        if !compiled.outlets_seeded {
            for (plan, d) in compiled.plans.iter().zip(&dispatch) {
                (d.seed_outlets)(&mut arena, plan);
            }
            compiled.outlets_seeded = true;
        }

        // Empty every event list in place, keeping its allocation. This is
        // what stops a node that stopped writing its event outlet from
        // firing the same occurrence forever.
        for &(slot, clear) in &compiled.clears {
            clear(&mut *arena.values[slot]);
        }

        for (plan_idx, (plan, d)) in compiled.plans.iter().zip(&dispatch).enumerate() {
            // `dirty` accumulates this tick's reasons to cook; it is OR-ed
            // into the sticky flag rather than assigned, so a reason raised
            // on an earlier tick is not lost.
            let mut dirty = false;

            for &(src, dst) in &plan.copies {
                let incoming = clone_slot(&*arena.values[src]);
                // `reflect_partial_eq` returns None for values that cannot be
                // compared — including the `()` a freshly-resized slot holds
                // — and None must mean "changed", never "unchanged".
                let changed = arena.values[dst]
                    .reflect_partial_eq(&*incoming)
                    .map(|equal| !equal)
                    .unwrap_or(true);
                arena.values[dst] = incoming;
                dirty |= changed;
            }

            let current = (d.inlets_changed_tick)(world, plan.entity);
            let last = world
                .get::<NodeRuntime>(plan.entity)
                .and_then(|r| r.last_inlets_tick);
            if last != current {
                (d.prefill)(world, plan.entity, &mut arena, plan);
                dirty = true;
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_inlets_tick = current;
                }
            }

            // Only touch NodeRuntime when there is something to record — an
            // unconditional get_mut churns its change tick every tick.
            if dirty && let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = true;
            }

            let mut view = PortView::new(
                &mut arena,
                plan.base,
                &plan.fields,
                &plan.field_offsets,
                &plan.field_lens,
                &plan.connected,
            );
            (d.tick)(world, plan.entity, &mut view, &ctx);

            // The cook, immediately after this node's tick. Its own params
            // are already applied, and every upstream product it reads has
            // already cooked, because the same order guarantees both.
            let Some(cook_fn) = d.cook else {
                continue;
            };
            let _ = plan_idx;

            let sources: Vec<Option<Tick>> = plan
                .product_inlets
                .iter()
                .map(|&(slot, access)| {
                    let source = (access.get)(&*arena.values[slot])?;
                    let source_plan = *compiled.plan_index_of.get(&source)?;
                    (dispatch[source_plan].produced_change_tick)(world, source)
                })
                .collect();

            let cook_dirty = match world.get::<NodeRuntime>(plan.entity) {
                Some(rt) => rt.cook_dirty || rt.last_product_ticks != sources,
                None => false,
            };
            if !cook_dirty {
                continue;
            }

            let view = PortView::new(
                &mut arena,
                plan.base,
                &plan.fields,
                &plan.field_offsets,
                &plan.field_lens,
                &plan.connected,
            );
            cook_fn(world, plan.entity, &view);

            if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = false;
                rt.last_product_ticks = sources;
            }
        }
    });

    world.insert_resource(compiled);
}

/// Inserts the graph engine's resources and wires `graph_tick` into
/// `FixedUpdate`.
pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PortArena::new(0))
            .init_resource::<NodeTypeRegistry>()
            .init_resource::<GraphTickCount>()
            .register_type::<crate::edges::EditorPos>()
            .add_systems(FixedUpdate, graph_tick);
    }
}
```

The `let _ = std::marker::PhantomData::<InletLensFn>;` and `let _ = plan_idx;` lines are scaffolding to keep the imports and the loop index honest; delete both and the `InletLensFn` import once the file compiles without them.

- [ ] **Step 8: Delete the dead modules**

```bash
git rm crates/sway-graph/src/slots.rs crates/sway-graph/src/structure.rs
```

In `crates/sway-graph/src/lib.rs`: remove `mod slots;`, `mod structure;` and every `pub use slots::{...}` line; remove `Event`, `EventIdx`, `ContinuousIdx`, `BoxedOccurrence`, `ParamEdge`, `FeedsEdge`, `ParentEdge`, `PortKind`, `SchemaHalf`, `PortField`, `derive_schema`, `register_event_port`, `NodeSchema` from the re-exports; and add `Edge`, `Endpoint`, `NodePlan`, `SeedOutletsFn`, `InletLensFn` and `CookFn` where the old equivalents were named.

Also delete `derive_schema`, `PortField`, `SchemaHalf`, `ReflectEventPort`, `register_event_port` and `is_event_marker_path` from `schema.rs`, along with the four `derive_schema` tests — `derive_fields` replaces all of it.

- [ ] **Step 9: Port the compiler and runner test suites**

The existing tests in `compile.rs`, `tick.rs` and `structure.rs` are the acceptance criterion for this task: **every one must survive in meaning.** Move the `structure.rs` tests into `compile.rs`'s test module and rewrite all three suites against the new fixtures. The full list, with what each becomes:

| Old test | Becomes |
|---|---|
| `an_edge_carries_a_value_within_one_tick` | unchanged in meaning; `connect(a, Gain::OUT_VALUE, b, Gain::GAIN)` |
| `an_unconnected_input_reads_its_authored_value` | unchanged |
| duplicate continuous fan-in | `InletAlreadyConnected`, message contains "exactly one edge" |
| `WrongPortDirection` (continuous + event) | `WrongDirection`, one test per direction, message contains "an outlet" / "an inlet" |
| `TypeMismatch` | unchanged in meaning; asserts both type paths appear |
| `PortOutOfRange` | `FieldOutOfRange`, asserts the arity is named |
| `Cycle` | unchanged; names the blocked set |
| `two_parent_edges_from_one_child_are_rejected` | `SpatialFanOut`; message contains "one parent" and names child and both parents |
| `parenting_a_non_spatial_node_is_rejected` | now a `TypeMismatch` — asserts the message names `Product<Spatial>` and the other capability |
| `parenting_under_a_non_spatial_node_is_rejected` | same as above, from the other side |
| `a_parenting_cycle_is_rejected` | `ParentCycle`; message contains "parenting" |
| `a_slot_filled_twice_is_rejected` | `InletAlreadyConnected`; names the field and both sources |
| `a_source_that_produces_nothing_is_rejected` | now `WrongDirection` or `TypeMismatch` depending on the field used; assert it names the source node |
| `a_slot_type_mismatch_names_the_capability_on_both_sides` | `TypeMismatch` between `Product<Blob>` and `Product<Sludge>`; assert each path appears on its own side |
| `a_slot_ordinal_out_of_range_reports_the_arity` | `FieldOutOfRange` |
| `a_feeds_chain_orders_producer_before_consumer` | one order now: assert `plans` puts `Producer` before `Consumer` |
| every `cooking` module test | unchanged in meaning against `Producer`/`Consumer` |

Add these, which are new behaviour this task introduces:

```rust
    #[test]
    fn a_spatial_edge_does_not_constrain_the_compiled_order() {
        // Design §4: a parent reads nothing from its child, so parenting is
        // excluded from the sort. Including it would reject this graph --
        // a child driving a param on its own parent.
        let mut app = engine_app();
        let group = spawn_group(app.world_mut(), 1);
        let child = spawn_group(app.world_mut(), 0);
        connect_at(app.world_mut(), child, Group::OUT_SPATIAL, group, Group::CHILDREN, 0);
        let gain = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect(app.world_mut(), gain, Gain::OUT_VALUE, group, Group::ROTATION_Y);

        assert!(compile(app.world_mut()).is_ok(), "parenting must not enter the sort");
    }

    #[test]
    fn a_union_cycle_across_both_old_dags_is_rejected() {
        // Design §4: this compiled before, with one side silently reading
        // stale data from phase ordering. An error is the better outcome.
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), producer, Producer::OUT_BLOB, consumer, Consumer::INPUT);
        connect(app.world_mut(), consumer, Consumer::OUT_BLOB, producer, Producer::SCALE);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains("cycle"), "{err}");
    }

    #[test]
    fn a_variadic_inlet_takes_one_edge_per_element() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 4.0, 5.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 1);
        recompile(&mut app);

        app.update();
        app.update();

        assert_eq!(port_value(&app, sum, Sum::OUT_TOTAL), 26.0, "6 + 20");
    }

    #[test]
    fn two_edges_into_one_variadic_element_are_rejected() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0]);
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        let b = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 0);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains("exactly one edge"), "{err}");
        assert!(err.contains("terms"), "must name the field: {err}");
    }

    #[test]
    fn an_edge_past_a_variadic_field_names_its_length() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 7);

        let err = compile(app.world_mut()).unwrap_err().to_string();
        assert!(err.contains('7') && err.contains('2'), "must name index and length: {err}");
    }

    #[test]
    fn resizing_a_variadic_field_leaves_other_fields_addressable() {
        // (field, index) addressing exists for exactly this: growing
        // `children` must not renumber `rotation_y`.
        let mut app = engine_app();
        let group = spawn_group(app.world_mut(), 1);
        let gain = spawn_gain(app.world_mut(), 2.0, 3.0);
        connect(app.world_mut(), gain, Gain::OUT_VALUE, group, Group::ROTATION_Y);
        recompile(&mut app);
        app.update();
        app.update();
        assert_eq!(port_value(&app, group, Group::ROTATION_Y), 6.0);

        app.world_mut()
            .get_mut::<GroupInlets>(group)
            .expect("inlets")
            .children
            .push(Product::<Spatial>::default());
        recompile(&mut app);
        app.update();

        assert_eq!(
            port_value(&app, group, Group::ROTATION_Y),
            6.0,
            "the edge into rotation_y must still resolve after children grew"
        );
    }

    #[test]
    fn an_event_slot_is_empty_at_the_start_of_every_tick() {
        let mut app = engine_app();
        let emitter = spawn_emitter(app.world_mut(), 0.001);
        let sink = spawn_sink(app.world_mut());
        connect(app.world_mut(), emitter, Emitter::OUT_PULSE, sink, Sink::PULSE);
        recompile(&mut app);

        app.update();
        app.update();
        let after_one = event_offsets(&app, sink, Sink::PULSE).len();
        app.update();
        let after_two = event_offsets(&app, sink, Sink::PULSE).len();

        assert_eq!(after_one, 1, "one occurrence per tick");
        assert_eq!(after_two, 1, "occurrences must not accumulate across ticks");
    }

    #[test]
    fn a_product_outlet_is_seeded_with_its_own_entity() {
        let mut app = engine_app();
        let producer = spawn_producer(app.world_mut());
        let consumer = spawn_consumer(app.world_mut());
        connect(app.world_mut(), producer, Producer::OUT_BLOB, consumer, Consumer::INPUT);
        recompile(&mut app);

        app.update();
        app.update();

        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == consumer).expect("compiled");
        let (slot, access) = plan.product_inlets[0];
        let arena = app.world().resource::<PortArena>();
        assert_eq!(
            (access.get)(&*arena.values[slot]),
            Some(producer),
            "the consumer's product inlet must hold the producer's entity"
        );
    }
```

- [ ] **Step 10: Run the suite**

Run: `cargo test -p sway-graph`
Expected: PASS.

Run: `cargo build --workspace 2>&1 | tail -5`
Expected: FAIL, in `sway-geo`/`sway-nodes`/`sway-app`/`sway-editor` only. That is the flip window; Tasks 5–7 close it.

- [ ] **Step 11: Commit**

```bash
git add -A crates/sway-graph
git commit -m "$(cat <<'EOF'
feat(graph)!: one edge, one arena, one order

Inlets and Outlets replace Params/Outputs/Slots/Produces/SPATIAL. Every inlet
is a typed value slot taking exactly one edge; a Vec field varies a node's
inlet count rather than an inlet's arity. Product<T> carries the source
entity, so the capability system collapses into the ordinary type check and
the tick and cook orders merge into one.

Breaks every downstream crate until the node types migrate.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## The migration rules for Tasks 5–7

Every node type changes the same way. These rules apply to all of them; each task below states only what is specific to its nodes.

**Declarations.** `XParams` → `XInlets`, `XOutputs` → `XOutlets`. `type Slots` and `type Produces` are deleted: a former slot becomes a `Product<T>` field on `Inlets`, and a former `Produces = T` becomes a `Product<T>` field on `Outlets`. `SPATIAL: bool = true` is deleted and becomes `spatial: Product<Spatial>` on `Outlets`. `NoSlots`/`NoOutputs` are gone — use an empty struct.

**Ordinals.** `PORT_ORDINALS` and `SLOT_ORDINALS` merge into one `ORDINALS` listing **inlet fields in declaration order, then outlet fields in declaration order**, numbered from 0. Every `Self::CONST` is renumbered accordingly. Put former slot fields first in `Inlets` so the structural inputs read at the top.

**Call sites.**

| Before | After |
|---|---|
| `ports.read(ContinuousIdx(Self::X as u32))` | `ports.read(Self::X)` |
| `ports.write(ContinuousIdx(Self::X as u32), v)` | `ports.write(Self::X, v)` |
| `ports.events(EventIdx(Self::X as u32))` | `ports.events::<T>(Self::X)` — a `&[Occurrence<T>]`, so `occ.value` is owned and `occ.offset` unchanged |
| `ports.emit(EventIdx(Self::X as u32), off, v)` | `ports.emit(Self::X, off, v)` |
| `fn cook(world, node, slots: &SlotView)` | `fn cook(world, node, ports: &PortView)` |
| `slots.source(Self::IN_GEO)` | `ports.source(Self::IN_GEO, 0)` |
| `register_slot::<T>(app)` | `register_product::<T>(app)` |
| `register_event_port::<T>(app)` | `register_events::<T>(app)` |

**Bodies are otherwise unchanged.** No `tick` or `cook` logic changes in Tasks 5–7 except `Envelope`'s, which gains the merge the engine used to do.

---

### Task 5: `sway-geo`

**Files:**
- Modify: `crates/sway-geo/src/grid.rs`, `crates/sway-geo/src/displace.rs`

**Interfaces:**
- Consumes: Task 4's `NodeType`, `PortView`, `register_product`.
- Produces: `Grid::{ROWS, COLS, WIDTH, HEIGHT, OUT_GEO}`, `Displace::{IN_GEO, AMOUNT, FREQUENCY, OUT_GEO}` — Task 7's demo graph wires these.

- [ ] **Step 1: Migrate `Grid`**

In `crates/sway-geo/src/grid.rs`, rename `GridParams` → `GridInlets` (fields and its `Default` impl unchanged), replace `NoOutputs` with:

```rust
#[derive(Reflect, Default)]
pub struct GridOutlets {
    pub geo: Product<Geometry>,
}
```

and replace the `impl NodeType` header:

```rust
impl Grid {
    pub const ROWS: u16 = 0;
    pub const COLS: u16 = 1;
    pub const WIDTH: u16 = 2;
    pub const HEIGHT: u16 = 3;
    pub const OUT_GEO: u16 = 4;
}

impl NodeType for Grid {
    type Inlets = GridInlets;
    type Outlets = GridOutlets;
    type State = GridState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("rows", Self::ROWS),
        ("cols", Self::COLS),
        ("width", Self::WIDTH),
        ("height", Self::HEIGHT),
        ("geo", Self::OUT_GEO),
    ];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Geometry>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _ports: &PortView) {
```

The body of `cook` is unchanged except `world.get::<GridParams>(node)` becomes `world.get::<GridInlets>(node)`. Update the `use sway_graph::{...}` line to import `NodeType, PortView, Product, TickCtx, register_product` and drop `NoOutputs`, `NoSlots`, `SlotView`.

- [ ] **Step 2: Migrate `Displace`**

In `crates/sway-geo/src/displace.rs`, delete `DisplaceSlots` and replace the declarations:

```rust
#[derive(Reflect, Component, Default)]
pub struct DisplaceInlets {
    pub geo: Product<Geometry>,
    pub amount: f32,
    pub frequency: f32,
}

#[derive(Reflect, Default)]
pub struct DisplaceOutlets {
    pub geo: Product<Geometry>,
}

impl Displace {
    pub const IN_GEO: u16 = 0;
    pub const AMOUNT: u16 = 1;
    pub const FREQUENCY: u16 = 2;
    pub const OUT_GEO: u16 = 3;
}

impl NodeType for Displace {
    type Inlets = DisplaceInlets;
    type Outlets = DisplaceOutlets;
    type State = DisplaceState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("geo", Self::IN_GEO),
        ("amount", Self::AMOUNT),
        ("frequency", Self::FREQUENCY),
        ("geo", Self::OUT_GEO),
    ];
    const COOKS: bool = true;

    fn register(app: &mut App) {
        register_product::<Geometry>(app);
    }

    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, ports: &PortView) {
        let Some(source) = ports.source(Self::IN_GEO, 0) else {
            return;
        };
```

The rest of `cook` is unchanged except `world.get::<DisplaceParams>(node)` becomes `world.get::<DisplaceInlets>(node)`.

Note that `geo` appears twice in `ORDINALS` — once as the inlet and once as the outlet. That is legal and already covered by the registry's `(name, ordinal)` matching, which M2a added for exactly this case (`Remap` has an input and output both named `value`).

- [ ] **Step 3: Run the crate's tests**

Run: `cargo test -p sway-graph -p sway-geo`
Expected: PASS. `sway-geo`'s `Arc`-sharing assertions and the ignored measurement test are unchanged by this task.

- [ ] **Step 4: Commit**

```bash
git add crates/sway-geo
git commit -m "$(cat <<'EOF'
feat(geo): migrate Grid and Displace to Inlets/Outlets

A Feeds slot is now a Product<Geometry> inlet and a Produces is a
Product<Geometry> outlet, so both ends of a geometry edge are addressable.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `sway-nodes`

All seven node files. The only body change in the whole task is `Envelope`'s.

**Files:**
- Modify: `crates/sway-nodes/src/{midi,lfo,envelope,math,material,mesh,scene}.rs`, `crates/sway-nodes/src/lib.rs`
- Modify: `crates/sway-nodes/tests/traces.rs`

**Interfaces:**
- Produces: every node's renumbered ordinal consts, which Task 7's demo graph and Task 8's snapshot tests consume.

- [ ] **Step 1: Migrate the four signal nodes**

`lfo.rs`, `math.rs` (`Math`, `Remap`, `Switch`, `Select`) and `midi.rs` (`MidiNote`, `MidiCC`) follow the migration rules with no structural change: every one has `type Slots = NoSlots` and `type Produces = ()`, so each simply renames its two structs, merges its ordinal consts, and renumbers outlets to follow inlets.

`LFO` is the worked example; the others are the same shape:

```rust
#[derive(Reflect, Component, Default)]
pub struct LfoInlets {
    pub hz: f32,
    pub shape: Waveform,
    pub phase: f32,
    pub amplitude: f32,
}

#[derive(Reflect, Default)]
pub struct LfoOutlets {
    pub value: f32,
}

impl NodeType for LFO {
    type Inlets = LfoInlets;
    type Outlets = LfoOutlets;
    type State = LfoState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("hz", Self::HZ),
        ("shape", Self::SHAPE),
        ("phase", Self::PHASE),
        ("amplitude", Self::AMPLITUDE),
        ("value", Self::OUT_VALUE),
    ];
    // register() and tick() unchanged except the call-site rules above.
```

`LFO`'s ordinal consts keep their current values (0–4) because it had no slots and its outputs already followed its inputs. `MidiNote` and `MidiCC` have event outputs, whose ordinals move from the separate event space into the one field space — renumber them and update `SignalNodesPlugin` if it names any.

- [ ] **Step 2: Give `Envelope` variadic event inlets and its own merge**

This is the one behavioural migration. `Envelope`'s two event inputs become variadic, and the merge the compiler used to perform moves into `tick`:

```rust
#[derive(Reflect, Component, Default)]
pub struct EnvelopeInlets {
    pub triggers: Vec<Events<NoteMsg>>,
    pub release_triggers: Vec<Events<NoteMsg>>,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Envelope {
    pub const TRIGGERS: u16 = 0;
    pub const RELEASE_TRIGGERS: u16 = 1;
    pub const ATTACK: u16 = 2;
    pub const DECAY: u16 = 3;
    pub const SUSTAIN: u16 = 4;
    pub const RELEASE: u16 = 5;
    pub const OUT_VALUE: u16 = 6;
}
```

Keep whatever fields `EnvelopeParams` has today for the four scalars; only the two event fields and the ordinals change.

Add this helper to `envelope.rs` and call it wherever `tick` currently iterates its trigger events:

```rust
/// Merges this node's trigger elements into one offset-ordered stream.
///
/// The engine used to do this, ordering sources by compiled rank and stable
/// sorting by offset. Element order now plays the part compiled rank did, and
/// the sort is still stable, so equal offsets resolve by element index —
/// which is what keeps the `event-fan-in` golden trace bit-identical.
fn merged(ports: &PortView, field: u16) -> Vec<(f32, NoteMsg)> {
    let mut merged: Vec<(f32, NoteMsg)> = Vec::new();
    for index in 0..ports.len(field) {
        for occurrence in ports.events_at::<NoteMsg>(field, index as u16) {
            merged.push((occurrence.offset, occurrence.value.clone()));
        }
    }
    merged.sort_by(|a, b| a.0.total_cmp(&b.0));
    merged
}
```

`tick`'s body changes only where it read the trigger streams: it now iterates `merged(ports, Self::TRIGGERS)` and `merged(ports, Self::RELEASE_TRIGGERS)` instead of `ports.events(EventIdx(..))`. Its envelope state machine is untouched.

- [ ] **Step 3: Migrate the material node**

In `material.rs`, `type Produces = MaterialOf<StandardMaterial>` becomes an outlet:

```rust
#[derive(Reflect, Default)]
pub struct StandardMaterialOutlets {
    pub material: Product<MaterialOf<StandardMaterial>>,
}
```

`StandardMaterialParams` → `StandardMaterialInlets` unchanged, `ORDINALS` lists its fields then `("material", <last>)`, and `register` calls `register_product::<MaterialOf<StandardMaterial>>(app)`. `MaterialOf<M>` itself is unchanged — it is still a bare capability marker.

- [ ] **Step 4: Migrate `Mesh` and the scene nodes**

`mesh.rs`:

```rust
#[derive(Reflect, Component)]
pub struct MeshNodeInlets {
    pub geo: Product<Geometry>,
    pub material: Product<MaterialOf<StandardMaterial>>,
    pub translation: Vec3,
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

#[derive(Reflect, Default)]
pub struct MeshNodeOutlets {
    pub spatial: Product<Spatial>,
}

impl MeshNode {
    pub const IN_GEO: u16 = 0;
    pub const IN_MATERIAL: u16 = 1;
    pub const TRANSLATION: u16 = 2;
    pub const ROTATION_X: u16 = 3;
    pub const ROTATION_Y: u16 = 4;
    pub const ROTATION_Z: u16 = 5;
    pub const SCALE: u16 = 6;
    pub const OUT_SPATIAL: u16 = 7;
}
```

Keep `MeshNodeInlets`'s hand-written `Default` (it sets `scale: Vec3::ONE`), adding `geo: Product::default()` and `material: Product::default()`. `SPATIAL: bool = true` is deleted. `register` calls `register_product` three times: `Geometry`, `MaterialOf<StandardMaterial>`, `Spatial`. `cook` reads `ports.source(Self::IN_GEO, 0)` and `ports.source(Self::IN_MATERIAL, 0)`; its `GeometryFingerprint` logic and its doc comment are unchanged.

`scene.rs`'s `Group` gains the variadic children inlet:

```rust
#[derive(Reflect, Component)]
pub struct GroupInlets {
    pub children: Vec<Product<Spatial>>,
    pub translation: Vec3,
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

#[derive(Reflect, Default)]
pub struct GroupOutlets {
    pub spatial: Product<Spatial>,
}
```

with `CHILDREN = 0` and the rest renumbered after it. `Rgb` is a plain rename of its two structs with its fields and ordinals unchanged, since it has neither slots nor a product.

- [ ] **Step 5: Update the trace harness**

In `crates/sway-nodes/tests/traces.rs`, `PortKindSpec::Continuous`/`NoteEvents` now both address a field ordinal in one space, so `snapshot_port` reads `plan.base + plan.field_offsets[ordinal]` and downcasts to `f32` or `Events<NoteMsg>` as before. `build_event_fan_in` connects its two `MidiNote` nodes to `Envelope::TRIGGERS` elements **0 and 1 in that order**, matching the compiled rank order the old engine derived — this is what makes the trace comparison meaningful rather than merely green.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p sway-graph -p sway-geo -p sway-nodes`
Expected: PASS, **including `event_fan_in` against its unmodified golden file.** If that trace differs, do not bless it — the difference is the merge semantics changing, which is precisely what this task must not do.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-nodes
git commit -m "$(cat <<'EOF'
feat(nodes): migrate every node type to Inlets/Outlets

Envelope's trigger inputs become variadic and it merges them itself; the
event-fan-in golden trace reproduces bit-identically, which is what proves
the merge moved from the engine to the node without changing semantics.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `sway-app` — the demo graph

**Files:**
- Modify: `crates/sway-app/src/demo_graph.rs`

- [ ] **Step 1: Rewrite the graph construction**

Replace the `param`/`feeds`/`parent` helpers with one:

```rust
fn edge(world: &mut World, from: Entity, from_field: u16, to: Entity, to_field: u16, to_index: u16) {
    world.spawn((
        Edge {
            from: Endpoint::field(from_field),
            to: Endpoint { field: to_field, index: to_index },
        },
        EdgeFrom(from),
        EdgeTo(to),
    ));
}
```

Every `XParams { .. }` in the spawn calls becomes `XInlets { .. }`. The `Group` spawn sizes its children: `GroupInlets { children: vec![Product::<Spatial>::default(); 1], .. }`, since the demo parents one `Mesh` under the root.

The wiring, unchanged in meaning:

```rust
edge(world, grid, Grid::OUT_GEO, displace, Displace::IN_GEO, 0);
edge(world, displace, Displace::OUT_GEO, mesh, MeshNode::IN_GEO, 0);
edge(world, material, StandardMaterialNode::OUT_MATERIAL, mesh, MeshNode::IN_MATERIAL, 0);
edge(world, mesh, MeshNode::OUT_SPATIAL, root, Group::CHILDREN, 0);
edge(world, rgb, Rgb::OUT_COLOR, material, StandardMaterialNode::BASE_COLOR, 0);
edge(world, midi_cc, MidiCC::OUT_VALUE, displace, Displace::AMOUNT, 0);
edge(world, envelope, Envelope::OUT_VALUE, rgb, Rgb::R, 0);
edge(world, midi_note, MidiNote::OUT_NOTE_ON, envelope, Envelope::TRIGGERS, 0);
edge(world, lfo, LFO::OUT_VALUE, root, Group::ROTATION_Y, 0);
```

Substitute the actual outlet const names for `Rgb` and `StandardMaterialNode` as renamed in Task 6. Note the parenting edge now runs `mesh → root.children[0]`, where before it was a `ParentEdge` with implicit ends: same direction, both ends now named.

- [ ] **Step 2: Update the module doc**

The ASCII diagram at the top of `demo_graph.rs` should name the fields, since they are now addressable:

```rust
//! Grid.geo ──→ Displace.geo ──→ Mesh.geo,  StandardMaterial.material ──→ Mesh.material
//! Mesh.spatial ──→ Group("root").children[0]
//! MidiCC 74.value ──→ Displace.amount
//! MidiNote.note_on ──→ Envelope.triggers[0].value ──→ Rgb.r
//! LFO.value ──→ Group("root").rotation_y
```

- [ ] **Step 3: Run everything but the editor**

Run: `cargo test --workspace --exclude sway-editor`
Expected: PASS, including `sway-app`'s own demo-graph tests and its ignored measurement tests' compilation.

- [ ] **Step 4: Verify by eye**

Run: `cargo run -p sway-app`
Expected: the displaced grid renders, rotates with the LFO, and reacts to MIDI exactly as before this milestone. Nothing in this work should be visible.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-app
git commit -m "$(cat <<'EOF'
feat(app): build the demo graph from one edge kind

The parenting edge now names both of its ends, like every other edge.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: The editor snapshot

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`, `crates/sway-editor/src/test_graph.rs`

**Interfaces:**
- Produces: `EdgeView { from: NodeId, from_field: u16, from_index: u16, to: NodeId, to_field: u16, to_index: u16, kind: EdgeKind, activity: Option<f32> }` and `EdgeKind { Value, Events, Product, Spatial }`, which Task 9's canvas draws.

- [ ] **Step 1: Write the failing tests**

In `snapshot.rs`'s test module, replace the edge tests with:

```rust
    #[test]
    fn every_edge_carries_both_of_its_endpoints() {
        let (app, ids) = fixture_with_parenting();
        let snap = capture(app.world());

        let parenting = snap
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Spatial)
            .expect("a parenting edge must appear in the snapshot");
        assert_eq!(parenting.from, ids.child);
        assert_eq!(parenting.to, ids.parent);
        // The canvas needs a socket at each end; before this milestone
        // parenting had neither and was dropped from the snapshot entirely.
        assert_eq!(parenting.to_index, 0, "children[0]");
    }

    #[test]
    fn edge_kinds_distinguish_what_an_edge_carries() {
        let (app, _) = fixture_with_parenting();
        let snap = capture(app.world());
        let kinds: std::collections::HashSet<_> = snap.edges.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EdgeKind::Value));
        assert!(kinds.contains(&EdgeKind::Product));
        assert!(kinds.contains(&EdgeKind::Spatial));
    }

    #[test]
    fn activity_is_some_only_for_an_f32_value_edge() {
        let (app, _) = fixture_with_parenting();
        let snap = capture(app.world());
        for edge in &snap.edges {
            match edge.kind {
                EdgeKind::Value => {}
                _ => assert!(
                    edge.activity.is_none(),
                    "only value edges carry a sampled value"
                ),
            }
        }
    }
```

`fixture_with_parenting` extends `test_graph.rs`'s existing builder with a `Group` and a spatial child, returning their `NodeId`s.

- [ ] **Step 2: Run them to make sure they fail**

Run: `cargo test -p sway-editor snapshot::`
Expected: FAIL to compile — `EdgeKind::Spatial` does not exist and `EdgeView` has no `to_index`.

- [ ] **Step 3: Rewrite `EdgeKind` and `EdgeView`**

```rust
/// What an edge carries, derived from the type of the inlet it lands on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EdgeKind {
    Value,
    Events,
    Product,
    /// A product edge whose capability is `Spatial` — parenting.
    Spatial,
}

pub struct EdgeView {
    pub from: NodeId,
    pub from_field: u16,
    pub from_index: u16,
    pub to: NodeId,
    pub to_field: u16,
    pub to_index: u16,
    pub kind: EdgeKind,
    /// The source slot's value, when it downcasts to `f32`. Events and
    /// products get none: an event occupies one tick and a frame-rate
    /// sampler would observe it at random, and a product is a reference.
    pub activity: Option<f32>,
}
```

`capture_edges` queries `(&Edge, &EdgeFrom, &EdgeTo)` — one query where there were three — resolves each end's `NodeId`, reads the target node's `FieldSpec` from its `NodePlan` to classify the kind, and samples `activity` from the source slot only when `kind == Value` and the slot downcasts to `f32`. Delete the comment on `EdgeKind` explaining why parenting is absent, and the `ParentEdge` skip at the former line 230.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-editor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-editor/src/snapshot.rs crates/sway-editor/src/test_graph.rs
git commit -m "$(cat <<'EOF'
feat(editor): the snapshot carries every edge, parenting included

An edge now has two addressable ends, so there is nothing left to invent and
nothing left to drop.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: Sockets on the canvas

**Files:**
- Modify: `crates/sway-editor/src/canvas.rs`, `crates/sway-editor/src/node_box.rs`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_node_box_lays_out_one_socket_per_slot() {
        // Inlet counts are per-instance now, so sockets come from the
        // snapshot rather than from a node type.
        let mut harness = TestHarness::create(canvas_with(vec![
            NodeView { id: NodeId(0), name: "Group".into(), inlets: vec![2, 1], outlets: 1, .. },
        ]));
        let box_id = /* the NodeBox widget id for NodeId(0) */;
        harness.edit_widget(box_id, |node_box| {
            let node_box = node_box.downcast::<NodeBox>();
            assert_eq!(NodeBox::inlet_socket_count(&node_box), 3, "2 children + 1 scalar");
            assert_eq!(NodeBox::outlet_socket_count(&node_box), 1);
        });
    }
```

`NodeView` gains `inlets: Vec<u16>` (slot count per inlet field) and `outlets: u16`, both filled by `capture` in Task 8's `capture_nodes` from the same `NodePlan::field_lens` the compiler produced.

- [ ] **Step 2: Run it to make sure it fails**

Run: `cargo test -p sway-editor canvas::`
Expected: FAIL — `NodeView` has no `inlets` field.

- [ ] **Step 3: Draw sockets and route edges to them**

`NodeBox` grows a socket row: inlet sockets along its left edge, outlet sockets along its right, evenly spaced, one per slot. `NodeBox::inlet_socket_pos(field, index) -> Point` and `outlet_socket_pos(field) -> Point` return canvas-space positions, and `GraphCanvas`'s bezier routing uses them as endpoints instead of the box centres.

Edges paint by kind:

```rust
fn edge_color(kind: EdgeKind) -> Color {
    match kind {
        EdgeKind::Value => Color::from_rgb8(140, 140, 155),
        EdgeKind::Events => Color::from_rgb8(150, 130, 170),
        EdgeKind::Product => Color::from_rgb8(120, 165, 140),
        EdgeKind::Spatial => Color::from_rgb8(170, 150, 110),
    }
}
```

Keep the auto-ranging activity normalisation exactly as it is; it now applies only to `EdgeKind::Value`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-editor`
Expected: PASS, **including M1b's zoom hit-test and pan regressions**, which are the gate assertions for the masonry bet and must not regress.

- [ ] **Step 5: Verify by eye**

Run: `cargo run -p sway-app -- --editor`
Expected: the canvas shows the parenting edge from `Mesh` to `Group("root")`, every edge starts and ends at a socket, and the `Group`'s `children` field shows as many inlet sockets as it has elements.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-editor
git commit -m "$(cat <<'EOF'
feat(editor): sockets on both ends of every edge

Parenting appears on the canvas for the first time. Socket counts come from
the snapshot, because a Vec inlet's slot count is per instance.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: Close out

**Files:**
- Modify: `docs/superpowers/specs/2026-07-25-sway-design.md`
- Create: `docs/superpowers/reports/2026-08-03-unified-edges-findings.md`

- [ ] **Step 1: Full workspace verification**

Run: `cargo test --workspace`
Expected: PASS.

Run: `cargo clippy -p sway-graph -p sway-geo -p sway-nodes -p sway-editor --all-targets -- -D warnings`
Expected: clean.

If `sway-midi`'s `virtual_destination_receives_midisend_note_on` fails, re-run before investigating: it is a CoreMIDI loopback test with a wall-clock deadline, known flaky under parallel load, and unrelated to this work.

- [ ] **Step 2: Check the dependency constraints still hold**

Run: `cargo tree -p sway-graph -e normal --depth 1`
Expected: `bevy_app`, `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_time`, `bevy_transform` and nothing else. No `bevy`, no `bevy_render`.

Run: `cargo tree -p sway-editor -e normal --depth 1`
Expected: no `bevy`, `bevy_render`, `wgpu`, `vello` or `imaging_vello`.

- [ ] **Step 3: Apply the parent spec's revisions**

Design §9 lists six sections of `docs/superpowers/specs/2026-07-25-sway-design.md` this work invalidates. Apply each:

- **§2.4** — variable arity is declared, not designed out; the port type registry subsumes capabilities; a registry entry is constant per type except for one per-instance count.
- **§2.5** — one pass and one sort; the structure/dataflow split becomes "everything except `Spatial` is a dependency".
- **§2.10** — the three-edge-kind table becomes one edge; `Feeds` and `ChildOf` become inlet types.
- **§2.11** — steps A and C merge; the arena carries entity references as well as signals.
- **§7** — strike the same-tick cook dependency question; it is closed.
- **§5** — record that M4 opened with this work.

Add a **Revision** line at the top of the parent spec in the style the existing ones use.

- [ ] **Step 4: Write the findings report**

Answer design §13's five questions in `docs/superpowers/reports/2026-08-03-unified-edges-findings.md`, following the house style of the M2a and M2b reports: state what was measured and the exact command that produced it, and keep a "What was not proven" section. The five questions are the allocation profile of clear-in-place, whether one order cost anything real, how `(field, index)` read at the call site, whether `Product`-as-entity-reference removed the capability system cleanly, and what `Spatial`'s three special behaviours cost.

- [ ] **Step 5: Commit**

```bash
git add docs
git commit -m "$(cat <<'EOF'
docs: unified edges findings, and the parent spec's revisions

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Self-review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §2 model — three type shapes | 1 (types), 2 (derivation), 4 (arena, view) |
| §2 multiplicity — `Vec` fields | 2 (variadic detection), 4 (per-instance layout, `Sum` fixture) |
| §2 addressing — `(field, index)` | 3, 4 (compiler resolution), 8 (snapshot), 9 (sockets) |
| §2 node contract — two structs, one `ORDINALS` | 4 (trait, guard), 5–6 (every node) |
| §2 ≤1 product outlet, no `Vec` outlets | 4 (`check_outlets`) |
| §3 occurrence lists cleared before each tick | 1 (`clear_events_of`), 2 (`ReflectEventList`), 4 (runner) |
| §3 `Spatial` — `ChildOf`, single-consumer, excluded from order | 4 (all three, one test each) |
| §4 one pass, one sort | 4 |
| §4 union cycles rejected | 4 (`a_union_cycle_across_both_old_dags_is_rejected`) |
| §4 error vocabulary preserved | 4 step 9's table |
| §5 tick and cook in one pass | 4 (runner) |
| §6 editor draws every edge socket-to-socket | 8, 9 |
| §7 deletions | 4 step 8, 5, 6 |
| §8 event allocation risk | 1 (capacity test), 10 (measured in findings) |
| §9 parent spec revisions | 10 step 3 |
| §10 placement, no shim | the flip-window table |
| §11 testing | 4 step 9, 6 step 6, 8, 9 |
| §13 findings report | 10 step 4 |

**Placeholder scan.** One deliberate ellipsis remains, in Task 9 step 1 (`let box_id = /* … */`), because the widget-id lookup depends on `canvas.rs`'s existing test helper, which the implementer will have open. Everything else carries its code.

**Type consistency.** `Inlets`/`Outlets`/`State`/`ORDINALS`/`COOKS` are used identically in Tasks 4–6. `PortView::{read, read_at, write, write_at, len, events, events_at, emit, source, is_connected}` are defined in Task 4 step 3 and called at exactly those signatures in Tasks 4–6. `NodePlan`'s fields are defined in Task 4 step 5 and read in step 7, in `prefill_of`/`seed_outlets_of` (step 4), and by `port_value`/`event_offsets` (step 1). `FieldKind`/`FieldSpec`/`ProductAccess` are defined in Task 2 and consumed unchanged in Task 4. `EdgeKind` gains `Spatial` in Task 8 and is matched exhaustively in Task 9's `edge_color`.

**Known gap, stated rather than hidden.** Task 4 is large — six files in one commit — and no smaller decomposition keeps `sway-graph` compiling, because `NodeType`, the arena, the view and the compiler are mutually dependent. Its steps are individually small, and step 9's table is the acceptance criterion. If the implementer wants a checkpoint, the natural one is after step 7, when the engine compiles and only the test suites are outstanding.

