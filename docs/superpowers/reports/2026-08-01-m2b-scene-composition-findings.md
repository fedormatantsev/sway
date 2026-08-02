# M2b scene composition — findings

Consolidated answers to the five questions required by
`docs/superpowers/specs/2026-08-01-m2b-scene-composition-design.md` §12, plus
the implementation facts a later milestone would otherwise have to rediscover.
Evidence is the committed code and tests on this branch and the measurements
recorded below; where a claim rests on a measurement, the exact command that
produced it is given.

## 1. Did the sticky dirty flag hold?

**It held as a mechanism, and it is incomplete as a specification.** Every
property §6 and §10 asked for is implemented and covered by a discriminating
test in `crates/sway-graph/src/tick.rs`'s `cooking` module:

| Property | Test |
|---|---|
| every node cooks once after compilation | `every_node_cooks_exactly_once_after_compilation` |
| an idle graph cooks nothing | `a_steady_graph_cooks_nothing_after_the_first_tick` |
| an upstream cook reaches its consumer the same tick | `an_upstream_cook_propagates_to_its_feeds_consumer_in_the_same_tick` |
| dirt flows with `Feeds` direction only | `a_param_change_on_one_node_does_not_cook_its_upstream` |
| a node joining mid-session still cooks | `a_node_added_after_an_upstream_cook_still_cooks_against_it` |
| a driven input that moved dirties its node | `a_driven_input_that_changed_dirties_the_node` |
| a driven input held still does not | `a_steady_driven_input_does_not_dirty_the_node` |

The last two are the pair §6 exists for — the case that fails if the gate
reads the `Params` change tick — and they pass. §6's diagnosis was right, and
the sticky flag is the correct fix for it.

**What §6 did not anticipate: the flag is one bit per node, not one bit per
reason.** A node with both param ports and geometry slots is dirtied by *any*
of its reasons and then has no way to ask which one fired. `MeshNode` is that
node. Its params are `translation`/`rotation_*`/`scale`, so an edit to the
mesh's position dirties it, its cook runs, and — with nothing further — it
re-uploads the mesh asset for a geometry that did not change. `Assets::get_mut`
marks the asset `Modified` by the act of being called, so that is precisely
the waste §2.11 names.

The implementation therefore carries a **second, node-local gate inside the
cook**: `GeometryFingerprint`, `P`'s `Arc` pointer plus the point count
(`crates/sway-nodes/src/mesh.rs:53-78`), compared before the asset is touched
at all (`re_cooking_unchanged_geometry_does_not_modify_the_asset`). The engine
gate decides *whether to call cook*; a node that owns an expensive resource
still has to decide *whether to write it*. That two-level structure works, but
§6 and §7 present the engine gate as the whole answer, and it is not.

**Yes, there is a cook whose correctness depends on a value the gate does not
observe — two of them.** Both are sound today for reasons that are not the
gate's doing:

- **The Mesh fingerprint tracks `P` only.** A change to `N`, `uv` or the index
  buffer that leaves `P`'s `Arc` untouched does not re-upload, though
  `geometry_to_mesh` bakes all four into the same `Mesh`. Unlike the
  fingerprint's other imprecisions, this one fails toward a *silently stale
  mesh*, not a wasted upload. Nothing in today's operator set reaches it —
  `Grid` rewrites every attribute, `Displace` rewrites only `P` — but a
  "recompute normals" or "UV project" operator would. Documented at the type
  (commit `76c87ed`) rather than fixed, because fixing it means deciding what
  a cheap whole-`Geometry` identity is, and that decision belongs with M5's
  GPU residency rather than ahead of it.

- **A material node never dirties its `Mesh` consumer.** `produced_change_tick`
  defaults to `None`, which the material node keeps, so the stored-tick path
  is inert across a `material` slot. This is right in the sense §6 intended —
  consumers hold a `Handle`, and editing the material's colour does not change
  the handle. But `MeshNode::cook` *does* read that handle and insert
  `MeshMaterial3d` from it, so a cook that never runs is a handle that never
  gets re-read. It is safe today only because nothing recreates a handle
  mid-session, and because compilation dirties every node, so the one moment a
  handle is created is also a moment every node cooks. Neither fact is
  enforced anywhere.

Both are recorded in §11-style terms rather than fixed: they are correct now,
and the invariants holding them up are undocumented in the design.

## 2. Did two orders hold?

**Yes, and nothing in the M2b node set even strained it.** `compile` emits
`cook_order` over the `Feeds` DAG (`compile.rs:668`) and `tick_order` over the
param DAG, `ParentEdge` enters neither, and `graph_tick` runs the two passes
back to back (`tick.rs:129` and `tick.rs:214`).

The demo graph exercises the one interaction that could have wanted a
cross-DAG constraint, and it resolves in favour of the design as written:
`Rgb --param--> StandardMaterial` and `StandardMaterial --feeds--> Mesh` means
a value crosses from the param DAG into the `Feeds` DAG within one tick.
Because ticks precede cooks *globally* rather than per node, the material
node's tick has already written its handle and applied its colour before the
`Mesh` cook reads the handle. §7's "step B before step C" ordering delivers
this for free; no ordering edge spanning both DAGs was needed.

**The case that would break it did not arise:** a node whose *tick* depends on
something a *cook* produces this same tick — a hypothetical `PointCount` node
outputting its input geometry's length as a signal. That inverts the global
pass order and two passes cannot express it; it would need either a third
pass or a fixpoint. No M2b node reads `Geometry` in `tick`. Worth naming now
so that when such a node is proposed it is recognised as an architectural
change rather than a new node type.

## 3. What did `Geometry`'s `Arc` sharing actually save?

**Measured at the demo graph's own grid size, 14.2×.**

```
cargo test -p sway-geo --release measure_arc_clone_vs_deep_clone_at_demo_grid_scale \
  -- --ignored --nocapture
```

At N = 2304 (48×48, `demo_graph.rs`'s `GridParams`) with three attributes
(`P`, `N`, `uv`), 200,000 iterations each:

| | per clone |
|---|---|
| `Geometry::clone()` as `Displace::cook` actually performs it | **81 ns** |
| deep-copy counterfactual (owned `Vec<T>` per attribute) | **1.159 µs** |

So the saving is ~1.08 µs per operator per cook at this size, and it scales
with N and attribute count while the `Arc` path stays flat — the `Arc` figure
is a three-entry `BTreeMap` clone plus three refcount bumps, and is
independent of N entirely.

Two honest qualifications:

- **This measures the pass-through, which is what §2.11's claim was about.**
  It does not measure the attribute the operator rewrites: `Displace` still
  allocates a fresh 2304-element `Vec<Vec3>` for `P` every cook, and that
  allocation is unaffected by the `Arc` decision. The saving is on `N` and
  `uv`, and it grows as operators carry more attributes they do not touch —
  which is the normal case for a deep chain.
- **The sharing is asserted structurally, not only measured.**
  `displace.rs:141` asserts `Arc::ptr_eq` on `N` after a cook and
  `displace.rs:151` asserts the negation on the rewritten `P`, which is what
  §10 asked for and what distinguishes the claim from a copy that happens to
  be fast.

`Arc` identity also turned out to be load-bearing for something §5 did not
anticipate: it is the mesh upload gate (§1 above). The refcount discipline
buys a cheap change-detection signal on top of the copy it avoids.

## 4. How did slot typing read at the call site?

**`Slots` plus `Produces` is the right split, and it was not sufficient — a
third member was needed.**

The split reads well. A node declares what it accepts as a reflect struct of
`Slot<T>` fields and what it offers as one type:

```rust
#[derive(Reflect, Default)]
pub struct MeshNodeSlots {
    pub geo: Slot<Geometry>,
    pub material: Slot<MaterialOf<StandardMaterial>>,
}
// ...
type Slots = MeshNodeSlots;
type Produces = ();
```

Neither wants to be the other. They are genuinely different shapes — a node
has many slots and at most one product — and the structure pass's check is a
plain `TypeId` comparison between the slot's declared capability and the
source's `Produces`, static, reading no components, exactly as §4 promised.
Adding a capability touches no engine code: `MaterialOf<M>` is declared in
`sway-nodes` and `sway-graph` never learns it exists.

**What was missing is `produced_change_tick`.** `Produces` names a capability
but carries no way to ask *when it last changed*, which is exactly what §6's
third dirty source needs. Adding a `ProducedTickFn` to the registry
(`registry.rs:68-79`) is what lets the engine gate on `Geometry` without
`sway-graph` depending on `sway-geo`. §4's trait sketch does not have it; it
is a required member of the split, not an optional extra.

**Two smaller deviations from §4's sketch, both forced:**

- `Produces` is bounded `TypePath + Send + Sync + 'static`, not the bare
  `'static` §4 specified. `Slot<T>` needs `T: TypePath` to populate
  `ReflectSlot::capability_path`, and error messages use `type_path()` rather
  than §4's stated `type_name`. `Geometry` derives `TypePath` only, so §4's
  actual concern — not forcing `Reflect` onto `Arc<Vec<T>>` storage — is
  still honoured.
- Slot ordinals needed `SLOT_ORDINALS` as a separate const alongside
  `PORT_ORDINALS`, carrying M2a's `(name, ordinal)` correction unchanged.
  Name-only matching was already known to be insufficient; the same applies to
  slots for the same reason.

**`Slot<T>` derived `Reflect` without incident.** M2a's finding that a
`PhantomData<fn() -> T>` field can be `#[reflect(ignore)]`d in `bevy_reflect`
0.19 carried over exactly; no non-generic fallback was needed, matching
`Event<T>`'s shape.

**`Group` did need the scalar `rotation_y` port Task 11 anticipated.** Both
`GroupParams` and `MeshNodeParams` carry `rotation_x`/`rotation_y`/
`rotation_z` as three `f32`s rather than one `Vec3` or `Quat`. The reason is
§2.4's own rule: a node's ports are its fields, rotation is what a signal
actually drives, and every M2a signal node outputs `f32`. A `Vec3` rotation
port would have needed a vector-producing node that does not exist.
`translation` and `scale` stay `Vec3` because nothing drives them at M2b —
when something does, they will face the identical choice.

## 5. What the tick costs with cooks in it

**A data point for §11, explicitly not its answer.** Apple M4, optimized test
binary, 1,000 warm-up iterations then the timed run.

The headline figures call `graph_tick` directly against the world, which is
the only way to measure the graph rather than the harness around it:

| | per tick |
|---|---|
| `graph_tick`, gate closed (10 nodes, no cook runs) | **624 ns** |
| `graph_tick`, gate open (`Displace` + `Mesh` cook every tick, 2304 points) | **13.07 µs** |

So the two cooks cost ~12.4 µs and **the gate is worth ~20× on this graph.**

```
cargo test -p sway-app --release measure_ -- --ignored --nocapture --test-threads=1
```

Three cautions, each of which cost time to establish and would cost it again:

- **`--test-threads=1` is mandatory.** Run in parallel, the same three
  measurements read 44.4 / 56.9 / 40.6 µs across runs. Cargo's default
  parallelism contends for cores and inflates timings by up to 40%.
- **The `App::update()`-based figures are not measurements of the graph.**
  They sit on a fixture floor that dwarfs the signal:

  | fixture | per update |
  |---|---|
  | bare `App::new()`, no plugins | 8.12 µs |
  | `TimePlugin` only | 30.23 µs |
  | `MinimalPlugins` | 29.99 µs |
  | `MinimalPlugins` + `AssetPlugin` + 2 `init_asset` | 39.24 µs |
  | the above + the compiled demo graph, cooks gated off | 40.22 µs |
  | the above, `Displace.amount` edited every tick | 62.03 µs |

  The graph contributes ~1 µs of the idle 40 µs. Everything else is
  `App::update()` machinery.
- **M2a's 2.226 µs/tick figure is not reproducible in this environment and
  should not be compared against these.** M2a measured a `TimePlugin`-only
  `App::update()` (`sway-nodes/tests/traces.rs:387`); the same fixture with
  *no graph at all* measures 30.23 µs here. Whatever the discrepancy is, it is
  in the harness, not in either milestone's engine, and the comparison the
  M2a report invites cannot be made. Future milestones should time
  `graph_tick` directly.

One result worth carrying forward, with its caveat attached: the
`App::update()` delta for cooking (62.03 − 40.22 = 21.8 µs) is **larger than
the cooks themselves** (12.4 µs), by ~9.4 µs. The likely explanation is the
asset system reacting to a `Mesh` marked `Modified` every tick — which would
mean the gate suppresses downstream work as well as the cook, strengthening
§8's argument for gating the mesh upload. **This was not isolated**: the two
figures come from different harnesses (`App::update()` vs. direct
`graph_tick`), and no measurement here attributes the 9.4 µs. Treat it as a
hypothesis worth one experiment at M5, not as a result.

At 120 Hz a tick has 8.33 ms. Even the hot figure uses 0.16% of it. This says
nothing about the right tick rate: one 48×48 grid with two CPU cooks, no
renderer, no live MIDI callback and no M5 residency traffic is not the graph
that number should be chosen against. **§11's tick-rate question stays open.**

## What a later milestone would otherwise rediscover

- **`#[derive(Reflect)]` on `Slot<T>` needs nothing special.** M2a's
  `PhantomData<fn() -> T>` + `#[reflect(ignore)]` recipe transfers verbatim.
  `PhantomData<fn() -> T>` rather than `PhantomData<T>` keeps the marker
  `Send + Sync` regardless of `T`.
- **Unit-struct `TypeInfo` never came up.** `NoSlots`, `NoOutputs`,
  `GroupOutputs` and `MeshNodeOutputs` are all declared as braced structs
  (`struct NoSlots;` is a unit struct but is only ever reflected through
  `struct_info`, which accepts it). The anticipated obstacle did not
  materialise; `derive_schema` and `derive_slots` share one struct-cast
  preamble (commit `acc33ea`) and handle the empty case without a special
  branch.
- **`ChildOf`'s import path is `bevy::ecs::hierarchy::ChildOf`** (and
  `bevy_ecs::hierarchy::ChildOf` from a non-facade crate). It is *not* in
  `bevy::prelude` under a name that resolves unambiguously in a file that also
  imports graph types.
- **`Assets::get_mut` marks the asset `Modified` unconditionally**, by the act
  of being called and before any write. Read-compare-then-write is the only
  way to keep a still scene from re-uploading, and it applies to both the mesh
  and the material node.
- **`Assets::get_mut` can return `None` through a still-strong handle** if the
  asset was removed directly. A cook must not record "I uploaded this" when
  the write silently did nothing, or it will never retry
  (`a_failed_asset_write_does_not_falsely_record_the_new_fingerprint`).
- **A freshly built `App` cannot run `FixedUpdate` on its first `update()`.**
  Bevy's first `Time::<Real>` update always reports a zero delta by design
  (`bevy_time-0.19.0/src/real.rs:99-108`), so the fixed-timestep accumulator
  cannot reach its threshold on frame 0 regardless of wall-clock time or
  implementation. Every test fixture needs a warm-up `app.update()` before it
  can assert on anything the graph tick does. This bit `demo_graph.rs`,
  `test_nodes.rs`'s `headless_app` and `structure_app` identically.
- **`graph_tick` called as a bare function does not advance the world change
  tick.** `App::update()` does it as part of running a schedule. Any loop that
  calls `graph_tick` directly *and* mutates params between calls must call
  `world.increment_change_tick()` itself, or the prefill gate sees the same
  tick value every iteration and nothing is ever dirtied. This silently turned
  a "cooks every tick" measurement into a gate-closed one; it was caught only
  because the number came out equal to the idle case.
- **`reflect_partial_eq` returning `None` must mean "changed".** A
  freshly-resized arena slot holds `()`, which compares as `None`, and
  treating that as "unchanged" would skip the first real gather.
- **Kahn over `Feeds` needs the node-index → plan-index conversion applied to
  both `cook_order` and each plan's slot sources.** Tests that spawn nodes in
  dependency order have `topo_rank[i] == i` throughout and cannot catch a
  missing conversion; commit `7fc15fa` reworked two such tests to force a
  genuine permutation via a `ParamEdge`-connected probe.
- **`sway-midi`'s `virtual_destination_receives_midisend_note_on` is flaky
  under parallel test load.** It failed once with `Timeout` during a
  `cargo test --workspace` run on this branch and passed on re-run and under
  `--no-fail-fast`. It is a CoreMIDI loopback test with a wall-clock deadline,
  predates M2b (commit `d12145c`), and is unrelated to any M2b change — but a
  future milestone seeing a red workspace run should re-run before
  investigating.

## What was not proven

- **No operator rewrites `N`, `uv` or the index buffer while passing `P`
  through**, so the Mesh fingerprint's one stale-output blind spot is
  documented and untested rather than fixed.
- **No node recreates an asset handle mid-session**, so the material slot's
  inert `produced_change_tick` is safe by circumstance rather than by
  construction.
- **No graph wanted an ordering constraint spanning both DAGs**, so two orders
  is confirmed only against the M2b node set, not against a node that reads
  geometry in `tick`.
- **The measurements are a single machine, single run each.** No variance is
  reported and none was computed beyond establishing that parallel test
  execution invalidates them.
