## Context

Today a value wire is a Bevy `Relationship` on the consumer (`Wire` with associated `Source` / `Target` and a typed `propagate`). `register_wire::<W>` monomorphizes collect / legality / insert / remove / `propagate_of::<W>` into `WireRegistry`. Behaviours are a parallel `BehaviourRegistry` of `fn(&mut World, Entity, &TickCtx)`. The tick never reads those resources: `GraphOrder` already holds fn pointers. The editor keys inlets by registry order; the document keys them by `Wire::NAME`. See proposal.md for why that dual catalog goes away. Specs: `specs/graph/spec.md`, `specs/document/spec.md`, `specs/editor/spec.md`.

Constraints: Bevy 0.19 `bevy_reflect` (`#[reflect_trait]`, `ReflectComponent`, `AppTypeRegistry`); `sway-graph` still must not depend on `bevy_render` or the document format; `ComponentDocRegistry` stays for authorable **component** short names.

## Goals / Non-Goals

**Goals:**

- One catalog: `AppTypeRegistry` type data (`ReflectWire`, `ReflectBehaviour`). No sidecar TypeData.
- One shared field-copy `propagate`; typed overrides only where conversion is the meaning (`RotationFrom`) or the wire is structural (`ChildOf`).
- Rebuild stores `src` on the step. The tick `FromReflect`s a stack copy of the relationship (`Copy` Entity newtype), fetches outlet and inlet from `Wire` methods, and calls `propagate` on that copy.
- Behaviours as a second reflected trait: a node is optional **inlets**, **state**, and **outlets**. `evaluate` reads inlets, read/writes state in place, write-only outlets in place. No return struct; the trait never sees `&mut World`.
- Keep `GraphOrder` as a baked step list; steps identify types, not registry fns.

**Non-Goals:**

- Event wires / a reflected `EventWire` catalog (architecture §3; not implemented).
- Collapsing dedicated relationship types into one generic wire component (one inlet type still needs one component type).
- Short-name aliases or `#[type_path = "sway::wire"]` wrappers; full crate type paths are the keys, including `bevy_ecs::hierarchy::ChildOf`.
- Reflecting behaviour *computation* (wave, math, remap); only dispatch.
- Behaviours that reach services, other entities, or `&mut World` (architecture’s old “mechanically `&mut World`” convention). That work stays ordinary systems or observer triggers.
- Migrating version-1 documents.

## Decisions

### 1. Two object-safe traits, not one

`Wire` lives on the relationship type. A **node** is up to three optional components; `Behaviour` is the function over them, not a wire.

| Part | Role | Editor | Document |
|---|---|---|---|
| Inlets | Ports: authored and/or driven by wires | Fields + inlet sockets | Yes (authorable) |
| State | Internal memory for the behaviour | Hidden | No |
| Outlets | Values other wires can read | Outlet sockets only | No |

Each part may be absent. Today’s nodes have inlets + outlets and no state (`Oscillator` / `Math` / `Remap` / `Vec3Value` → `FloatOut` / `Vec3Out`).

`Relationship` is `Sized` and has an associated target type, so it cannot be a supertrait of `#[reflect_trait] Wire`. The component still derives `#[relationship]`; the reflected trait is a parallel surface.

```rust
#[reflect_trait]
pub trait Wire {
    fn producer(&self) -> Entity;
    fn source_type(&self) -> TypeId;
    fn target_type(&self) -> TypeId;
    fn source_path(&self) -> &'static str;
    fn target_path(&self) -> &'static str;
    fn propagate(&self, outlet: &dyn PartialReflect, inlet: Mut<dyn Reflect>);
}

#[reflect_trait]
pub trait Behaviour {
    fn state_type(&self) -> Option<TypeId>;
    fn outlet_type(&self) -> Option<TypeId>;
    fn evaluate(
        &self,
        state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        ctx: &TickCtx,
    );
}
```

The tick owns `World`. A `Propagate` step already has `src`, `dst`, and `type_id` from rebuild. The tick `FromReflect`s the relationship on the consumer onto the stack (one `Entity`, `Copy`), drops that world borrow, then `get_entity_mut([src, dst])`, reflects the producer’s **outlet** (`source_type()`) as `&dyn PartialReflect`, and the consumer’s **inlet** / target (`target_type()`) as `Mut<dyn Reflect>`, then `propagate` on the stack copy. Possible because producer ≠ consumer (Bevy drops self-edges). The live relationship is not borrowed during `propagate`.

`Mut<dyn Reflect>` rather than a raw `&mut`: `get_mut` marks `Changed` unconditionally. Default field copy (and `RotationFrom`) must `map_unchanged` the target field and write only when `reflect_partial_eq` is not `Some(true)`.

“Inlet” here is the wire’s target component (`Oscillator`, or `Transform` for translation), not only a behaviour inlets struct. “Outlet” is the producer’s source component (`FloatOut`, `Vec3Out`, …).

If `#[reflect_trait]` cannot take `Mut<dyn Reflect>`, keep `propagate` on the trait and call it on the `FromReflect` value as `&dyn Wire` without putting that method on the reflect vtable.

For `Behaviour`, `&self` is the **inlets** after inbound wires this tick (read-only). When a node has no inlets, implement `Behaviour` on a non-authorable marker component; `&self` is unused. **State** is `Option<Mut<dyn Reflect>>` — read/write in place, not a return value. If the node has a state type and the component is missing, the tick inserts `Default` / `FromWorld` first, then passes `Some`. **Outlets** are the same shape, write-only: the tick seeds a slot if needed; `evaluate` must not depend on the previous outlet value. `TickCtx` is this tick’s clock. No `Entity`, no `&mut World`, no `BehaviourOutput`.

Equal-value: writes go through `Mut` (`map_unchanged` / `reflect_partial_eq`) so an unchanged state or outlet is not marked `Changed`.

`Behaviour` methods `state_type` / `outlet_type` return `Option<TypeId>`. Rebuild collects entities that have the `Behaviour` carrier (inlets or marker). Wires target **inlet** fields; wire sources are **outlet** components.

Default `Wire::propagate` is a shared helper that copies `source_path()` (tuple field `"0"`) onto `target_path()`. `RotationFrom` overrides `propagate`. `ChildOf` returns an empty `target_path` so the default is a no-op.

**Alternative:** a single trait on the relationship that also runs node logic. Rejected: a behaviour is not routing.

**Alternative:** keep typed `propagate(src: &S, dst: Mut<T>)` and only erase dispatch. Rejected: that is still one monomorphized body per inlet.

**Alternative:** `propagate(&self, world, consumer)` and fetch inside the method. Rejected: `World` is only needed to look up the two components the tick already has.

**Alternative:** `Behaviour::run(&self, world, entity, ctx)` as today’s free functions. Rejected: World access belongs to systems and observers.

**Alternative:** return a new state box from `evaluate` while also taking `&state`. Rejected: that duplicates the write; state is read/write through `Mut`.

### 2. Metadata lives on `Wire`, not TypeData

`#[reflect_trait]` methods need a value. There is no `WireInfo` and no `Entity::PLACEHOLDER`.

Call `Wire` methods only on a value that already exists: `FromReflect` of the live relationship (tick, rebuild, emit, a connected inlet), or `W::from(producer)` with the **real** producer at insert. Registry membership is a point lookup: that `TypeId` / type path has `ReflectWire`. It does not need an instance.

Editor snapshot and connect legality **scan canvas entities and the components they already carry**. Inlet sockets come from that entity’s inlet part and from any component whose type is a reflected wire. `accepts_from` walks other canvas entities and checks those entities’ components (does this producer actually have the source this wire names; does this consumer actually have the target). The editor does not iterate the type registry to invent sockets, and it does not synthesize a dummy `Entity` to ask `source_type` / `target_type`.

`field_wire!` implements `Wire` (producer, type ids, paths, default propagate). Registration is `#[derive(Reflect)] #[reflect(Component, Wire)]` plus `register_type::<W>()`.

**Alternative:** a `WireInfo` `FromType` sidecar (source/target `TypeId`, paths, `propagate` fn pointer). Rejected: a second catalog next to `ReflectWire`.

**Alternative:** `FromReflect` a stack value with `Entity::PLACEHOLDER` so type-erased code can call methods with no live component. Rejected: the editor can scan entities; insert already has the producer; tick already has the relationship.

**Alternative:** dummy `FromWorld` placeholders in the world just to call methods. Rejected: extra world traffic and easy to mistake for a real wire.

### 3. Borrowing: World stays in the tick

The relationship is routing plus type/path methods (`Entity`, `Copy`). Rebuild stores `src` on the step. For each propagate step:

1. Reflect the relationship on the consumer; `FromReflect` onto the stack; drop the world borrow.
2. `get_entity_mut([src, dst])` using the step’s entities.
3. `outlet = ReflectComponent::reflect` on the producer for `wire.source_type()` (shared).
4. `inlet = ReflectComponent::reflect_mut` on the consumer for `wire.target_type()` (`Mut<dyn Reflect>`).
5. `wire.propagate(outlet, inlet)` on the stack copy.

Missing source or target: skip the call (no panic). A live `&Wire` on the consumer cannot overlap `Mut` of the inlet on the same entity; the stack copy is why `&self` is safe.

Behaviours: after inbound wires, clone inlets (`&self`). If the node has state or outlets, the tick inserts a default component when missing, then `reflect_mut`. `evaluate(&self, state, outlets, ctx)` writes through those `Mut`s. Do not pass previous outlet values as a logical input. `state_type` / `outlet_type` are methods on `Behaviour` (the cloned inlets already exist); no `BehaviourInfo` sidecar.

**Alternative:** a TypeData `propagate` fn pointer so the tick never holds `&W`. Rejected: that is `WireInfo` again.

### 4. `GraphOrder` keeps steps, drops fn pointers

```rust
enum Step {
    Propagate { src: Entity, dst: Entity, type_id: TypeId, wire: &'static str },
    Run { entity: Entity, type_id: TypeId },
}
```

`wire` is `TypePath::type_path()` for diagnostics and the editor. Rebuild clones `AppTypeRegistry`, iterates registrations that have `ReflectWire` / `ReflectBehaviour`, collects instances by `ComponentId`, Kahn-sorts entities as today. Tick clones the registry `Arc` off the world, then for each `Propagate` `FromReflect`s the relationship, fetches outlet + inlet from `source_type` / `target_type`, and calls `dyn Wire::propagate`, and for each `Run` clones inlets, seeds/muts state and outlets from `Behaviour` methods, and calls `evaluate`.

`PropagateFn` / `register_wire` / `register_behaviour` / `WireRegistry` / `BehaviourRegistry` go away. `WiresPlugin` no longer `init_resource`s those two.

**Alternative:** walk the type registry every tick. Rejected: show builds must not rescan; the baked list is the show artifact.

### 5. Topology hooks replace `watch::<W>`

A helper used when registering a wire or behaviour type installs `on_add` / `on_remove` hooks that set `TopologyDirty` only if `Authoring` exists. This also fixes today’s gap: adding an `Oscillator` without inserting a wire never marked dirty.

`ChildOf` already has relationship hooks; the dirty hook is additional and must not skip Bevy’s parenting maintenance.

### 6. Document and editor share the type path

`FORMAT_VERSION = 2`. Wire map keys are `type_path()` strings. Apply/emit: a type path is a wire iff that registration has `ReflectWire`. Insert `W::from(producer)` with the producer entity from the document (or `ReflectComponent` apply onto a value built from that entity). `UnknownWire` means “no registration with that path and `ReflectWire`.”

`EditorCommand::{Connect,Disconnect}` use the same `&'static str`. Connect is legal when the named type is a reflected wire **and** a scan of the two entities’ components shows the producer has the source and the consumer has the target. Snapshot `InletView.wire` / `EdgeView.wire` are type paths, discovered by scanning each canvas entity’s components (inlet parts and reflected-wire relationships), not by walking the catalog with a dummy instance. Drop `from_field`, `to_field`, `to_index` from `EdgeView`. `SocketKind::Inlet` holds the path (or a `TypeId` plus path for display). `node_box` may still space sockets by a sorted list of paths; hit-test maps visual slot → path.

`Wire::NAME` is deleted as a key. Socket labels may show `TypePath::ident()` (`TranslationFrom`).

### 7. Node crate follow-ons

- `field_wire!` grows `Reflect` / `#[reflect(Component, Wire)]` / `From<Entity>` and `Wire` methods (type ids, paths, default propagate); it no longer takes a typed `propagate` closure except through an opt-in override.
- `RotationFrom` is not generated as default copy; it keeps a custom `propagate`.
- `impl Behaviour for {Oscillator, Vec3Value, Math, Remap}` (those types are inlets): `evaluate(&self, None, Some(outlets), ctx)` writes `FloatOut` / `Vec3Out` in place. No `World`. `#[reflect(Behaviour)]`. Stop `register_authorable` on `FloatOut` and `Vec3Out` — they are outlets, not document/inspector fields. `#[require(*Out)]` may still seed runtime presence.
- `MaterialOut` gets `Reflect` + `#[reflect(Component)]` so field copy can read it. It stays off `ComponentDocRegistry`.
- `WireNodesPlugin` calls `register_type_data::<ChildOf, ReflectWire>()` because `#[reflect(Wire)]` cannot be added on Bevy’s type. Register `MeshMaterial3d<StandardMaterial>` if the target path must resolve through the type registry.

### 8. Tests

Replace `propagate_of::<W>(world, src, dst)` with a helper that `FromReflect`s the wire, fetches outlet/inlet from its type methods, and calls `dyn Wire::propagate`. Keep the equal-value tests. Rewrite document fixtures and `demo.sway.ron`. Replace `to_field == 1` with “edge.wire is `ChildOf`’s type path.”

## Risks / Trade-offs

- [Tick cost of type-registry lookup + reflect field access] → Mitigation: rebuild bakes entity `TypeId`; tick clones the `Arc` registry once per tick. A `FromReflect` of a `Copy` relationship is one `Entity`. Cache `ReflectComponent` handles on the step if profiling shows it. Tens of wires at 120 Hz.
- [Equal-value misses if `reflect_partial_eq` returns `None`] → Mitigation: treat `None` as “not equal” and apply (may over-dirty, never skip a real change). Pin with the existing change-detection tests; `#[reflect(PartialEq)]` on source/target types used by field copy.
- [Bevy moves `ChildOf`’s type path] → Mitigation: accepted; documents pin `bevy_ecs::hierarchy::ChildOf`. A newtype wrapper is a later change if an upgrade breaks files.
- [Generic `MeshMaterial3d<M>` not registered] → Mitigation: register the `StandardMaterial` instantiation in `WireNodesPlugin`; test MaterialFrom through the reflected path.
- [Hook double-fire with relationship insert] → Mitigation: dirty flag is idempotent; tests for insert/remove still assert one rebuild.

## Migration Plan

No live-document migration. Bump `FORMAT_VERSION` to 2; parse rejects 1. Rewrite `crates/sway-app/assets/demo.sway.ron` and apply/emit tests in the same change. Rollback is revert the change; old files remain version 1 and load on the previous build.

## Open Questions

None. Remaining choices (cache type data on `Step` vs lookup each tick; socket label = `ident()` vs a display string) do not change specs or the task shape.
