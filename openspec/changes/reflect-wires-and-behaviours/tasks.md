## 1. sway-graph: reflected Wire and field copy

- [x] 1.1 Replace `Wire` with an object-safe `#[reflect_trait]` (`producer`, `source_type`, `target_type`, `source_path`, `target_path`, `propagate(&self, outlet, inlet)`). Default reflected field copy via those paths (immutable `reflect_partial_eq`, `map_unchanged` + apply; empty `target_path` is a no-op). Delete associated `Source`/`Target`, `NAME`, `propagate_of`, `PropagateFn`, and any `WireInfo` TypeData. The tick `FromReflect`s a stack copy, fetches outlet/inlet from the type methods, and calls `propagate`; it does not pass `&mut World`.
- [x] 1.2 Add a registration helper that `register_type`s a wire and installs `Authoring`-gated `on_add`/`on_remove` topology hooks. Implement `Wire` for `ChildOf` (empty target path). Update `test_wires::GainFrom` to the new surface.
- [x] 1.3 Port `wire.rs` tests (copy, despawned producer, missing source/target, equal-value no dirty) onto a helper that `FromReflect`s the wire, fetches outlet/inlet, and calls `dyn Wire::propagate`. Run `cargo test -p sway-graph --lib wire`.

## 2. sway-graph: reflected Behaviour, order, tick

- [x] 2.1 Add `#[reflect_trait] Behaviour` with `state_type` / `outlet_type` (`Option<TypeId>`) and `evaluate(&self, state: Option<Mut<dyn Reflect>>, outlets: Option<Mut<dyn Reflect>>, ctx)` (`&self` = inlets; state read/write in place; outlets write-only in place; no `World` / `Entity` / `BehaviourOutput` / `BehaviourInfo`). Tick inserts default state/outlet when the type is present but the component is missing. Registration helper: `register_type` + `#[reflect(Behaviour)]` + topology hooks. Delete `BehaviourRegistry`, `register_behaviour`, and `BehaviourFn` as a public catalog API.
- [x] 2.2 Rebuild `GraphOrder` from `AppTypeRegistry` (`ReflectWire` / `ReflectBehaviour` + `ComponentId` collect). Change `Step` to `Propagate { src, dst, type_id, wire: type_path }` and `Run { entity, type_id }`. Delete `WireRegistry`, `register_wire`, and `registry_wires` catalog types. Tick: fetch outlet + inlet, `propagate`; clone inlets, `Mut` state and outlets, `evaluate`.
- [x] 2.3 Point `EditorCommand::{Connect,Disconnect}` at type path + `ReflectWire` / `ReflectComponent`. Insert `W::from(producer)` with the real producer. Legality: type is a reflected wire, and a scan of the two entities’ components shows source on the producer and target on the consumer. Drop `watch::<W>` systems in favor of the hooks. `WiresPlugin` no longer inits the two registries.
- [x] 2.4 Port order/rebuild/diagnostics/watch/command/run tests (including “add a behaviour without a wire still rebuilds”). Run `cargo test -p sway-graph`.

## 3. sway-nodes: field_wire, behaviours, ChildOf

- [x] 3.1 Rewrite `field_wire!` to emit `Reflect` / `#[reflect(Component, Wire)]` / `From<Entity>` / `Wire` methods (type ids, paths, default propagate) instead of a typed `propagate`. Keep a custom `propagate` on `RotationFrom`. Register every wire type (and `ChildOf` as `ReflectWire`) from `WireNodesPlugin`; stop calling `register_wire`.
- [x] 3.2 Move `oscillator_behaviour` / `vec3_behaviour` / `math_behaviour` / `remap_behaviour` onto `impl Behaviour for …` on the inlet types (`evaluate` writes outlets in place; current nodes have no state). Stop calling `register_behaviour`. Remove `register_authorable` for `FloatOut` and `Vec3Out`. Leave `sync_pbr_materials` as a `Changed<T>` system.
- [x] 3.3 Add `Reflect` + `#[reflect(Component)]` to `MaterialOut` (do not `register_authorable`). Register `MeshMaterial3d<StandardMaterial>` if needed for the target path. Port `wire_testing` and node tests off `propagate_of`. Run `cargo test -p sway-nodes`.

## 4. sway-document: type-path keys

- [x] 4.1 Set `FORMAT_VERSION` to 2. Apply/emit walk `ReflectWire` by `type_path()`; unknown path skips without removing; omitted catalog wires disconnect. Reject version 1. Emit/apply only authorable inlets — not state, not outlets.
- [x] 4.2 Rewrite apply/emit fixtures from short names (`factor`, `translation`) to full type paths. Run `cargo test -p sway-document`.

## 5. sway-editor: sockets by type path

- [x] 5.1 Snapshot: `InletView.wire` / `EdgeView.wire` are type paths; drop `from_field` / `to_field` / `to_index`. Inlet list by scanning each entity’s components (inlet part + reflected-wire relationships); point-lookup `ReflectWire` for validity; `accepts_from` by scanning other canvas entities’ components. Sorted by path for painting only. Inspector lists inlets only; outlets are sockets; state is omitted.
- [x] 5.2 Canvas / `node_box`: `SocketKind::Inlet` holds the path; hit-test maps visual slot → path; connect/disconnect send that path. Replace the `to_field == 1` test with `ChildOf`’s type path. Run `cargo test -p sway-editor`.

## 6. sway-app demo and architecture note

- [x] 6.1 Rewrite `crates/sway-app/assets/demo.sway.ron` to version 2 and type-path wire keys. Run `cargo test -p sway-app --test demo_document`.
- [x] 6.2 Update `docs/architecture.md` §2–4 so the wire/behaviour contract matches this change (reflected traits, no registries, type-path identity, inlets/state/outlets, pure evaluate). Do not rewrite historical Superpowers plans.
