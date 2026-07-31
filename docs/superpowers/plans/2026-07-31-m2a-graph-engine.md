# M2a Graph Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `sway-graph` (the node engine: port arena, reflect-derived schema, node type registry, param edges, dataflow compiler, `FixedUpdate` tick runner) and `sway-nodes` (eight signal node types), then retire M0's hardcoded graph behind them.

**Architecture:** A node type is plugin-shaped and a node instance is an entity carrying `Params` + `State` components. Registration erases `NodeType::tick` to a bare `fn` in a registry. The compiler reads the world, validates param edges, topologically sorts, and produces a flat plan plus an arena layout. One exclusive system in `FixedUpdate` walks that plan: gather edges, prefill unconnected inputs from `Params`, dispatch. Port values are `Box<dyn PartialReflect>` in a two-collection arena — continuous slots persist, event slots clear each tick.

**Tech Stack:** Rust 2024 edition, bevy 0.19.0 (`bevy_app`/`bevy_ecs`/`bevy_reflect`/`bevy_time` only — **not** the `bevy` facade), `ron` 0.12 for the trace harness, existing `sway-midi` for CoreMIDI input.

## Read this before Task 1

**The spec is `docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md`.** It is the authority; this plan is its mechanical expansion. Where they disagree, the spec wins and this plan gets fixed.

### API facts, verified against the vendored sources

Every signature below was read out of `~/.cargo/registry/src/*/bevy_{reflect,ecs}-0.19.0/src` while writing this plan. Do not guess replacements for them; if one is wrong, read the source at the cited path.

| What | Signature | Source |
|---|---|---|
| Clone a reflected value | `PartialReflect::to_dynamic(&self) -> Box<dyn PartialReflect>` | `bevy_reflect/src/reflect.rs:277` |
| Overwrite in place | `PartialReflect::apply(&mut self, value: &dyn PartialReflect)` (panics on mismatch) | `reflect.rs:206` |
| Fallible overwrite | `PartialReflect::try_apply(&mut self, &dyn PartialReflect) -> Result<(), ApplyError>` | `reflect.rs:219` |
| Downcast | `<dyn PartialReflect>::try_downcast_ref<T: Any>(&self) -> Option<&T>` | `reflect.rs:494` |
| Struct field access | `Struct::field_at(&self, index: usize) -> Option<&dyn PartialReflect>` | `structs.rs:62` |
| Struct field name | `Struct::name_at(&self, index: usize) -> Option<&str>` | `structs.rs:69` |
| Static type info | `Typed::type_info() -> &'static TypeInfo` | `type_info.rs:99` |
| Narrow to struct | `TypeInfo::as_struct(&self) -> Result<&StructInfo, TypeInfoError>` | `type_info.rs:365` |
| Field metadata | `StructInfo::field_at(usize) -> Option<&NamedField>`, `field_len()`, `iter()` | `structs.rs:172,187,182` |
| Field name / type | `NamedField::name() -> &'static str`, `.type_id() -> TypeId`, `.type_path() -> &'static str` | `fields.rs:49`, `type_info.rs:538,548` |
| Type data lookup | `TypeRegistry::get_type_data<T: TypeData>(&self, TypeId) -> Option<&T>` | `type_registry.rs:524` |
| Type data insert | `TypeRegistry::register_type_data<T: Reflect + TypePath, D: TypeData + FromType<T>>(&mut self)` | `type_registry.rs:343` |
| Take a resource out | `World::resource_scope<R: Resource, U>(&mut self, impl FnOnce(&mut World, Mut<R>) -> U) -> U` | `world/mod.rs:2851` |
| Change ticks | `EntityRef::get_change_ticks<T: Component>(&self) -> Option<ComponentTicks>` | `world/entity_access/entity_ref.rs:140` |
| Tick fields | `ComponentTicks { pub added: Tick, pub changed: Tick }` | `change_detection/tick.rs:137` |
| Relationship | `#[relationship(relationship_target = X)]` on `struct Foo(#[entities] pub Entity)` | `hierarchy.rs:105` |
| Relationship target | `#[relationship_target(relationship = Foo, linked_spawn)]` on `struct Bar(Vec<Entity>)` | `hierarchy.rs:148` |

`linked_spawn` is what makes "despawning a node despawns its edges" (spec §5) true without any code: the node holds the `RelationshipTarget` collections, the edge holds the `Relationship` components.

### The one thing genuinely unknown

**Whether `#[derive(Reflect)]` accepts a generic marker struct with an ignored `PhantomData`.** Task 2 needs `Event<T>` to be `Reflect` so it can appear as a field in a `Params`/`Outputs` struct. The intended form is:

```rust
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default)]
pub struct Event<T: Reflect + TypePath> {
    #[reflect(ignore)]
    _marker: PhantomData<fn() -> T>,
}
```

If the derive rejects this, **read its error and the derive's source rather than inventing a workaround.** The documented fallback is a non-generic `EventPort` unit marker plus a `ReflectEventPort` type data carrying the payload `TypeId`, registered per payload type — the schema code in Task 2 reads the payload from the type data either way, so only the field type changes. Record whichever happened in the Task 10 findings report; this is exactly the "reflect ergonomics" question spec §11 says M2a exists to answer.

## Global Constraints

- **Rust edition 2024**, resolver 3, matching the existing workspace.
- **bevy is pinned `=0.19.0`** in `[workspace.dependencies]`. Do not change the pin.
- **`sway-graph` depends on `bevy_app`, `bevy_ecs`, `bevy_reflect`, `bevy_time` only.** Not the `bevy` facade, not `bevy_render`, and not `bevy_transform`/`bevy_asset` — those join at M2b. The manifest is the only place this constraint is enforced (spec §2).
- **`sway-graph` contains no MIDI code.** `MidiNote`/`MidiCC` live in `sway-nodes` (spec §2).
- **`sway-nodes` does not depend on `sway-midi`.** It defines its own `RawMidi`; `sway-app` converts (spec §7).
- **Do not touch** `crates/sway-runtime/`, `crates/sway-gpu/`, `crates/sway-editor/`. M2a's only change outside the two new crates is Task 9's `sway-app` handover.
- **Nodes derive time-varying values from absolute time; never accumulate per tick** (spec §6). `crates/sway-app/src/graph.rs:53` documents its own violation of this — do not copy that pattern forward.
- **No node fires observer triggers at M2a** (spec §6).
- **All failure happens at compile; the tick is infallible** (spec §5). No `Result` in the tick path.
- Every compiler error message **names the offending node** (spec §5).
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` must be green at every commit.

## File Structure

```
crates/sway-graph/
  Cargo.toml
  src/lib.rs          re-exports; the crate's public surface
  src/ports.rs        ContinuousIdx, EventIdx, Occurrence, PortArena, Event<T>
  src/schema.rs       ReflectEventPort, PortField, SchemaHalf, derive_schema
  src/registry.rs     NodeType, NodeTypeId, NodeTypeEntry, NodeTypeRegistry, register_node_type
  src/edges.rs        ParamEdge, EdgeFrom/EdgeTo, InEdges/OutEdges, GraphNode, NodeRuntime
  src/compile.rs      CompileError, NodePlan, CompiledGraph, compile
  src/view.rs         PortView, TickCtx
  src/tick.rs         graph_tick, GraphPlugin

crates/sway-nodes/
  Cargo.toml
  src/lib.rs          SignalNodesPlugin; re-exports
  src/midi.rs         RawMidi, NoteMsg, MidiInbox, MidiNote, MidiCC
  src/lfo.rs          LFO
  src/envelope.rs     Envelope
  src/math.rs         Math, Remap, Switch, Select
  tests/traces.rs     the golden-trace harness
  tests/traces/*.ron  recorded cases

crates/sway-app/
  src/main.rs         modified: graph construction + MidiInbox feed
  src/bridge.rs       new: arena -> cube material (throwaway, deleted at M2b)
  src/graph.rs        DELETED
```

One file per responsibility, none expected past ~350 lines. `compile.rs` and `tick.rs` are the two that carry real logic; everything else is types and derivation.

---

### Task 1: `sway-graph` crate and the port arena

**Files:**
- Create: `crates/sway-graph/Cargo.toml`, `crates/sway-graph/src/lib.rs`, `crates/sway-graph/src/ports.rs`
- Modify: `Cargo.toml` (workspace members + dependencies)

**Interfaces:**
- Produces: `ContinuousIdx(pub u32)`, `EventIdx(pub u32)`, `Occurrence { offset: f32, value: Box<dyn PartialReflect> }`, `PortArena { continuous: Vec<Box<dyn PartialReflect>>, events: Vec<Vec<Occurrence>> }` with `PortArena::new(continuous_len, events_len)`, `clear_events(&mut self)`, `resize(&mut self, continuous_len, events_len)`.

- [ ] **Step 1: Add the crate to the workspace**

In the root `Cargo.toml`, add `"crates/sway-graph"` to `members`, and under `[workspace.dependencies]` add:

```toml
bevy_app = "=0.19.0"
bevy_ecs = "=0.19.0"
bevy_reflect = "=0.19.0"
bevy_time = "=0.19.0"
sway-graph = { path = "crates/sway-graph" }
```

- [ ] **Step 2: Create the manifest**

`crates/sway-graph/Cargo.toml`:

```toml
[package]
name = "sway-graph"
version.workspace = true
edition.workspace = true

# Spec §2: bevy_app/ecs/reflect/time only. NOT the `bevy` facade, NOT
# bevy_render. bevy_transform and bevy_asset join at M2b when scene nodes
# need them; this manifest is the only place that constraint is enforced.
[dependencies]
bevy_app.workspace = true
bevy_ecs.workspace = true
bevy_reflect.workspace = true
bevy_time.workspace = true
```

- [ ] **Step 3: Write the failing test**

`crates/sway-graph/src/ports.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_slots_persist_across_event_clears() {
        let mut arena = PortArena::new(2, 1);
        arena.continuous[0] = Box::new(0.75_f32);
        arena.events[0].push(Occurrence { offset: 0.004, value: Box::new(7_u8) });

        arena.clear_events();

        // Spec §4: continuous persists, events clear. This is what makes
        // "CC is 0" distinguishable from "no CC arrived".
        assert_eq!(
            arena.continuous[0].try_downcast_ref::<f32>().copied(),
            Some(0.75)
        );
        assert!(arena.events[0].is_empty());
    }

    #[test]
    fn clearing_events_retains_allocation() {
        let mut arena = PortArena::new(0, 1);
        for i in 0..16 {
            arena.events[0].push(Occurrence { offset: i as f32, value: Box::new(i) });
        }
        let cap = arena.events[0].capacity();

        arena.clear_events();

        // Spec §4 claims per-tick event churn goes to zero after warm-up.
        // That is only true if clear() keeps the buffer.
        assert!(arena.events[0].capacity() >= cap);
    }

    #[test]
    fn resize_preserves_existing_continuous_values() {
        // Recompilation resizes the arena; a graph that grew must not lose
        // the values of the nodes that survived.
        let mut arena = PortArena::new(1, 0);
        arena.continuous[0] = Box::new(3.5_f32);

        arena.resize(3, 2);

        assert_eq!(
            arena.continuous[0].try_downcast_ref::<f32>().copied(),
            Some(3.5)
        );
        assert_eq!(arena.continuous.len(), 3);
        assert_eq!(arena.events.len(), 2);
    }
}
```

- [ ] **Step 4: Run it to verify it fails**

Run: `cargo test -p sway-graph`
Expected: FAIL — `PortArena` not found.

- [ ] **Step 5: Implement `ports.rs`**

```rust
//! The port arena: where signal values live between nodes.
//!
//! Spec §4. Two collections, not one enum: nothing iterates slots
//! kind-agnostically, so an enum would buy a discriminant and a match arm at
//! every access and nothing else.

use bevy_ecs::resource::Resource;
use bevy_reflect::PartialReflect;

/// Index of a continuous port, absolute within [`PortArena::continuous`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct ContinuousIdx(pub u32);

/// Index of an event port, absolute within [`PortArena::events`].
///
/// A distinct newtype from [`ContinuousIdx`] so that reading a continuous
/// port as an event stream is a type error rather than a runtime panic.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct EventIdx(pub u32);

/// One event occurrence, stamped with its offset inside the tick window.
///
/// `offset` is seconds from the tick's start, so it is bounded by the
/// timestep (~8.3ms at 120Hz) and f32 has precision to spare. A node needing
/// absolute time writes `ctx.tick_start + offset as f64` (spec §7).
pub struct Occurrence {
    pub offset: f32,
    pub value: Box<dyn PartialReflect>,
}

#[derive(Resource)]
pub struct PortArena {
    /// Persists across ticks — a continuous port always holds a current value.
    pub continuous: Vec<Box<dyn PartialReflect>>,
    /// Cleared at tick start — zero or more occurrences for *this* tick only.
    pub events: Vec<Vec<Occurrence>>,
}

impl PortArena {
    pub fn new(continuous_len: usize, events_len: usize) -> Self {
        Self {
            continuous: (0..continuous_len)
                .map(|_| Box::new(()) as Box<dyn PartialReflect>)
                .collect(),
            events: (0..events_len).map(|_| Vec::new()).collect(),
        }
    }

    /// Clears every event slot, retaining each vec's allocation.
    pub fn clear_events(&mut self) {
        for slot in &mut self.events {
            slot.clear();
        }
    }

    /// Grows or shrinks to a new compiled layout, keeping the continuous
    /// values that still have a slot. Recompilation calls this.
    pub fn resize(&mut self, continuous_len: usize, events_len: usize) {
        self.continuous
            .resize_with(continuous_len, || Box::new(()) as Box<dyn PartialReflect>);
        self.events.resize_with(events_len, Vec::new);
    }
}
```

Note the placeholder value: a freshly-allocated continuous slot holds `()` until its first prefill or edge copy. That is deliberate — an unwritten slot must be visibly wrong if it is ever read, rather than a plausible `0.0`.

`crates/sway-graph/src/lib.rs`:

```rust
//! The sway graph engine. Spec: docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md

pub mod ports;

pub use ports::{ContinuousIdx, EventIdx, Occurrence, PortArena};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, 3 tests.

- [ ] **Step 7: Verify the workspace still builds**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green, with the pre-existing test count plus 3.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sway-graph
git commit -m "feat(graph): sway-graph crate and the port arena"
```

---

### Task 2: `Event<T>`, the event-port type data, and schema derivation

**Files:**
- Create: `crates/sway-graph/src/schema.rs`
- Modify: `crates/sway-graph/src/ports.rs` (add `Event<T>`), `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `ContinuousIdx`, `EventIdx` from Task 1.
- Produces:
  - `Event<T>` — the marker field type that makes a field an event port.
  - `ReflectEventPort { payload: TypeId, payload_path: &'static str }` — type data marking a type as an event port.
  - `register_event_port::<T>(app: &mut App)`.
  - `PortField { name: &'static str, field_index: usize, type_id: TypeId, type_path: &'static str }`
  - `SchemaHalf { continuous: Vec<PortField>, events: Vec<PortField> }`
  - `derive_schema<T: Typed>(registry: &TypeRegistry) -> Result<SchemaHalf, SchemaError>`
  - `SchemaError` with `Display`.

- [ ] **Step 1: Add `Event<T>` to `ports.rs`**

Append to `crates/sway-graph/src/ports.rs`:

```rust
use core::marker::PhantomData;
use bevy_reflect::{Reflect, TypePath};

/// Marks a `Params`/`Outputs` field as an **event** port.
///
/// Zero-sized: the occurrences live in [`PortArena::events`], not in the
/// struct. An event input has no authored value (spec §3), which is why this
/// carries no data — there is nothing for an author to write.
///
/// `PhantomData<fn() -> T>` rather than `PhantomData<T>` so the marker is
/// `Send + Sync + Default` regardless of `T`.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default)]
pub struct Event<T: Reflect + TypePath> {
    #[reflect(ignore)]
    _marker: PhantomData<fn() -> T>,
}

impl<T: Reflect + TypePath> Default for Event<T> {
    fn default() -> Self {
        Self { _marker: PhantomData }
    }
}
```

**If the derive rejects this, stop and read its error** — see "The one thing genuinely unknown" above for the sanctioned fallback and the requirement to record which one happened.

- [ ] **Step 2: Write the failing test**

`crates/sway-graph/src/schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Event;
    use bevy_reflect::{Reflect, TypeRegistry};

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg { note: u8, velocity: u8 }

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

        // Spec §3: field type decides kind. Spec §4: ordinals are per-kind,
        // so `amplitude` is continuous #1 even though it is field #2.
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

        // Edge validation compares payloads, so the schema must report
        // NoteMsg here — not Event<NoteMsg>.
        assert_eq!(s.events[0].type_id, core::any::TypeId::of::<NoteMsg>());
        assert_ne!(s.events[0].type_id, core::any::TypeId::of::<Event<NoteMsg>>());
    }

    #[test]
    fn an_unregistered_event_field_is_an_error_not_a_continuous_port() {
        // The failure this prevents: a node author adds an Event<T> field but
        // forgets register_event_port, and it silently becomes a continuous
        // port of a zero-sized type that no edge can ever usefully drive.
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<MixedParams>();
        r.register::<Event<NoteMsg>>();
        // deliberately NOT register_type_data::<_, ReflectEventPort>

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
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p sway-graph schema`
Expected: FAIL — `derive_schema` not found.

- [ ] **Step 4: Implement `schema.rs`**

```rust
//! Deriving a node's port schema from its `Params` / `Outputs` types.
//!
//! Spec §3: the schema is derived from the types, never written beside them.
//! A plain field is a continuous port; a field typed `Event<T>` is an event
//! port whose payload is `T`.

use core::any::TypeId;
use core::fmt;

use bevy_app::App;
use bevy_reflect::{FromType, Reflect, TypeInfo, TypePath, TypeRegistry, Typed};

use crate::ports::Event;

/// Type data marking a type as an event-port marker, and carrying the
/// payload type the port actually transports.
#[derive(Clone)]
pub struct ReflectEventPort {
    pub payload: TypeId,
    pub payload_path: &'static str,
}

impl<T: Reflect + TypePath> FromType<Event<T>> for ReflectEventPort {
    fn from_type() -> Self {
        Self {
            payload: TypeId::of::<T>(),
            payload_path: T::type_path(),
        }
    }
}

/// Registers `Event<T>` and its `ReflectEventPort` data. A node type with an
/// `Event<T>` port must call this in its `register`.
pub fn register_event_port<T: Reflect + TypePath + Typed + bevy_reflect::FromReflect + bevy_reflect::GetTypeRegistration>(
    app: &mut App,
) {
    let registry = app.world().resource::<bevy_ecs::reflect::AppTypeRegistry>().clone();
    let mut registry = registry.write();
    registry.register::<T>();
    registry.register::<Event<T>>();
    registry.register_type_data::<Event<T>, ReflectEventPort>();
}

/// One port, as derived from one struct field.
#[derive(Clone, Debug, PartialEq)]
pub struct PortField {
    pub name: &'static str,
    /// Index of the field in the reflect struct — what `Struct::field_at`
    /// takes. Not the port ordinal, which is per-kind.
    pub field_index: usize,
    /// For a continuous port, the field's type. For an event port, the
    /// *payload* type, so edge validation compares like with like.
    pub type_id: TypeId,
    pub type_path: &'static str,
}

/// The ports derived from one struct — either a node's inputs (`Params`) or
/// its outputs (`Outputs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SchemaHalf {
    pub continuous: Vec<PortField>,
    pub events: Vec<PortField>,
}

#[derive(Debug)]
pub enum SchemaError {
    NotAStruct { type_path: &'static str },
    UnregisteredEventField { type_path: &'static str, field: &'static str },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAStruct { type_path } => write!(
                f,
                "`{type_path}` must be a struct to derive a port schema from it"
            ),
            Self::UnregisteredEventField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like an event port but its type is not \
                 registered as one — call `register_event_port::<Payload>(app)` in \
                 this node type's `register`"
            ),
        }
    }
}

impl core::error::Error for SchemaError {}

pub fn derive_schema<T: Typed>(registry: &TypeRegistry) -> Result<SchemaHalf, SchemaError> {
    let info = T::type_info();
    let s = info.as_struct().map_err(|_| SchemaError::NotAStruct {
        type_path: info.type_path(),
    })?;

    let mut half = SchemaHalf::default();
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");
        match registry.get_type_data::<ReflectEventPort>(field.type_id()) {
            Some(ev) => half.events.push(PortField {
                name: field.name(),
                field_index: i,
                type_id: ev.payload,
                type_path: ev.payload_path,
            }),
            None => {
                // An `Event<_>` field whose type data is missing would land
                // here and become a continuous port of a zero-sized type,
                // which no edge could usefully drive. Catch it by type path
                // rather than letting it through silently.
                if field.type_path().starts_with(concat!(module_path!(), "::")) {
                    // unreachable in practice; kept for symmetry
                }
                if is_event_marker_path(field.type_path()) {
                    return Err(SchemaError::UnregisteredEventField {
                        type_path: info.type_path(),
                        field: field.name(),
                    });
                }
                half.continuous.push(PortField {
                    name: field.name(),
                    field_index: i,
                    type_id: field.type_id(),
                    type_path: field.type_path(),
                });
            }
        }
    }
    Ok(half)
}

/// Recognises `sway_graph::ports::Event<..>` by path.
///
/// This exists only to turn a forgotten `register_event_port` into a clear
/// error instead of a silently useless port. The *authoritative* test for an
/// event port is the `ReflectEventPort` type data above; this is the
/// diagnostic for its absence.
fn is_event_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Event<")
}
```

Delete the vestigial `module_path!` branch shown above when implementing — it is in the listing only to mark where the temptation to path-match sits. The real logic is `is_event_marker_path`.

- [ ] **Step 5: Export from `lib.rs`**

```rust
pub mod ports;
pub mod schema;

pub use ports::{ContinuousIdx, Event, EventIdx, Occurrence, PortArena};
pub use schema::{derive_schema, register_event_port, PortField, ReflectEventPort, SchemaError, SchemaHalf};
```

Add `bevy_app` usage requires no manifest change — it is already a dependency from Task 1.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, 7 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): Event<T> ports and reflect-derived port schema"
```

---

### Task 3: The `NodeType` contract, the registry, and the ordinal check

**Files:**
- Create: `crates/sway-graph/src/registry.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `SchemaHalf`, `derive_schema` (Task 2); `PortArena` (Task 1).
- Produces:
  - `NodeTypeId(pub u32)`
  - `trait NodeType` with `Params`, `Outputs`, `State`, `PORT_ORDINALS`, `register`, `tick`
  - `NodeSchema { inputs: SchemaHalf, outputs: SchemaHalf }` with `continuous_len()`, `events_len()`
  - `NodeTypeEntry { name, schema, tick, prefill, params_changed_tick }`
  - `NodeTypeRegistry` resource with `get(NodeTypeId)`, `id_of(&str)`
  - `register_node_type::<N>(app: &mut App) -> NodeTypeId`

The erased function pointers are declared here and consumed by Task 5's runner:

```rust
pub type TickFn    = fn(&mut World, Entity, &mut PortView, &TickCtx);
pub type PrefillFn = fn(&World, Entity, &mut PortArena, &NodePlan);
pub type TickOfFn  = fn(&World, Entity) -> Option<Tick>;
```

`PortView`, `TickCtx` and `NodePlan` are defined in Tasks 4 and 5; declare them as forward `use` items and let this task's tests use a stub node whose `tick` body is empty.

- [ ] **Step 1: Write the failing test**

`crates/sway-graph/src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Event;
    use bevy_app::App;
    use bevy_ecs::component::Component;
    use bevy_reflect::Reflect;

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg { note: u8, velocity: u8 }

    #[derive(Reflect, Component, Default)]
    struct ProbeParams { gain: f32, trigger: Event<NoteMsg>, bias: f32 }

    #[derive(Reflect, Default)]
    struct ProbeOut { value: f32 }

    #[derive(Component, Default)]
    struct ProbeState;

    struct Probe;

    impl Probe {
        const GAIN: u16 = 0;
        const BIAS: u16 = 1;
        const OUT_VALUE: u16 = 2; // inputs then outputs, within the kind
        const TRIGGER: u16 = 0;   // event space is separate
    }

    impl NodeType for Probe {
        type Params = ProbeParams;
        type Outputs = ProbeOut;
        type State = ProbeState;

        const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
            ("gain", Probe::GAIN),
            ("bias", Probe::BIAS),
            ("value", Probe::OUT_VALUE),
            ("trigger", Probe::TRIGGER),
        ];

        fn register(app: &mut App) {
            crate::schema::register_event_port::<NoteMsg>(app);
        }

        fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
    }

    #[test]
    fn registration_derives_both_halves_and_lays_out_ordinals_per_kind() {
        let mut app = App::new();
        let id = register_node_type::<Probe>(&mut app);
        let reg = app.world().resource::<NodeTypeRegistry>();
        let entry = reg.get(id).expect("registered");

        assert_eq!(entry.schema.inputs.continuous.len(), 2);
        assert_eq!(entry.schema.inputs.events.len(), 1);
        assert_eq!(entry.schema.outputs.continuous.len(), 1);
        // Spec §4: continuous inputs then continuous outputs, contiguous.
        assert_eq!(entry.schema.continuous_len(), 3);
        assert_eq!(entry.schema.events_len(), 1);
    }

    #[test]
    fn a_wrong_ordinal_fails_registration_and_names_the_field() {
        struct Bad;
        impl NodeType for Bad {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type State = ProbeState;
            // "bias" is continuous #1, not #0. This is exactly the mistake a
            // field reorder makes, and it must not reach the tick loop.
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
                ("gain", 0), ("bias", 0), ("value", 2), ("trigger", 0),
            ];
            fn register(app: &mut App) { crate::schema::register_event_port::<NoteMsg>(app); }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Bad>(&mut app)
        }))
        .unwrap_err();
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("bias"), "must name the field: {msg}");
        assert!(msg.contains('1'), "must state the correct ordinal: {msg}");
    }

    #[test]
    fn a_missing_ordinal_declaration_fails_registration() {
        // The other half of the guard: declaring fewer consts than there are
        // ports means some port has no name in node code at all.
        struct Incomplete;
        impl NodeType for Incomplete {
            type Params = ProbeParams;
            type Outputs = ProbeOut;
            type State = ProbeState;
            const PORT_ORDINALS: &'static [(&'static str, u16)] = &[("gain", 0)];
            fn register(app: &mut App) { crate::schema::register_event_port::<NoteMsg>(app); }
            fn tick(_w: &mut World, _n: Entity, _p: &mut PortView, _t: &TickCtx) {}
        }

        let mut app = App::new();
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register_node_type::<Incomplete>(&mut app)
        }))
        .unwrap_err();
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("bias") || msg.contains("undeclared"), "{msg}");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-graph registry`
Expected: FAIL — `NodeType` not found.

- [ ] **Step 3: Implement `registry.rs`**

```rust
//! The node type registry. Spec §3.
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
use bevy_reflect::{Reflect, Struct, Typed};
use std::collections::HashMap;

use crate::compile::NodePlan;
use crate::ports::PortArena;
use crate::schema::{derive_schema, SchemaHalf};
use crate::view::{PortView, TickCtx};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeTypeId(pub u32);

pub type TickFn = fn(&mut World, Entity, &mut PortView, &TickCtx);
pub type PrefillFn = fn(&World, Entity, &mut PortArena, &NodePlan);
pub type TickOfFn = fn(&World, Entity) -> Option<Tick>;

pub trait NodeType: 'static {
    type Params: Reflect + Typed + Component;
    type Outputs: Reflect + Typed;
    type State: Component + Default;

    /// `(field name, the ordinal the node's index const uses)` for every
    /// port. Verified against the reflect-derived schema at registration, so
    /// a field reorder fails at startup instead of silently swapping two
    /// ports (spec §3).
    const PORT_ORDINALS: &'static [(&'static str, u16)];

    fn register(app: &mut App);
    fn tick(world: &mut World, node: Entity, ports: &mut PortView, t: &TickCtx);
}

#[derive(Clone, Debug, Default)]
pub struct NodeSchema {
    pub inputs: SchemaHalf,
    pub outputs: SchemaHalf,
}

impl NodeSchema {
    /// Continuous inputs then continuous outputs, contiguous (spec §4).
    pub fn continuous_len(&self) -> usize {
        self.inputs.continuous.len() + self.outputs.continuous.len()
    }
    pub fn events_len(&self) -> usize {
        self.inputs.events.len() + self.outputs.events.len()
    }
}

pub struct NodeTypeEntry {
    pub name: &'static str,
    pub schema: NodeSchema,
    pub tick: TickFn,
    pub prefill: PrefillFn,
    pub params_changed_tick: TickOfFn,
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
        w.register::<N::Params>();
        w.register::<N::Outputs>();
    }

    let schema = {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        let r = registry.read();
        NodeSchema {
            inputs: derive_schema::<N::Params>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
            outputs: derive_schema::<N::Outputs>(&r)
                .unwrap_or_else(|e| panic!("{}: {e}", core::any::type_name::<N>())),
        }
    };

    check_ordinals::<N>(&schema);

    let entry = NodeTypeEntry {
        name: core::any::type_name::<N>(),
        schema,
        tick: N::tick,
        prefill: prefill_of::<N>,
        params_changed_tick: params_changed_tick_of::<N>,
    };

    app.init_resource::<NodeTypeRegistry>();
    let mut reg = app.world_mut().resource_mut::<NodeTypeRegistry>();
    let id = NodeTypeId(reg.entries.len() as u32);
    reg.by_name.insert(entry.name, id);
    reg.entries.push(entry);
    id
}

/// Spec §3's startup guard: the node's index consts must agree with the
/// reflect-derived per-kind ordinals.
fn check_ordinals<N: NodeType>(schema: &NodeSchema) {
    let node = core::any::type_name::<N>();
    let mut expected: Vec<(&'static str, u16)> = Vec::new();
    for (i, f) in schema.inputs.continuous.iter().enumerate() {
        expected.push((f.name, i as u16));
    }
    for (i, f) in schema.outputs.continuous.iter().enumerate() {
        expected.push((f.name, (schema.inputs.continuous.len() + i) as u16));
    }
    for (i, f) in schema.inputs.events.iter().enumerate() {
        expected.push((f.name, i as u16));
    }
    for (i, f) in schema.outputs.events.iter().enumerate() {
        expected.push((f.name, (schema.inputs.events.len() + i) as u16));
    }

    for (name, want) in &expected {
        match N::PORT_ORDINALS.iter().find(|(n, _)| n == name) {
            Some((_, got)) if got == want => {}
            Some((_, got)) => panic!(
                "{node}: port `{name}` is ordinal {want}, but PORT_ORDINALS declares {got} \
                 — a field was reordered, or the const is stale"
            ),
            None => panic!(
                "{node}: port `{name}` is undeclared in PORT_ORDINALS (expected ordinal {want})"
            ),
        }
    }
    for (name, _) in N::PORT_ORDINALS {
        if !expected.iter().any(|(n, _)| n == name) {
            panic!("{node}: PORT_ORDINALS declares `{name}`, which is not a port");
        }
    }
}

/// Copies every **unconnected** continuous input from the node's `Params`
/// component into its arena slots. Spec §4's authored-versus-driven rule:
/// `Params` is never written by the graph, so a disconnect returns the port
/// to its authored value.
fn prefill_of<N: NodeType>(world: &World, node: Entity, arena: &mut PortArena, plan: &NodePlan) {
    let Some(params) = world.get::<N::Params>(node) else {
        return;
    };
    let params: &dyn Struct = params.reflect_ref().as_struct().expect("Params is a struct");
    for (ordinal, field) in plan.schema.inputs.continuous.iter().enumerate() {
        if plan.connected_continuous[ordinal] {
            continue;
        }
        let value = params
            .field_at(field.field_index)
            .expect("field_index came from this type's own schema");
        arena.continuous[plan.continuous_base + ordinal] = value.to_dynamic();
    }
}

fn params_changed_tick_of<N: NodeType>(world: &World, node: Entity) -> Option<Tick> {
    world
        .get_entity(node)
        .ok()?
        .get_change_ticks::<N::Params>()
        .map(|t| t.changed)
}
```

Note `plan.schema` — `NodePlan` carries a cloned `NodeSchema` so prefill needs no registry lookup. Task 4 defines it that way.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS. This task's tests depend on `NodePlan`, `PortView` and `TickCtx`, so if Task 4/5 have not landed yet, add minimal stubs in `compile.rs` and `view.rs` in this task and let Tasks 4 and 5 fill them in. The stub bodies must be `todo!()`-free — an empty struct with the fields this task reads.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): NodeType contract, type registry, and the ordinal guard"
```

---

### Task 4: Param edges and the dataflow compiler

**Files:**
- Create: `crates/sway-graph/src/edges.rs`, `crates/sway-graph/src/compile.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: `NodeTypeRegistry`, `NodeSchema`, `NodeTypeId` (Task 3); `PortArena` (Task 1).
- Produces:
  - `GraphNode { pub id: NodeId, pub node_type: NodeTypeId }`, `NodeId(pub u32)`
  - `NodeRuntime { pub continuous_base: usize, pub event_base: usize, pub last_params_tick: Option<Tick> }`
  - `ParamEdge { pub source_port: u16, pub target_port: u16, pub kind: PortKind }`, `EdgeFrom(Entity)`, `EdgeTo(Entity)`, `OutEdges`, `InEdges`
  - `PortKind { Continuous, Event }`
  - `NodePlan { entity, node_type, schema, continuous_base, event_base, connected_continuous: Vec<bool>, continuous_copies: Vec<(usize, usize)>, event_merges: Vec<(usize, usize)> }`
  - `CompiledGraph { pub plans: Vec<NodePlan>, pub continuous_len: usize, pub events_len: usize }` (resource)
  - `compile(world: &mut World) -> Result<CompiledGraph, CompileError>`
  - `CompileError` with `Display`

- [ ] **Step 1: Implement `edges.rs`**

```rust
//! Node and edge components. Spec §5.

use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;

use crate::registry::NodeTypeId;

/// Stable authored identity, used by M4's reconcile. Carried now because it
/// costs nothing and the loader will need it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Component)]
pub struct GraphNode {
    pub id: NodeId,
    pub node_type: NodeTypeId,
}

/// Engine-owned, inserted by `compile`. Spec §4.
#[derive(Component, Default)]
pub struct NodeRuntime {
    pub continuous_base: usize,
    pub event_base: usize,
    /// The `Params` change tick this node last prefilled against. `None`
    /// forces a prefill, which is how a recompile makes a disconnect take
    /// effect.
    pub last_params_tick: Option<Tick>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    Continuous,
    Event,
}

/// A param edge is an entity (spec §5), so Bevy maintains the reverse index
/// and `linked_spawn` below makes despawning a node despawn its edges.
#[derive(Component)]
pub struct ParamEdge {
    /// Ordinal within the source node's kind-space.
    pub source_port: u16,
    /// Ordinal within the target node's kind-space.
    pub target_port: u16,
    pub kind: PortKind,
}

#[derive(Component)]
#[relationship(relationship_target = OutEdges)]
pub struct EdgeFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = EdgeFrom, linked_spawn)]
pub struct OutEdges(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = InEdges)]
pub struct EdgeTo(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = EdgeTo, linked_spawn)]
pub struct InEdges(Vec<Entity>);
```

- [ ] **Step 2: Write the failing tests**

`crates/sway-graph/src/compile.rs`, test module. These are the table-driven failure tests spec §5 and §9 require. Use the `Probe` node type from Task 3 — move it into a `#[cfg(test)] pub(crate) mod test_nodes;` so both tasks share one definition rather than duplicating it.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_nodes::{probe_app, spawn_probe, Probe};

    fn edge(world: &mut World, from: Entity, to: Entity, sp: u16, tp: u16, kind: PortKind) -> Entity {
        world.spawn((ParamEdge { source_port: sp, target_port: tp, kind }, EdgeFrom(from), EdgeTo(to))).id()
    }

    #[test]
    fn a_chain_compiles_in_topological_order() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        // a.value (continuous ordinal 2) -> b.gain (continuous ordinal 0)
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);

        let compiled = compile(app.world_mut()).expect("compiles");
        let order: Vec<Entity> = compiled.plans.iter().map(|p| p.entity).collect();
        assert_eq!(order, vec![a, b], "producer must be ordered before consumer");
    }

    #[test]
    fn bases_are_allocated_contiguously_per_node() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        let compiled = compile(app.world_mut()).expect("compiles");

        let pa = compiled.plans.iter().find(|p| p.entity == a).unwrap();
        let pb = compiled.plans.iter().find(|p| p.entity == b).unwrap();
        assert_ne!(pa.continuous_base, pb.continuous_base);
        assert_eq!(compiled.continuous_len, 6, "two probes, 3 continuous ports each");
        assert_eq!(compiled.events_len, 2);
    }

    #[test]
    fn a_cycle_is_rejected_and_names_every_node_in_it() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);
        edge(app.world_mut(), b, a, 2, 0, PortKind::Continuous);

        let err = compile(app.world_mut()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "{msg}");
        assert!(msg.contains(&format!("{a}")) && msg.contains(&format!("{b}")), "{msg}");
    }

    #[test]
    fn a_second_edge_into_a_continuous_input_is_rejected() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        let c = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, c, 2, 0, PortKind::Continuous);
        edge(app.world_mut(), b, c, 2, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        // Spec §5: "which one wins" has no defensible answer.
        assert!(msg.contains("gain"), "must name the target port: {msg}");
        assert!(msg.contains(&format!("{c}")), "must name the target node: {msg}");
    }

    #[test]
    fn many_edges_into_an_event_input_are_allowed() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        let c = spawn_probe(app.world_mut());
        // Probe has no event output, so drive the event input from two
        // sources' event port 0 — see test_nodes for the emitter variant.
        edge(app.world_mut(), a, c, 0, 0, PortKind::Event);
        edge(app.world_mut(), b, c, 0, 0, PortKind::Event);

        assert!(compile(app.world_mut()).is_ok(), "event fan-in is legal (spec §5)");
    }

    #[test]
    fn a_type_mismatch_names_both_nodes_both_ports_and_both_types() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_int_probe(app.world_mut()); // u32 params, see test_nodes
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("f32") && msg.contains("u32"), "{msg}");
        assert!(msg.contains("value") && msg.contains("count"), "{msg}");
    }

    #[test]
    fn a_port_index_out_of_range_is_rejected_with_the_arity() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 99, 0, PortKind::Continuous);

        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains("99"), "{msg}");
        assert!(msg.contains('3'), "must state the schema's arity: {msg}");
    }

    #[test]
    fn an_edge_to_a_despawned_node_is_rejected() {
        let mut app = probe_app();
        let a = spawn_probe(app.world_mut());
        let b = spawn_probe(app.world_mut());
        edge(app.world_mut(), a, b, 2, 0, PortKind::Continuous);
        app.world_mut().despawn(b);

        // linked_spawn should have taken the edge with it, so this compiles
        // clean — which is the actual assertion. A dangling edge would be a
        // Bevy relationship bug, and this test is what would catch it.
        assert!(compile(app.world_mut()).is_ok());
    }

    #[test]
    fn an_unregistered_node_type_is_rejected() {
        let mut app = probe_app();
        let e = app.world_mut().spawn(GraphNode { id: NodeId(0), node_type: NodeTypeId(999) }).id();
        let msg = compile(app.world_mut()).unwrap_err().to_string();
        assert!(msg.contains(&format!("{e}")), "{msg}");
        assert!(msg.contains("999"), "{msg}");
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p sway-graph compile`
Expected: FAIL — `compile` not found.

- [ ] **Step 4: Implement `compile.rs`**

Structure, in order:

1. `NodePlan` and `CompiledGraph` as declared in the Interfaces block above. `NodePlan.schema` is a cloned `NodeSchema` so the tick loop and prefill need no registry lookup per node.
2. `CompileError` — one variant per row of spec §5's table: `UnknownNodeType { node: Entity, id: NodeTypeId }`, `PortOutOfRange { node: Entity, port: u16, kind: PortKind, arity: usize }`, `TypeMismatch { source: Entity, source_port: &'static str, source_type: &'static str, target: Entity, target_port: &'static str, target_type: &'static str }`, `DuplicateContinuousInput { target: Entity, port: &'static str, first: Entity, second: Entity }`, `MissingEndpoint { edge: Entity, missing: Entity }`, `Cycle { nodes: Vec<Entity> }`. Each `Display` arm names its node(s).
3. Pass 1 — collect: query `(Entity, &GraphNode)`, resolve each `node_type` in `NodeTypeRegistry`, error on miss. Sort collected nodes by `GraphNode::id` before anything else, so the compiled order is deterministic for graphs the topology does not fully order.
4. Pass 2 — allocate bases: walk nodes in that sorted order, assigning `continuous_base` and `event_base` cumulatively from each schema's `continuous_len()` / `events_len()`.
5. Pass 3 — validate edges: query `(Entity, &ParamEdge, &EdgeFrom, &EdgeTo)`. For each, resolve both endpoints (error `MissingEndpoint` if either is absent from the collected set), bounds-check both ports against the relevant schema half, compare `type_id`s, and record continuous fan-in in a `HashMap<(Entity, u16), Entity>` to catch duplicates.
6. Pass 4 — topological sort: Kahn's over the node set with edges as dependencies. Seed the ready queue in the sorted-by-`NodeId` order and pop from the front, so ties break deterministically. On termination with nodes remaining, collect them into `CompileError::Cycle`.
7. Pass 5 — build plans: for each node in topological order, fill `connected_continuous` (a `Vec<bool>` over its continuous *inputs*), `continuous_copies` (absolute `(src_slot, dst_slot)` pairs), and `event_merges` (same, appended rather than overwritten). Sort `event_merges` by the source node's position in the compiled order, which is the deterministic tiebreak spec §5 requires.
8. Write `NodeRuntime` onto each node entity with its bases and `last_params_tick: None`.

The `resize` call on `PortArena` belongs to Task 5's runner, not here — `compile` produces the layout, the runner applies it.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, 9 compile tests plus the earlier ones.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): param edges and the dataflow compiler"
```

---

### Task 5: `PortView`, `TickCtx`, and the tick runner

**Files:**
- Create: `crates/sway-graph/src/view.rs`, `crates/sway-graph/src/tick.rs`
- Modify: `crates/sway-graph/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–4.
- Produces:
  - `TickCtx { pub dt: f32, pub tick_start: f64, pub tick_index: u64 }`
  - `PortView<'a>` with:
    - `read<T: Reflect + Clone>(&self, ContinuousIdx) -> T`
    - `write<T: Reflect>(&mut self, ContinuousIdx, T)`
    - `events<T: Reflect>(&self, EventIdx) -> impl Iterator<Item = EventRef<'_, T>>`
    - `emit<T: Reflect>(&mut self, EventIdx, offset: f32, value: T)`
  - `EventRef<'a, T> { pub offset: f32, pub value: &'a T }`
  - `graph_tick(world: &mut World)` — the exclusive system
  - `GraphPlugin` — inserts `PortArena`, `NodeTypeRegistry`, adds `graph_tick` to `FixedUpdate`

`PortView`'s indices are **node-relative ordinals**, resolved against the node's bases inside the view. That is what stops a node reaching another node's ports by arithmetic (spec §4).

- [ ] **Step 1: Write the failing tests**

`crates/sway-graph/src/tick.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_nodes::{gain_app, spawn_gain, Gain};

    // `Gain` (test_nodes): out.value = in.gain * in.bias, plus a
    // `saw` node whose out.value = ctx.tick_start as f32 for the
    // absolute-time assertions below.

    #[test]
    fn an_edge_carries_a_value_within_one_tick() {
        // Spec §6: writes are immediate, so a node later in topological order
        // sees an earlier node's output in the SAME tick — not one tick late.
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 0.0, 5.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(port_value(&app, b, Gain::OUT_VALUE), 30.0, "6.0 * 5.0 in one tick");
    }

    #[test]
    fn an_unconnected_input_reads_its_authored_value() {
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 4.0, 0.5);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 2.0);
    }

    #[test]
    fn a_connected_input_shadows_the_authored_value_without_overwriting_it() {
        // Spec §4: Params holds what the author wrote; the arena holds what
        // the edge is sending. Saving a project must not bake in the latter.
        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 7.0, "driven, not authored");
        assert_eq!(
            app.world().get::<GainParams>(dst).unwrap().gain,
            4.0,
            "Params must be untouched by the graph"
        );
    }

    #[test]
    fn disconnecting_and_recompiling_returns_the_port_to_its_authored_value() {
        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        let e = connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 7.0);

        app.world_mut().despawn(e);
        recompile(&mut app);
        app.update();

        // Spec §4: not frozen where the edge left it.
        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 4.0);
    }

    #[test]
    fn a_params_change_is_seen_however_many_ticks_later_it_is_read() {
        // THE `Changed<T>` FAILURE MODE (spec §4, §9). A filter would be true
        // for exactly one tick; this must hold across many.
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        for _ in 0..10 { app.update(); }

        app.world_mut().get_mut::<GainParams>(a).unwrap().gain = 9.0;
        for _ in 0..10 { app.update(); }

        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 9.0);
    }

    #[test]
    fn an_unchanged_node_does_not_reprefill() {
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        app.update();
        let first = app.world().get::<NodeRuntime>(a).unwrap().last_params_tick;
        app.update();
        let second = app.world().get::<NodeRuntime>(a).unwrap().last_params_tick;
        assert_eq!(first, second, "gate must not re-fire on an unchanged node");
    }

    #[test]
    fn event_slots_are_empty_at_the_start_of_every_tick() {
        let mut app = emitter_app();       // emits one occurrence per tick
        let e = spawn_emitter(app.world_mut());
        recompile(&mut app);
        app.update();
        assert_eq!(event_count(&app, e, Emitter::OUT_PULSE), 1);
        app.update();
        assert_eq!(event_count(&app, e, Emitter::OUT_PULSE), 1, "not 2 — cleared each tick");
    }

    #[test]
    fn merged_event_streams_arrive_in_offset_order() {
        // Spec §5: sorted by (offset, source's compiled index).
        let mut app = emitter_app();
        let late = spawn_emitter_at(app.world_mut(), 0.006);
        let early = spawn_emitter_at(app.world_mut(), 0.001);
        let sink = spawn_sink(app.world_mut());
        connect_event(app.world_mut(), late, Emitter::OUT_PULSE, sink, Sink::IN_PULSE);
        connect_event(app.world_mut(), early, Emitter::OUT_PULSE, sink, Sink::IN_PULSE);
        recompile(&mut app);

        app.update();

        assert_eq!(sink_offsets(&app, sink), vec![0.001, 0.006]);
    }
}
```

Write the `test_nodes` helpers (`gain_app`, `spawn_gain`, `connect`, `connect_event`, `recompile`, `port_value`, `event_count`, `sink_offsets`, plus the `Gain`, `Emitter` and `Sink` node types) in `crates/sway-graph/src/test_nodes.rs` behind `#[cfg(test)]`. `recompile` runs `compile`, inserts the result as a resource, and calls `PortArena::resize`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p sway-graph tick`
Expected: FAIL — `graph_tick` not found.

- [ ] **Step 3: Implement `view.rs`**

```rust
//! `PortView` — a node's typed, scoped window onto the arena. Spec §4.

use bevy_reflect::Reflect;

use crate::ports::{ContinuousIdx, EventIdx, Occurrence, PortArena};

pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds. A node needing
    /// absolute time writes `ctx.tick_start + offset as f64` (spec §7).
    pub tick_start: f64,
    pub tick_index: u64,
}

pub struct EventRef<'a, T> {
    pub offset: f32,
    pub value: &'a T,
}

/// Scoped to one node: indices are that node's own ordinals, resolved against
/// its bases here, so a node cannot reach another node's ports by arithmetic.
pub struct PortView<'a> {
    arena: &'a mut PortArena,
    continuous_base: usize,
    event_base: usize,
}
```

Methods: `read` downcasts and clones (panicking with the ordinal and the expected type if the slot holds something else — a compile-validated graph makes that unreachable, so the panic message should say so); `write` replaces the slot via `Box::new`; `events` filters the slot's occurrences through `try_downcast_ref`; `emit` pushes an `Occurrence`.

- [ ] **Step 4: Implement `tick.rs`**

```rust
pub fn graph_tick(world: &mut World) {
    let Some(compiled) = world.remove_resource::<CompiledGraph>() else { return };

    let dt = world.resource::<Time<Fixed>>().delta_secs();
    let tick_start = world.resource::<Time<Fixed>>().elapsed_secs_f64() - dt as f64;
    let tick_index = /* a GraphTickCount resource, incremented here */;
    let ctx = TickCtx { dt, tick_start, tick_index };

    world.resource_scope(|world: &mut World, mut arena: Mut<PortArena>| {
        arena.clear_events();

        for plan in &compiled.plans {
            // gather
            for &(src, dst) in &plan.continuous_copies {
                arena.continuous[dst] = arena.continuous[src].to_dynamic();
            }
            for &(src, dst) in &plan.event_merges {
                let copied: Vec<Occurrence> = arena.events[src]
                    .iter()
                    .map(|o| Occurrence { offset: o.offset, value: o.value.to_dynamic() })
                    .collect();
                arena.events[dst].extend(copied);
            }

            // prefill, gated on the Params change tick (spec §4)
            let entry = /* registry lookup by plan.node_type */;
            let current = (entry.params_changed_tick)(world, plan.entity);
            let last = world.get::<NodeRuntime>(plan.entity).and_then(|r| r.last_params_tick);
            if last != current {
                (entry.prefill)(world, plan.entity, &mut arena, plan);
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_params_tick = current;
                }
            }

            // dispatch
            let mut view = PortView::new(&mut arena, plan.continuous_base, plan.event_base);
            (entry.tick)(world, plan.entity, &mut view, &ctx);
        }
    });

    world.insert_resource(compiled);
}
```

Three things this sketch elides that the implementation must handle:

- **The registry borrow.** `world` is borrowed mutably for the `tick` call, so the registry entry's three fn pointers must be copied out into locals *before* it, not held as a `&NodeTypeEntry` across it. Copy `(tick, prefill, params_changed_tick)` into a `Vec` keyed by plan index once, before the loop.
- **`event_merges` copies through a temporary** because `arena.events[src]` and `arena.events[dst]` alias. The `Vec` allocation there is the one per-tick allocation this design does not avoid; note it in the findings report.
- **The `last != current` comparison is a plain inequality**, not `Tick::is_newer_than`. We need only "did it move", and inequality needs no `this_run` tick and no wraparound reasoning.

`GraphPlugin` inserts `PortArena::new(0, 0)`, `NodeTypeRegistry::default()`, a `GraphTickCount(u64)` resource, and adds `graph_tick` to `FixedUpdate`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-graph`
Expected: PASS, 8 tick tests plus the earlier ones.

- [ ] **Step 6: Run clippy and the workspace suite**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-graph
git commit -m "feat(graph): PortView and the FixedUpdate tick runner"
```

---

### Task 6: `sway-nodes` — MIDI ingress, `MidiNote`, `MidiCC`

**Files:**
- Create: `crates/sway-nodes/Cargo.toml`, `crates/sway-nodes/src/lib.rs`, `crates/sway-nodes/src/midi.rs`
- Modify: root `Cargo.toml`

**Interfaces:**
- Consumes: the whole `sway-graph` surface.
- Produces:
  - `RawMidi { pub status: u8, pub data1: u8, pub data2: u8 }`
  - `NoteMsg { pub note: u8, pub velocity: u8 }` (`Reflect`)
  - `MidiInbox { pub events: VecDeque<(f64, RawMidi)> }` (resource) with `push(&mut self, t: f64, m: RawMidi)`
  - `MidiNote` — params `channel: u8`, `note_lo: u8`, `note_hi: u8`; outputs `note_on: Event<NoteMsg>`, `note_off: Event<NoteMsg>`
  - `MidiCC` — params `channel: u8`, `cc: u8`; output `value: f32`
  - `drain_inbox` — the system that maps buffered events into the tick window

**The window rule** (spec §7), implemented in `drain_inbox`, which runs first in `FixedUpdate` ahead of `graph_tick`: take events with `t <= ctx.tick_start + dt`, stamp `offset = (t - tick_start).clamp(0.0, dt)`, leave later ones buffered. Because `MidiNote` and `MidiCC` both need the drained slice, `drain_inbox` writes it into a `TickMidi { events: Vec<(f32, RawMidi)> }` resource that both nodes read from `world`.

- [ ] **Step 1: Create the manifest**

```toml
[package]
name = "sway-nodes"
version.workspace = true
edition.workspace = true

# Deliberately NOT sway-midi: RawMidi is defined here so this crate carries
# no macOS-only FFI and is testable anywhere (spec §2, §7).
[dependencies]
sway-graph.workspace = true
bevy_app.workspace = true
bevy_ecs.workspace = true
bevy_reflect.workspace = true
bevy_time.workspace = true
```

Add `"crates/sway-nodes"` to workspace members and `sway-nodes = { path = "crates/sway-nodes" }` to `[workspace.dependencies]`.

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn an_event_inside_the_window_gets_its_offset_and_one_past_it_waits() {
    let mut app = midi_app(); // 120Hz, one fixed tick per update
    app.world_mut().resource_mut::<MidiInbox>().push(0.002, note_on(60, 100));
    app.world_mut().resource_mut::<MidiInbox>().push(0.020, note_on(64, 100));

    app.update(); // window [0.0, 0.00833)

    let drained = &app.world().resource::<TickMidi>().events;
    assert_eq!(drained.len(), 1, "the 0.020 event belongs to a later tick");
    assert!((drained[0].0 - 0.002).abs() < 1e-6);
}

#[test]
fn a_late_arrival_clamps_to_zero_rather_than_going_negative() {
    let mut app = midi_app();
    for _ in 0..3 { app.update(); }
    // stamped before the current window began
    app.world_mut().resource_mut::<MidiInbox>().push(0.0, note_on(60, 100));
    app.update();
    let drained = &app.world().resource::<TickMidi>().events;
    assert_eq!(drained[0].0, 0.0, "clamped, not dropped and not negative");
}

#[test]
fn note_on_with_zero_velocity_is_a_note_off() {
    // Many devices spell note-off that way — sway-app/src/graph.rs:44
    // already handled this and the behaviour must survive the move.
    let mut app = midi_app_with_node();
    app.world_mut().resource_mut::<MidiInbox>().push(0.001, RawMidi { status: 0x90, data1: 60, data2: 0 });
    app.update();
    assert_eq!(note_on_count(&app), 0);
    assert_eq!(note_off_count(&app), 1);
}

#[test]
fn the_channel_and_note_range_filters_reject_non_matching_events() {
    // MidiNote spawned with channel 0, note_lo 60, note_hi 72.
    let mut app = midi_app_with_node();
    let inbox = &mut app.world_mut().resource_mut::<MidiInbox>();
    inbox.push(0.001, RawMidi { status: 0x91, data1: 64, data2: 100 }); // channel 1
    inbox.push(0.002, RawMidi { status: 0x90, data1: 48, data2: 100 }); // below range
    inbox.push(0.003, RawMidi { status: 0x90, data1: 80, data2: 100 }); // above range
    inbox.push(0.004, RawMidi { status: 0x90, data1: 64, data2: 100 }); // matches

    app.update();

    assert_eq!(note_on_count(&app), 1, "only the in-range, in-channel note passes");
    assert_eq!(first_note_on(&app).note, 64);
}

#[test]
fn midi_cc_holds_its_value_between_messages() {
    // The continuous/event distinction made observable: a CC with no new
    // message this tick still reads its last value, where an event port
    // would read empty (spec §4).
    let mut app = midi_app_with_cc();
    app.world_mut().resource_mut::<MidiInbox>().push(0.001, RawMidi { status: 0xB0, data1: 74, data2: 127 });
    app.update();
    assert_eq!(cc_value(&app), 1.0);
    app.update();
    assert_eq!(cc_value(&app), 1.0, "held, not reset");
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p sway-nodes`
Expected: FAIL — crate has no such items.

- [ ] **Step 4: Implement `midi.rs`**

`MidiNote::tick` iterates `TickMidi`, filters on channel and note range, and calls `ports.emit(Self::OUT_NOTE_ON, offset, NoteMsg { .. })` — status `0x90` with `data2 > 0` is a note-on, `0x80` or `0x90` with `data2 == 0` is a note-off. `MidiCC::tick` writes `ports.write(Self::OUT_VALUE, data2 as f32 / 127.0)` for the last matching message in the window, and writes nothing when there is none, so the slot holds.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sway-nodes
git commit -m "feat(nodes): sway-nodes crate, MIDI ingress, MidiNote and MidiCC"
```

---

### Task 7: The remaining six signal nodes

**Files:**
- Create: `crates/sway-nodes/src/lfo.rs`, `crates/sway-nodes/src/envelope.rs`, `crates/sway-nodes/src/math.rs`
- Modify: `crates/sway-nodes/src/lib.rs` (add `SignalNodesPlugin` registering all eight)

**Interfaces:**
- Produces: `LFO`, `Envelope`, `Math`, `Remap`, `Switch`, `Select`, and `SignalNodesPlugin`.

Per spec §8:

| Node | Params | Outputs | State |
|---|---|---|---|
| `LFO` | `hz: f32`, `shape: Waveform`, `phase: f32`, `amplitude: f32` | `value: f32` | `()` |
| `Envelope` | `trigger: Event<NoteMsg>`, `attack: f32`, `decay: f32`, `sustain: f32`, `release: f32` | `value: f32` | `EnvelopeState { gate_on: Option<f64>, gate_off: Option<f64>, velocity: f32 }` |
| `Math` | `op: MathOp`, `a: f32`, `b: f32` | `value: f32` | `()` |
| `Remap` | `value`, `in_min`, `in_max`, `out_min`, `out_max: f32`, `clamp: bool` | `value: f32` | `()` |
| `Switch` | `select: bool`, `a: f32`, `b: f32` | `value: f32` | `()` |
| `Select` | `trigger: Event<NoteMsg>`, `field: NoteField` | `value: f32` | `SelectState { held: f32 }` |

`Waveform`, `MathOp` and `NoteField` are `Reflect` enums — spec §2.4's "a type-selector param is a smell; make it a node type" does **not** apply to them: they select an arithmetic operation, not a type, so no port schema varies with them.

- [ ] **Step 1: Write the failing tests**

The two that carry real weight:

```rust
#[test]
fn the_lfo_is_a_function_of_absolute_time_not_an_accumulator() {
    // Spec §6. Run one app for 100 ticks and another for 100 ticks with a
    // deliberate 10-tick gap in the middle; at the same elapsed time they
    // must agree. An accumulating LFO diverges here — which is exactly the
    // bug sway-app/src/graph.rs:53 documents in M0's decay.
    let a = run_lfo_continuously(100);
    let b = run_lfo_with_dropped_ticks(100, 45..55);
    assert!((a - b).abs() < 1e-6, "accumulated phase: {a} vs {b}");
}

#[test]
fn two_notes_in_one_tick_at_different_offsets_give_different_envelope_values() {
    // THE sub-tick discrimination test (spec §9). Without it §7 is
    // unfalsifiable: an implementation that stamped every offset 0.0 would
    // pass every other test in this plan.
    let mut app = envelope_app(); // attack 0.05s
    app.world_mut().resource_mut::<MidiInbox>().push(0.0001, note_on(60, 127));
    app.update();
    let early = envelope_value(&app);

    let mut app = envelope_app();
    app.world_mut().resource_mut::<MidiInbox>().push(0.0080, note_on(60, 127));
    app.update();
    let late = envelope_value(&app);

    assert!(early > late, "earlier note is further into its attack: {early} vs {late}");
    assert!((early - late).abs() > 1e-4, "difference must be real, not float noise");
}
```

Plus per-node value tests: `Math` for each `MathOp`, `Remap` with and without clamping, `Switch` selecting both ways, `Select` latching and holding across a tick with no event, `Envelope` reaching sustain and releasing.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p sway-nodes`
Expected: FAIL.

- [ ] **Step 3: Implement the six nodes**

`Envelope::tick` reads its trigger stream, and on each note-on stores `ctx.tick_start + offset as f64` into `EnvelopeState::gate_on`. Its output is a pure function of `now - gate_on`, where `now = ctx.tick_start + ctx.dt as f64`. It never accumulates.

`LFO::tick` computes `phase = (ctx.tick_start * hz as f64).fract()` plus the authored `phase` offset — again a function of absolute time.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sway-nodes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-nodes
git commit -m "feat(nodes): LFO, Envelope, Math, Remap, Switch and Select"
```

---

### Task 8: The golden-trace harness

**Files:**
- Create: `crates/sway-nodes/tests/traces.rs`, `crates/sway-nodes/tests/traces/*.ron`
- Modify: `crates/sway-nodes/Cargo.toml` (add `ron = "0.12"` and `serde` to `[dev-dependencies]`)

**Interfaces:**
- Produces: the harness functions `load_input`, `run_trace`, `assert_or_bless`.

Formats:

```ron
// tests/traces/envelope-retrigger.in.ron
(
    tick_hz: 120.0,
    ticks: 60,
    events: [ (0.0008, (status: 144, data1: 60, data2: 100)),
              (0.1200, (status: 144, data1: 60, data2: 64)) ],
)
```

```ron
// tests/traces/envelope-retrigger.out.ron
(
    ports: ["envelope.value", "midinote.note_on"],
    ticks: [ (0, [Continuous(0.0),  Events([])]),
             (1, [Continuous(0.016), Events([(0.0008, "note_on(60,100)")])]) ],
)
```

- [ ] **Step 1: Write the harness and one case**

`assert_or_bless` compares tick-by-tick and, on mismatch, reports the **first differing tick and port** — that is the whole reason this is not a single hash (spec §9). When `SWAY_BLESS=1` is set it rewrites the `.out.ron` instead of asserting, and prints `~ rewrote <path>`.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sway-nodes --test traces`
Expected: FAIL — no expected file yet.

- [ ] **Step 3: Bless the first case, then verify it locks**

```bash
SWAY_BLESS=1 cargo test -p sway-nodes --test traces
cargo test -p sway-nodes --test traces
```
Expected: the first run rewrites, the second passes.

**Then verify the harness is not vacuous:** change one node's arithmetic by hand, re-run without `SWAY_BLESS`, and confirm the failure names the tick and port. Revert. A blessing harness that cannot fail is worse than no harness, and this is the only step that proves it can.

- [ ] **Step 4: Add the remaining cases**

At minimum: `lfo-one-cycle`, `envelope-retrigger`, `cc-hold`, `chain-math-remap`, `two-notes-one-tick` (the sub-tick case), and `event-fan-in` (two `MidiNote`s into one `Envelope`).

- [ ] **Step 5: Add the determinism test**

```rust
#[test]
fn the_same_trace_twice_is_bit_identical() {
    let a = run_trace("envelope-retrigger");
    let b = run_trace("envelope-retrigger");
    assert_eq!(a, b);
}
```

This does not replace the golden files — compared only against itself it would pass while every value was wrong.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-nodes
git commit -m "test(nodes): golden-trace harness and the first six cases"
```

---

### Task 9: The `sway-app` handover

**Files:**
- Delete: `crates/sway-app/src/graph.rs`
- Create: `crates/sway-app/src/bridge.rs`
- Modify: `crates/sway-app/src/main.rs`, `crates/sway-app/src/scene.rs`, `crates/sway-app/Cargo.toml`

**Interfaces:**
- Consumes: `sway-graph`'s `GraphPlugin` and `compile`; `sway-nodes`' `SignalNodesPlugin`, `MidiInbox`, `RawMidi`, `MidiNote`, `Envelope`.

- [ ] **Step 1: Feed `MidiInbox` from `MidiRx`**

`MidiRx` stays in `sway-app` (as `crates/sway-app/src/graph.rs:16` always said it would). A `PreUpdate` system drains the crossbeam channel, converts `host_time` to seconds via `mach_timebase_info`, and pushes into `MidiInbox`. The timebase conversion goes in `sway-midi` next to the FFI it belongs to, exposed as `sway_midi::host_time_to_secs(u64) -> f64`, with a test asserting it is monotonic and that a known tick count converts to the expected nanoseconds.

- [ ] **Step 2: Build the cube graph in Rust**

```rust
// crates/sway-app/src/bridge.rs
//
// THROWAWAY. M2b's scene nodes delete this file. It exists so M2a has a live
// path: without it the engine is verified only by tests and never by an
// Octatrack plugged into a real machine (spec §10).
```

Spawn a `MidiNote` and an `Envelope`, connect note_on → trigger, run `compile`, insert the result, resize the arena.

- [ ] **Step 3: Replace `apply_level`**

`crates/sway-app/src/scene.rs`'s `apply_level` reads `GraphState`. Point it at the envelope's output slot in the arena instead, and follow spec §10's asset rule — `Assets::get`, compare, and only then `get_mut`, because `get_mut` marks the asset changed by the act of calling it.

- [ ] **Step 4: Delete `graph.rs` and its module declaration**

Remove `mod graph;` from `main.rs` and delete the file. Its `TICK_HZ` moves to `sway-app`'s `main.rs` unchanged at `120.0`, with a comment pointing at spec §11 for why it is still provisional.

**The deliberate regression** (spec §10): `graph.rs`'s stored-trace test pinned M0's linear decay. The envelope replaces that behaviour, so those expectations do not survive and are **not** ported. The equivalent coverage is Task 8's trace harness. Say this in the commit message so a reviewer reads the deletion as intended rather than as lost coverage.

- [ ] **Step 5: Run the app and look at it**

Run: `cargo run -p sway-app -- --windowed --midi <substring>`
Expected: playing a note brightens the cube, and it decays with the envelope's release rather than M0's linear ramp. **A human must confirm this** — the M1b findings report's "what was not proven" section exists because test-clean and visually-wrong are compatible states.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: green. Test count drops by the 8 deleted `graph.rs` tests and rises by everything M2a added.

- [ ] **Step 7: Commit**

```bash
git add -A crates/sway-app crates/sway-midi
git commit -m "feat(app): drive the cube from sway-graph, retire the M0 graph

Deletes crates/sway-app/src/graph.rs. Its stored-trace test pinned M0's
linear decay, which the Envelope node replaces; that coverage moves to
sway-nodes' golden traces rather than being ported (spec §10)."
```

---

### Task 10: Findings report

**Files:**
- Create: `docs/superpowers/reports/2026-07-31-m2a-graph-engine-findings.md`
- Modify: `docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md` (only if a decision proved wrong)

- [ ] **Step 1: Answer the four questions spec §12 requires**

1. What resisted `Reflect` in a real node set, and the workaround — starting with whether `Event<T>`'s derive worked as written in Task 2.
2. Whether `Box<dyn PartialReflect>` proved adequate, and what would have to be true to force typed columns. Include the `event_merges` temporary from Task 5 as a known allocation.
3. Whether the positional-index-const scheme held across eight node types, or whether the derive macro should be pulled forward.
4. What the tick costs at this cardinality — as a data point, explicitly **not** as the tick-rate answer.

- [ ] **Step 2: Record what M2b would otherwise rediscover**

- [ ] **Step 3: State what was not proven**

Follow the M1b report's precedent: a plain list, at the same volume as the positive findings.

- [ ] **Step 4: Amend the design if a decision was wrong**

Add a **Revision** line at the top, in the style the parent spec and the M1b design use.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers
git commit -m "docs: M2a graph engine findings"
```

---

## Self-review

**Spec coverage.** §1 scope → the task list and its exclusions; §2 crate layout → Tasks 1 and 6 manifests, with the "not `bevy`, not `bevy_render`" constraint enforced where it is actually enforceable; §3 node contract → Tasks 2 and 3, with the ordinal guard as its own test; §4 arena → Tasks 1 and 5, with the split-collections rationale carried into the code comments and the prefill gate given three tests; §5 edges and compilation → Task 4, one test per row of the failure table; §6 tick → Task 5, with immediate-writes and absolute-time as named tests; §7 MIDI → Task 6, window rule and clamping tested; §8 node set → Tasks 6 and 7, including `Select` as the latch; §9 testing → distributed, with the trace harness as Task 8; §10 handover → Task 9, regression named in the commit message; §11 open questions → carried into Task 10's report rather than closed; §12 findings → Task 10.

**Two things this plan adds that the spec implies but does not state.** The compiler sorts nodes by `NodeId` before allocating bases and seeds Kahn's queue in that order — without it, a graph whose topology does not fully order its nodes would compile differently run to run, and every golden trace would be flaky for reasons no test would explain. And `derive_schema` rejects an `Event<T>` field whose type data is missing, because the alternative is a silently useless continuous port of a zero-sized type.

**Known soft spots, flagged in place rather than papered over.** `Event<T>`'s `#[derive(Reflect)]` (Task 2 Step 1) is the one genuinely unverified construct in this plan, and the header says to read the derive's error rather than invent a workaround. The `NodeTypeEntry` borrow in Task 5 Step 4 will fight the borrow checker if the fn pointers are not copied out first; the step says so rather than leaving it to be discovered. `test_nodes.rs` is shared between Tasks 3, 4 and 5, so whichever lands first creates it.

**Type consistency.** `ContinuousIdx`/`EventIdx` are u32-wrapping newtypes throughout; `NodePlan.continuous_base`/`event_base` are `usize` (arena indices) while `PORT_ORDINALS` and `ParamEdge`'s ports are `u16` (per-node ordinals) — the conversion happens inside `PortView` and `compile`, nowhere else. `SchemaHalf` is used under that name in Tasks 2–5. `NodeSchema::continuous_len()` is called in Tasks 3 and 4. `prefill`/`params_changed_tick`/`tick` keep their signatures from Task 3's declaration through Task 5's call sites.
