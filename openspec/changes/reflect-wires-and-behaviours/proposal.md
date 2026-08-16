## Why

`WireRegistry` and `BehaviourRegistry` exist only because `Wire::propagate` and behaviour run functions are not object-safe: registration monomorphizes a fn pointer per type and the editor keys inlets by that `Vec`'s order. Bevy's type registry already catalogs reflected types, so a parallel table is a second source of truth that blocks a single reflected field-copy propagate and makes inlet identity fragile.

## What Changes

- Replace `WireRegistry` with `#[reflect_trait]` `Wire` type data on each relationship type. `propagate(&self, outlet, inlet)` reads the producer outlet and writes the consumer inlet; type ids and field paths are methods on the same trait. The tick `FromReflect`s a stack copy and fetches those two components — no `&mut World` on the trait, no sidecar TypeData. Default body is one reflected field copy. `ChildOf` is a no-op; `RotationFrom` is the only typed override (euler degrees → quat).
- Replace `BehaviourRegistry` with a separate `#[reflect_trait]` `Behaviour` on the **inlets** type (or a marker if there are no inlets). A node is optional inlets, state, and outlets. `evaluate(&self, state, outlets, ctx)` reads inlets, read/writes state in place, write-only outlets in place. `state_type` / `outlet_type` are methods on the same trait. No return struct, no `&mut World`, no sidecar TypeData. State and outlets are not document or inspector fields.
- Drop `register_wire` / `register_behaviour` as catalog APIs. `app.register_type::<T>()` plus `#[reflect(Wire)]` / `#[reflect(Behaviour)]` is registration. Topology watches become component hooks on those types, gated on `Authoring`.
- `GraphOrder` stays a baked step list. `Propagate` / `Run` steps carry `TypeId` (and the type path for diagnostics), not monomorphized fn pointers.
- **BREAKING:** Document wire map keys become the wire type's full `TypePath` (`sway_nodes::spatial::TranslationFrom`, `bevy_ecs::hierarchy::ChildOf`). `FORMAT_VERSION` increments; version 1 is refused. Component keys stay short names via `ComponentDocRegistry`.
- **BREAKING:** Editor sockets, edges, and `Connect` / `Disconnect` identify an inlet by that same type path. The snapshot discovers sockets by scanning each entity’s components and checks connect legality on those entities; it does not walk the wire catalog with a dummy instance. Inlet ordinals (`to_field`, `to_index`, `SocketKind::Inlet(u16)`) go away. Visual layout may still sort paths for painting; that order is not identity.

## Capabilities

### New Capabilities

- `graph`: Value-wire and behaviour contracts, type-registry catalog, rebuild, tick dispatch, and change-detection rules for reflected propagate.
- `document`: Project document wire keys as full type paths, version bump, apply/emit against the type registry.
- `editor`: Canvas / snapshot inlet and edge identity keyed by wire type path; connect/disconnect commands.

### Modified Capabilities

- (none — `openspec/specs/` is empty; these domains are created here)

## Impact

- `sway-graph`: `wire.rs`, `registry_wires.rs` (delete the two resources), `order.rs`, `run.rs`, `watch.rs`, `command.rs`, `lib.rs` exports. `ComponentDocRegistry` / `register_authorable` stay.
- `sway-nodes`: `field_wire!` generates reflect + `Wire` methods instead of a typed `propagate`; `Behaviour` impls on inlet types; drop `register_authorable` for `FloatOut` / `Vec3Out`; `MaterialOut` gains `Reflect` / `ReflectComponent` (still not authorable); `ChildOf` type data registered from the plugin.
- `sway-document`: apply/emit walk `ReflectWire` instead of `WireRegistry`; `demo.sway.ron` and crate fixtures rewrite keys; `FORMAT_VERSION` 2.
- `sway-editor`: snapshot, canvas, node_box socket addressing.
- Tests that call `propagate_of::<W>` or assert `to_field` / `Wire::NAME` keys.
- Event wires (architecture §3) are out of scope; they are not implemented as a registry yet.
