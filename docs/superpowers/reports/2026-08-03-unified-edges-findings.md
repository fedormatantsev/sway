# Unified edges — findings

Consolidated answers to the five questions required by
`docs/superpowers/specs/2026-08-03-unified-edges-design.md` §13, plus the
implementation facts M4 would otherwise have to rediscover. The primary
source for what actually happened, as opposed to what a task brief assumed
going in, is `.superpowers/sdd/2026-08-03-unified-edges/progress.md`; where
this report cites a specific fix round or the Task 8/9 merge decision, that
ledger is the source, not the plan or a brief.

## 1. Did clear-in-place hold the event path's allocation profile?

**Measured on the same demo graph M2b measured, same discipline, and the
numbers hold within the same order of magnitude — gate-closed is slower by
about 1.7×, gate-open is statistically indistinguishable.**

Methodology matched M2b's exactly: Apple M4, `--release` binary, 1,000
warm-up iterations then a timed run of 100,000 iterations, `graph_tick`
called directly against the world (not `App::update()`), `--test-threads=1`.
The temporary test lived in `crates/sway-app/src/demo_graph.rs`'s existing
`#[cfg(test)] mod tests`, reusing its `app()` fixture and
`setup_demo_graph`, and was run with:

```
cargo test -p sway-app --release measure_ -- --ignored --nocapture --test-threads=1
```

For the gate-open case, `MidiCC`'s `tick` only overwrites its own outlet slot
when `TickMidi` holds a matching CC message (`midi.rs`'s `tick`); with none
queued it leaves the arena slot untouched, so the temporary test poked a new
`f32` directly into `MidiCC`'s output slot (located via
`CompiledGraph::plans[i].field_offsets`) before every `graph_tick` call, and
called `world.increment_change_tick()` each iteration — both are required,
per M2b's own finding that `graph_tick` called bare does not advance the
world's change tick, so an un-advanced tick silently turns a
"cooks-every-tick" measurement into a gate-closed one.

| | M2b (old split arena) | this milestone (unified arena) |
|---|---|---|
| `graph_tick`, gate closed (10 nodes, no cook) | 624 ns | **~1.05 µs** (1.104 / 1.047 / 1.007 µs across three runs) |
| `graph_tick`, gate open (`Displace` + `Mesh` cook, 2304 points) | 13.07 µs | **~12.78 µs** (12.837 / 12.738 / 12.759 µs across three runs) |

**The event/product path did not get measurably more expensive at this
graph's cook-heavy end, and did get measurably more expensive at the
signal-only end.** Gate-open is a wash — the two figures agree to within
noise, consistent with the design's own prediction that clear-in-place keeps
the occurrence-list allocation profile unchanged and that the 12.4 µs of
cook work dominates whatever the arena unification costs around it. Gate
closed is the case that isolates the arena's own overhead, since nothing
cooks and the entire figure is gather-plus-tick over ten nodes: it grew from
624 ns to ~1.05 µs, roughly +68%. Three candidate causes, not disentangled by
this measurement:

- **The graph is not really "the same graph."** M2b's ten-node demo graph
  predates the `MidiCC.value → Displace.amount` and `MidiNote → Envelope`
  edges Task 7 added when it re-authored `demo_graph.rs` against the unified
  model (`crates/sway-app/src/demo_graph.rs:146-158` wires five structural
  edges through `Product<T>` inlets plus five signal edges, all of which now
  enter the one topological sort and are gathered every tick). M2b's
  "10 nodes" figure and this milestone's "10 nodes" figure are the same node
  *count* but not a proven-identical edge set, so some of the delta may be
  more gathers per tick rather than a more expensive gather.
- **`Box<dyn PartialReflect>` downcasts for `Product<T>`'s `Option<Entity>`
  are new gather work that did not exist in M2b's split arena** — `Feeds`
  and `ChildOf` carried nothing at runtime before, so their old "gather" cost
  was zero; now every structural edge is a real arena copy plus a
  `reflect_partial_eq` comparison (`tick.rs:120-131`), unconditionally, every
  tick, gate open or closed. This is very likely the dominant cause of the
  gate-closed regression and is exactly the kind of cost §8 named as a risk,
  just distributed differently than §8's own emphasis (which was about
  event-list allocation, not about `Product` gathers becoming non-free).
- **Ordinary measurement noise** — three runs at ~5% spread around 1.05 µs is
  a small but real variance band at this timescale (sub-microsecond), on a
  machine running other things.

**No allocation counts were taken.** This is a wall-clock comparison only,
matching M2a/M2b's own precedent of not instrumenting allocator calls; it
answers "is the profile in the same ballpark" but not "how many allocations
per tick," so the causes above are plausible, not adjudicated. The temporary
test (`measure_graph_tick_gate_closed`, `measure_graph_tick_gate_open`) was
written, run three times each for stability, and removed before this commit
— `git diff --stat crates/sway-app/src/demo_graph.rs` shows no net change —
per M2a's house style: no benchmark code ships with this report.

## 2. Did one order cost anything real?

**No graph in the actual node set wanted the two-phase split back, and the
union-cycle rejection this design predicts is now a real, passing test — not
a hypothetical.**

`crates/sway-graph/src/compile.rs`'s
`a_union_cycle_across_both_old_dags_is_rejected` (line 995) builds exactly
the graph §4 of the design describes: two `Consumer` nodes, each with both a
`Product<Blob>` inlet and a `Product<Blob>` outlet, wired A→B→A. Under the
old two-pass model this would have needed one edge from each of the two old
DAGs to form a cross-DAG cycle; the test's own comment records that the
obvious version of that shape (`Consumer.blob → Producer.scale`) is instead
rejected earlier, by the ordinary type check, before the cycle check ever
runs — so the test had to use a node with both inlet and outlet on the same
capability to exercise the union-cycle path specifically. It compiles to a
`cycle` error, correctly, under the single topological sort.

No node in `sway-nodes`' seven files (`envelope.rs`, `lfo.rs`, `material.rs`,
`math.rs`, `mesh.rs`, `midi.rs`, `scene.rs`, migrated in Task 6) or in
`sway-app`'s demo graph (Task 7) wanted the tick/cook split restored. The one
place the design itself predicted friction —
`crates/sway-nodes/tests/traces.rs`'s `event-fan-in` fixture — is the
strongest evidence available that declared multiplicity actually replaced
engine-side fan-in cleanly: `build_event_fan_in` (line 320) wires two
`MidiNote` nodes into `Envelope.triggers[0]` and `Envelope.triggers[1]`
(`Vec<Events<NoteMsg>>`, `Envelope::TRIGGERS` addressed at explicit indices),
and `Envelope` merges the two streams itself rather than the engine doing it.
The trace's own comment (`traces.rs:324-329`) records that the ordering
(`midi_a` at element 0, `midi_b` at element 1) was chosen to reproduce the
old engine's compiled-rank fan-in order exactly, and the test passes as a
bit-identical golden-trace comparison — §11's strongest single regression
requirement, met.

No test, node, or ledger entry across Tasks 4–9 records a legitimate graph
being rejected by the new sort that the old two-pass model would have
accepted. The design's own prediction in §4 — "a genuine circular
dependency... today it compiles and one side silently reads stale data" — is
what the union-cycle test demonstrates is now caught instead, not a case of
the new sort being overzealous.

## 3. How did `(field, index)` addressing read at the call site?

**Readable, and the registration guard still catches a field reorder,
because it pins field ordinals — a per-type, per-field fact — while the
per-instance element count that `Vec` fields introduce is a separate axis the
guard was never asked to police.**

At real call sites, `(field, index)` addressing mostly disappears rather than
adding ceremony, because `PortView` offers a field-only convenience for the
overwhelmingly common non-variadic case and reserves the explicit `_at`
suffix for variadic access
(`crates/sway-graph/src/view.rs`: `read`/`read_at`, `write`/`write_at`,
`events`/`events_at`; `source` and `emit` always take an explicit index or
offset). A typical node reads as:

```rust
let translation: Vec3 = ports.read(Self::TRANSLATION);   // non-variadic: no index at all
let source = ports.source(Self::IN_GEO, 0);               // Product inlet: explicit index
```

(`crates/sway-nodes/src/scene.rs:76-80`, `crates/sway-nodes/src/mesh.rs:157`).
Where a field really is variadic, the index is doing real work and reads
naturally as "which element": `crates/sway-nodes/tests/traces.rs`'s
`build_event_fan_in` (above) and `crates/sway-app/src/demo_graph.rs`'s
`edge()` helper both take `(field, index)` positionally and every call site
that spawns an edge is a five- or six-argument function call naming named
`u16` constants (`Grid::OUT_GEO`, `Displace::IN_GEO`, `Group::CHILDREN`,
...), never a bare numeric literal for the field half — the ordinal guard is
exactly what makes that safe to do by hand rather than through a derive
macro, matching M2a's finding that a derive was not yet warranted.

The registration guard (`crates/sway-graph/src/registry.rs`'s
`check_ordinals`, line 174) still does its one job: it matches derived
`(name, ordinal)` pairs against `N::ORDINALS` and panics naming the field and
both ordinals on a mismatch, naming an undeclared field, or naming a stale
declaration that doesn't correspond to any field —
`a_wrong_ordinal_fails_registration_and_names_the_field` and
`a_missing_ordinal_declaration_fails_registration` (registry.rs, ~420-460)
cover both directions and both still pass. What changed is scope, not
strength: `ORDINALS` and the guard operate purely on field identity — which
named field sits at which ordinal, inlets then outlets — and have no opinion
about a `Vec` field's per-instance length. That number is resolved
separately, at compile time, by `inlet_lens_of` reading the actual `Inlets`
component (`registry.rs:332`) and by `compile.rs`'s layout pass consuming it
(`compile.rs:222-235`). The two mechanisms don't overlap: a field reorder is
still caught by the ordinal guard at registration (once, per type, at
startup); a wrong element count or an edge past a variadic field's length is
caught at compile time per instance (`an_edge_past_a_variadic_field_names_its_length`,
`two_edges_into_one_variadic_element_are_rejected`, `compile.rs:1038-1063`).
Nothing needed to be added to the guard itself for declared arity to work.

## 4. Did `Product<T>`-as-entity-reference remove the capability system cleanly?

**Cleanly — no parallel mechanism was reintroduced.** A `Product<Spatial>`
inlet is validated by exactly the same code path as an `f32` inlet.
`crates/sway-graph/src/compile.rs`'s edge-validation pass has one type check
for every edge, full stop (line 362): `if source_spec.slot_type !=
target_spec.slot_type { return Err(CompileError::TypeMismatch { ... }) }`,
where `slot_type` is a `TypeId` computed once per field at schema-derivation
time. `Product<Geometry>` and `Product<MaterialOf<StandardMaterial>>` are
just two more `TypeId`s to that check, indistinguishable in the code from
`f32` or `Vec3`. There is no second function, no separate slot-table lookup,
and no `Produces`-shaped capability enum anywhere in `compile.rs`.

What makes the check possible without `sway-graph` knowing what `Geometry`
or `MaterialOf<M>` are is `ReflectProduct`
(`crates/sway-graph/src/schema.rs:52-79`): a piece of `bevy_reflect` type
data, registered once per capability by that capability's own crate
(`register_product::<Geometry>(app)` in `sway-geo`, `register_product::<MaterialOf<StandardMaterial>>(app)`
in `sway-nodes`), carrying the capability's `TypeId` and a `ProductAccess` —
two plain fn pointers, `get: fn(&dyn PartialReflect) -> Option<Entity>` and
`set: fn(&mut dyn PartialReflect, Option<Entity>)` — that read and write a
`Product<T>`'s `source: Option<Entity>` through `dyn PartialReflect` without
the engine ever naming `T`. This is the direct architectural answer to the
M2b finding that `Slots` plus `Produces` needed a third member
(`produced_change_tick`) beyond §4's original two-member sketch: unifying
`Feeds`/`ChildOf`/param into one edge did not just relocate that need, it
folded it into the same registry mechanism that already handles ordinary
value types, so there is one type-data trait (`ReflectProduct`) doing what
used to need a whole parallel `Slot<T>`/`ReflectSlot`/`SlotField` module
(§7 of the design: "`Slot<T>`, `ReflectSlot`, `SlotField`, `derive_slots`,
`SlotView`, `SlotSource`, and `slots.rs` entirely" — all deleted).

The one thing that *is* special-cased, deliberately and narrowly, is
`Spatial` itself — covered in question 5 below — and it is special-cased in
the compiler's edge-validation and ordering passes, not in the type-check
step this question asked about. The type check that decides whether a
`Product<T>` edge is legal has no `if capability == Spatial` branch in it at
all; `Spatial`'s special behaviour is what the compiler does *after* an edge
of that type is already known to be legal.

## 5. What did `Spatial`'s three special behaviours cost, and did anything else want the same treatment?

**Contained exactly as the design predicted: one capability, three
behaviours, all three living in `compile.rs` and nowhere else, and no other
capability across five node crates and one compiler test file ever needed
any of the three.**

The three behaviours are each a few lines, all in `crates/sway-graph/src/compile.rs`:

1. **Single-consumer, not fan-out-free.** A `spatial_consumer: HashMap<usize, Entity>`
   keyed by source node index (line 295) rejects a second edge out of the
   same `Product<Spatial>` outlet with `CompileError::SpatialFanOut`
   (line 392-398) — the one outlet-side arity rule in the whole compiler,
   exactly as §3 of the design says.
2. **Excluded from the compiled order.** The topological sort's adjacency
   construction filters `.filter(|e| !e.spatial)` (line 438) before Kahn's
   algorithm runs, so a `Product<Spatial>` edge contributes no in-degree and
   no adjacency edge at all — it is invisible to the one sort, not sorted
   into a trivial position.
3. **Emits Bevy's `ChildOf`.** A separate, explicit pass after the sort
   (`compile.rs`'s "Pass 7: apply ChildOf", line 521) walks `parent_of` and
   inserts or removes `bevy_ecs::hierarchy::ChildOf` to match — the one place
   the compiler writes an ECS relationship component that isn't the arena.

Parenting acyclicity is checked separately from the topological sort
(`compile.rs`'s "Pass 4", line 412) for the reason the design gives:
`Spatial` edges are excluded from the one sort, so a parenting cycle would
sail through it undetected if there weren't a dedicated walk for exactly
that case (`a_parenting_cycle_is_rejected`, `compile.rs:786`).

Every other capability observed across Tasks 4-9 — `Geometry`
(`sway-geo`'s `Grid`/`Displace`, `sway-nodes`' `Mesh`), `MaterialOf<StandardMaterial>`
(`sway-nodes`' `StandardMaterialNode`/`MeshNode`), and the compiler's own
test-only `Blob` fixture (`crates/sway-graph/src/test_nodes.rs:20`, its
`Consumer` node used by the union-cycle test in question 2) — goes through
the ordinary
`Product<T>` path with none of the three behaviours: fan-out is legal (a
`Geometry` outlet can feed many consumers), the edge enters the topological
sort like any value edge, and nothing is emitted beyond the arena slot
holding the entity reference. No ledger entry, review round, or fix round
across Tasks 4-9 records a second capability wanting single-consumer
semantics, exclusion from ordering, or a synthesized ECS relationship. One
engine-known capability is the right shape for what this milestone actually
built; the honest caveat is that the node set exercised is still small
(eight signal nodes plus six scene/geometry nodes), and a future capability
genuinely needing Bevy-hierarchy-shaped fan-in (a second one-parent-style
relationship, say) would be the first real test of whether `Spatial`'s
three behaviours generalize to "one capability" being extensible to two, or
whether the compiler would need a small table rather than one
`TypeId::of::<Spatial>()` comparison sprinkled at three call sites.

## What M4 would otherwise rediscover

- **A fresh `PortArena` slot holds `()` until the first `graph_tick`.**
  `seed_outlets_of` (registry.rs:297) only runs on the first post-compile
  tick, gated by `CompiledGraph::outlets_seeded`. Any test or tool that
  reads or downcasts an arena slot after `compile()` but before at least one
  `graph_tick` call will find `()`, not a typed default — this bit the
  temporary benchmark test in question 1 directly (a `try_downcast_mut::<f32>()`
  panic) until a warm-up `graph_tick` call was added before locating the
  slot to poke.
- **`MidiCC`'s (and, by the same shape, any MIDI-driven node's) `tick`
  leaves its output slot untouched when no matching event is queued this
  tick** (`midi.rs`'s `tick`, no `else` branch on the CC-match `Option`) —
  useful for forcing a specific downstream node dirty in a test without a
  live MIDI feed, but a trap if a test assumes every node rewrites its own
  outlet every tick.
- **`graph_tick` called bare does not advance the world's change tick**
  (already an M2b finding, reconfirmed here) — still true and still a silent
  failure mode: forgetting `world.increment_change_tick()` between direct
  `graph_tick` calls that also mutate arena state turns a "dirty every tick"
  measurement into a "dirty once, then closed" one, with no error or panic
  to flag it.
- **The `sway-app` `editor` Cargo feature was temporary scaffolding, now
  removed.** Task 7 gated `sway-editor` behind an `editor` feature (default
  off) because `sway-editor` didn't compile against the unified model until
  Task 8; that gate, and the `#[cfg(feature = "editor")]`/`#[cfg(not(...))]`
  branches it required in `presenter.rs`, `shell.rs`, and the `--editor`
  panic path, are gone as of this task. `sway-editor` is now an
  unconditional dependency of `sway-app`; `cargo build -p sway-app` and
  `cargo test --workspace` both pass with the change, and `--editor` now
  runs the real `EditorPresenter` unconditionally rather than needing
  `--features editor`.
- **`cargo clippy -p sway-graph -p sway-geo -p sway-nodes -p sway-editor
  --all-targets -- -D warnings` had never actually been run to completion
  before this milestone's own verification step ran it.** It failed twice in
  succession, once per crate reached: `sway-graph`'s `compile.rs` (a
  `clippy::type_complexity` pair on `Vec<(usize, fn(&mut dyn
  PartialReflect))>`, from Task 4), then — once that was fixed and the
  command got past `sway-graph` for the first time ever — `sway-editor`'s
  `canvas.rs` (a `clippy::too_many_arguments` on a test-only fixture
  function, from Task 8+9). Both are fixed (Fix round 1, see the task-10
  report); the full four-crate gate is clean.

## Deferred minor findings

Carried forward from the ledger, reviewer-approved as non-blocking, still
open:

1. `SlotIdx` (`crates/sway-graph/src/ports.rs:123`) appears unused outside
   its own definition (Task 4).
2. A stale doc comment in `geometry.rs` (Task 5).
3. `node_fields()`, a test helper, is duplicated across five files in
   `sway-nodes` rather than shared (Task 6).
4. `capture_nodes`/`capture_edges` in `sway-editor`'s `snapshot.rs` each
   independently rebuild an identical `Entity -> NodePlan` `HashMap` (Task
   8+9).
5. `FieldKind::Events -> EdgeKind::Events` has no dedicated editor-level
   test (Task 8+9).
6. Resizing a `Group`'s sockets across a live snapshot is untested — a
   pre-existing gap mirroring the already-untested `label`-rename path
   (Task 8+9).

## What was not proven

- **The gate-closed allocation delta (624 ns → ~1.05 µs) was not
  attributed.** Three plausible causes are named in question 1 — a
  genuinely busier demo graph, new non-free `Product` gathers where `Feeds`/
  `ChildOf` used to be free, and ordinary noise — and none was isolated by a
  controlled experiment. Allocation counts were not taken; this is a
  wall-clock comparison only.
- **Whether any *other* crate in the workspace has its own latent
  clippy debt under this exact command was not checked.** The four-crate
  gate is now clean, but it took two rounds to get there because it had
  never previously been run to completion — `sway-graph` blocked it the
  first time, `sway-editor` the second. Nothing here rules out a third
  crate outside this specific four-crate list carrying similar debt;
  `cargo clippy --workspace` was already known red on `main` before this
  milestone (M2a's own finding) and this task did not attempt to fix that
  wider scope, only the exact four-crate command the brief names.
- **`Spatial` being "the right shape" for one engine-known capability rests
  on a small node set never having wanted a second one**, not on a positive
  argument that a second such capability couldn't arise. Eight signal nodes
  and six scene/geometry nodes is not evidence that a future
  Bevy-hierarchy-shaped relationship (a second one-parent capability, say)
  would still fit the current three-call-site special case rather than
  needing a small table.
- **The union-cycle rejection and the `event-fan-in` bit-identical trace are
  the only two behaviour-change regressions actually exercised.** §11 lists
  several other required tests (a `Product<Spatial>` edge not constraining
  order, a `Product<Spatial>` outlet fan-out rejection, a resized `Vec`
  inlet leaving other fields' addressing untouched) which Task 4's own
  step 9 table covers, but this report's question-2 answer speaks only to
  the two the design's §13 phrasing specifically asks about (the two-phase
  split and the union cycle), not to the full acceptance-criterion list.
- **Only one machine, one branch, and (for the benchmark) three runs each.**
  No cross-machine, cross-architecture, or long-session variance is
  reported, matching M2a's and M2b's own stated limits.
- **Live MIDI hardware and the editor's visual behaviour were not
  re-verified against a physical device or by eye as part of this task.**
  Task 10 is verification-and-documentation, not a fresh functional pass;
  it relies on Tasks 1-9's own review sign-offs (`progress.md`) for
  functional correctness and adds only the workspace-wide checks, the
  measurement above, and the spec/report writing.
