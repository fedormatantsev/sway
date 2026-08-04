# Ports as component values — Design

**Date:** 2026-08-04
**Status:** Approved, pre-implementation
**Parent spec:** `2026-07-25-sway-design.md` §2.1, §2.2, §2.4, §2.5, §2.10, §2.11
**Prior:** `2026-08-03-unified-edges-design.md` (one `Edge`, one order, `Inlets`/`Outlets`)
**Supersedes:** `Product<T>`, `Spatial`, `PortArena`, `FieldKind`, and editor
`EdgeKind` from the unified-edges design and its implementation
**Placement:** follows unified edges; before RON schema (M4)

## 1. What this is

Unified edges collapsed three edge kinds into one edge and one TypeId check, but
left a parallel carrier taxonomy in place: `FieldKind` / `EdgeKind`, a
`Product<T>` wrapper for entity capability refs, a `Spatial` marker with three
compiler special cases, and a `PortArena` of boxed reflect values beside the
node’s own `Inlets` component.

This design removes that taxonomy. The graph model has **no field kinds and no
edge kinds**. An edge is an endpoint pair; a field is a name, a `TypeId`, and
whether it is variadic. Engine behaviour that today hangs off kinds becomes
**local policy keyed by value `TypeId` or type data**, not variants of the model.

Values live on `Inlets` / `Outlets` components. The arena goes away. High-cardinality
data still does not travel on edges: geometry and similar pass as small **handles**
(CPU now, GPU later). Entity ids appear only for hierarchy, which compile still
backs with Bevy `ChildOf`.

## 2. Core model

**Ports are fields on components, not arena slots.**

- Each node entity carries its `Inlets` and `Outlets` as ECS components.
- An edge means: at gather time, copy the source outlet field into the target
  inlet field. Unconnected inlets keep their authored component values
  (parent §2.11).
- Validation is TypeId equality on the slot type. Direction comes from which
  struct the field sits in.

**The model does not classify ports or edges.** There is no `FieldKind`, no
`EdgeKind`, and no stored “this is a product edge” flag anywhere in
`sway-graph` or the editor snapshot.

**What a field may hold** is just a reflect value. Useful shapes in practice:

| Field type | Role |
|---|---|
| Plain value (`f32`, `Vec3`, `GeoHandle`, …) | Continuous data or a small handle on the wire |
| `Events<T>` | Ordinary value (`Vec<Occurrence<T>>`); still emptied each tick |
| `Entity` / `Vec<Entity>` | Hierarchy wiring only |

Buffer / geometry / material connections carry **handle values**, not entity
capability references. The heavy table (`Geometry`, GPU buffers) stays behind
the handle. Parent §2.1’s rule stands: high-cardinality data is not an edge
payload — only a handle is.

**Deleted from the API and model:** `Product<T>`, `Spatial`, `PortArena`,
`FieldKind`, editor `EdgeKind`, `ReflectProduct`, `ProductAccess`,
`register_product`.

## 3. Policies on TypeId (not kinds)

Compile and tick may special-case certain value types. That is engine policy,
not a graph-model enum.

| When | Behaviour |
|---|---|
| `registry.get_type_data::<ReflectEventList>(slot_type)` | Collect clear fns; empty those fields in place before each tick |
| `slot_type == TypeId::of::<Entity>()` | Single-consumer fan-out on outlets; exclude those edges from the topological sort; emit / refresh Bevy `ChildOf`; check parenting acyclicity separately |
| otherwise | Ordinary gather: copy outlet field → inlet field |

Missing `ReflectEventList` for a type whose path is an `Events<_>` marker remains
a derive/register **diagnostic** (silent “never cleared” would be a bug). That
check is not a kind.

Editor styling may map `TypeId` → colour locally if useful. It must not persist
an `EdgeKind`.

## 4. Schema

`FieldSpec` carries no `kind`:

```text
FieldSpec {
    name: &'static str,
    field_index: usize,
    slot_type: TypeId,
    slot_type_path: &'static str,
    variadic: bool,
}
```

`derive_fields` walks the `Inlets` / `Outlets` struct, treats `Vec<T>` as
variadic with element `TypeId`, and stops. It does not branch into
Value / Events / Product.

## 5. Compilation

Passes keep the unified-edges shape; kind flags disappear:

1. **Expand** — per node, walk registered Inlets/Outlets; read `Vec` lengths
   from the instance → field layout / offsets for gather.
2. **Validate** — per edge: direction, TypeId match, inlet-already-connected;
   Entity-typed outlet single-consumer (today’s Spatial fan-out rule).
3. **Parenting acyclicity** — walk Entity-typed edges only (they are excluded
   from the sort below).
4. **Order** — one topological sort over every edge **except** Entity-typed.
5. **Emit** — apply `ChildOf` for Entity-typed edges; seed Entity-typed
   **outlets** with the node entity (replacing today’s Product outlet seeding).

Error wording for single-consumer / parenting cycles may say parenting /
`ChildOf` rather than `Spatial`. The rule is unchanged.

## 6. Tick and gather

```text
clear     for each Events-typed field (via ReflectEventList): clear in place
per node, in compiled order:
  gather  for each connected edge: copy source Outlets field → target Inlets
          field (including Entity fields, so ticks/cooks see wired children;
          ChildOf itself is maintained at compile, not re-derived each tick)
  tick    node reads/writes Inlets/Outlets and its own components
  cook    if dirty && COOKS
```

`PortView` is a view over this node’s `Inlets` / `Outlets` (plus layout
metadata), not over a `PortArena`. Call-site shape (`get`/`set` by field
ordinal) can stay; backing storage changes.

There is no second arena and no boxed-per-slot resource for port values.

## 7. Handles

`GeoHandle` (name indicative; exact type lives in `sway-geo` / GPU layer) is a
small `Copy` / `Clone` / `Reflect` value. The graph matches it by `TypeId` like
`f32`. It names a CPU geometry buffer set now and must be able to name GPU
buffers later. Representation (generation index, store key, etc.) is an
implementation detail outside the graph model.

Material and other buffer-like products follow the same pattern: a handle value
on the port, heavy data off the edge.

Node authoring sketch:

```rust
struct MeshInlets {
    geo: GeoHandle,
    material: MaterialHandle,
}

struct GroupInlets {
    children: Vec<Entity>,
    translation: Vec3,
}

struct GroupOutlets {
    /// Compile-seeded with this node’s entity so parenting edges have a source.
    entity: Entity,
}
```

## 8. Relation to prior specs

| Prior claim | This design |
|---|---|
| Unified edges: one edge, TypeId match | Unchanged |
| `Product<T>` as entity capability ref | Removed; handles for buffers; `Entity` only for hierarchy |
| `Spatial` three behaviours | Same behaviours, keyed by `TypeId::of::<Entity>()`, not a marker type |
| `PortArena` holds all slot values | Removed; values on Inlets/Outlets components |
| `FieldKind` / `EdgeKind` | Removed; not part of the model |
| Parent §2.1 arena for low-cardinality signals | Signals live on Inlets/Outlets components instead |
| Parent §2.1 high-cardinality off edges | Unchanged (handles only on edges) |

## 9. Out of scope

- GPU upload / M5 residency beyond “handles must be able to name GPU buffers later”
- RON / project format (still M4)
- Tick rate, cook gating, MIDI
- Removing the need to clear `Events<T>` each tick

## 10. Success criteria

- No `FieldKind`, `EdgeKind`, `Product`, `Spatial`, or `PortArena` in the graph
  crate (or editor snapshot kinds enum)
- Hierarchy remains expressible as ordinary graph edges and is backed by `ChildOf`
- Geometry / material flow via handle values with TypeId matching
- `Events<T>` remains an ordinary reflected value with type-data clearing
- Policies exist only as TypeId / type-data branches in compile and tick, not as
  model variants
