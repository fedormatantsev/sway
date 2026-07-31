# M2a graph engine — findings

Consolidated answers to the four questions required by
`docs/superpowers/specs/2026-07-31-m2a-graph-engine-design.md` §12, plus the
implementation facts M2b would otherwise have to rediscover. The primary
source is `.superpowers/sdd/2026-07-31-m2a-graph-engine/progress.md`. Where an
early implementation claim and the reviewer-updated ledger disagree, this
report follows the ledger.

## 1. What resisted `Reflect`

**The generic event marker did not resist it.** `Event<T>` derives `Reflect`
successfully in `bevy_reflect` 0.19 with its
`PhantomData<fn() -> T>` field ignored. The fallback contemplated in the plan,
a non-generic `EventPort`, was not needed. The only wrinkle was import
placement: `ReflectDefault` must come from `bevy_reflect::prelude` rather than
the crate root for `#[reflect(Default)]` to resolve.

The real reflection failure was cloning, not deriving. Prefill originally used
`to_dynamic()`. That happened to work for opaque values such as `f32`, but a
reflected enum such as `Waveform` became a dynamic proxy and could no longer be
downcast to the concrete port type. Prefill now uses `reflect_clone()`, whose
contract is to preserve the concrete type. The same rule applies when the tick
runner clones continuous and event values across edges.

Two adjacent schema findings also belong with the reflection answer:

- `Remap` legitimately has an input and an output both named `value`.
  `check_ordinals` therefore has to consume declarations by the full
  `(name, ordinal)` pair. Name-only matching was insufficient.
- `Envelope` needs both `trigger` and `release_trigger`. The original node table
  had only the attack trigger; the second event input was added so note-off can
  enter release through the graph rather than through hidden controller logic.

These corrected decisions, plus the implementation corrections to arena
allocation, `PortView` bounds, port direction and cycle diagnostics, are
recorded in the design document's Revision line and relevant sections.

## 2. Was `Box<dyn PartialReflect>` adequate?

**Yes for M2a.** All eight signal nodes use the typed `PortView` API over the
erased arena, including enums, booleans, floats and reflected event payloads.
The representation did not force a hand-written schema or leak dynamic
downcasts into node implementations. At this scale, the simple arena was more
valuable than typed-column machinery.

That does not mean allocation-free. `graph_tick` currently builds a temporary
registry-function `Vec` each tick. More importantly, each `event_merges` entry
copies its source stream into a temporary `Vec<Occurrence>` because source and
destination live in the same `Vec<Vec<Occurrence>>`; every occurrence's
reflected payload is cloned before the temporary is extended into the
destination. Continuous edge gathers also replace a boxed reflected clone.
Those costs are known and accepted here, not optimized away by the arena split.

Typed columns keyed by `TypeId` become warranted if profiling on realistic M2b
graphs shows these allocations and downcasts materially consuming the tick
budget; if event fan-in or node cardinality makes reflected cloning dominant;
or if a hard real-time/no-allocation tick becomes a requirement. A desired tick
rate by itself is not evidence for the rewrite. `PortView` keeps that change
behind the engine boundary.

## 3. Did positional index consts hold?

**Yes, across all eight registered node types; the derive macro need not be
pulled forward yet.** Registration checks keep hand-written consts from
silently drifting when fields are reordered.

`Remap` exposed the important qualification: a port name is not globally unique
within a node because an input and output may share it. The first check matched
by name and could accept the wrong declaration. Matching and consuming the
exact `(name, ordinal)` fixed the collision without changing the positional
scheme. A derive macro remains useful cleanup if the node set grows enough that
maintaining consts becomes repetitive, but M2a did not produce evidence that it
is required for correctness.

## 4. Tick cost data point

On the Apple M4 used for this milestone, an optimized test binary ran the
existing `chain-math-remap` golden-trace graph (`LFO → Math → Remap`, three
nodes and two continuous edges) for 1,000 warm-up ticks and then 100,000 timed
ticks. Each timed iteration called `App::update()` with
`TimeUpdateStrategy::FixedTimesteps(1)`, so the measurement includes Bevy
schedule overhead as well as one graph tick.

The timed section took **222.635542 ms: 2.226 µs/tick, or about 449,165
ticks/second**. It was measured with:

`cargo test -p sway-nodes --test traces --release measure_chain_math_remap_ticks -- --ignored --nocapture`

The ignored test was temporary one-shot instrumentation and was removed after
the run; no benchmark code ships with this report.

This is a floor-scale data point, **not a tick-rate recommendation**. It has no
geometry cooks, live MIDI callback, renderer, high event fan-in, or M2b scene
nodes. The provisional 120 Hz choice remains open.

## What M2b would otherwise rediscover

- **CoreMIDI and Bevy clocks have different epochs.** CoreMIDI timestamps are
  mach-absolute seconds; `Time<Fixed>::elapsed` is app-relative. Feeding the
  former directly into `MidiInbox` leaves live events apparently far in the
  future. The bridge samples an epoch offset and converts before enqueueing.
  Its current throwaway implementation samples host time at first drain, not
  the first event, and does not correct long-session clock drift.
- **Inbox draining cannot assume timestamp-ordered insertion.** A future event
  at the front must not block an eligible late arrival behind it. The fixed
  implementation drains eligible entries with `retain`, preserving future
  entries.
- **`PortView` needs explicit per-node lengths as well as bases.** Bases alone
  allow an out-of-range ordinal to reach the next node's slots. Continuous
  connected masks also belong in the scoped view.
- **Port direction is a compile-time property.** Kind and type checks are not
  enough: sources must be outputs and targets must be inputs.
  `WrongPortDirection` covers this independently of continuous/event kind.
- **Kahn's remainder is not exactly the cycle.** Nodes downstream of a cycle can
  also retain nonzero in-degree, so cycle diagnostics name the blocked set
  rather than claiming every listed node lies in the cycle.
- **Use `reflect_clone()`, not `to_dynamic()`, for arena values that must later
  downcast to their declared concrete type.** Enums made this observable.
- **Ordinal identity is `(name, ordinal)`.** Name-only checks fail for nodes
  such as `Remap` with dual-named input/output ports.
- **Envelope release is graph input.** Preserve `release_trigger`; a single
  attack trigger cannot represent note-off-driven release.
- **Change-tick prefill is persistent state.** Comparing stored component ticks
  survives skipped cadence and recompilation; a one-run `Changed<T>` filter
  does not.
- **Event merge ordering is established twice.** Compilation orders sources by
  compiled rank; the tick performs a stable offset sort so equal offsets retain
  source rank.
- **Clippy evidence is scoped.** `cargo clippy --workspace` was already red on
  `main`; M2a's accepted gate is `cargo clippy -p sway-graph -p sway-nodes`.
  Do not attribute unrelated workspace debt to these crates.
- **The event marker itself is viable.** Keep generic `Event<T>` and import
  `ReflectDefault` from the reflect prelude; no parallel schema marker is
  needed.

## Deferred minor findings

These were reviewer-approved as non-blocking and remain open:

1. `PortArena::new`/`resize` intentionally fill fresh continuous slots with
   `Box::new(())`, so an unwritten read is visibly wrong rather than plausibly
   `0.0`; that rationale is not yet in the code comment.
2. `register_event_port` has no focused unit test, though real node
   registration exercises it.
3. Two compile/view stub comments use inconsistent task-number wording.
4. Event-kind `WrongPortDirection` has no dedicated regression; the check is
   structurally kind-independent and the continuous case is tested.
5. Equal-offset event fan-in lacks a direct tie-break regression.
6. `PortView` bounds regressions directly test write/emit; read/events share
   the helpers but are not asserted separately.
7. MIDI tests do not separately cover every explicit case: `0x80` note-off,
   emitted offsets, CC rejection, intermediate normalization and
   last-match-wins.
8. Golden-trace metadata-only diffs report tick 0 / `<metadata>` rather than a
   more specific changed tick or port.
9. The throwaway MIDI epoch bridge samples at first drain and leaves
   long-session mach-versus-fixed drift unaddressed.

## What was not proven

- **Live MIDI was not visually confirmed with CoreMIDI hardware.** The app
  launches and the epoch correction was reviewed and regression-tested, but no
  device was available in the agent environment to prove a physical note
  visibly drives the cube.
- **The fixed tick rate was not chosen.** The number above measures one small
  signal graph on one machine. It says nothing about the cost of M2b geometry
  cooks, larger graphs, rendering contention, or worst-case event bursts.
- **Typed columns were not disproved as a future need.** M2a established that
  erased reflected storage is functionally adequate, not that its allocation
  profile will remain adequate at production cardinality.
- **Event fan-in stability across recompiles was not specified or proven.**
  Ordering is deterministic for one compiled graph; M4's reload semantics must
  decide whether recompilation must preserve an earlier source order.
- **Every MIDI semantic corner was not independently tested.** The missing
  focused cases are listed in deferred finding 7; the accepted integration and
  golden tests do not turn those omissions into proofs.
- **All `PortView` access directions were not separately bounds-tested.**
  Shared helpers make the implementation structurally consistent, but only
  write and emit have direct out-of-range regressions.
- **Equal-offset fan-in's source-rank tie-break has no direct test.** Stable sort
  plus compiler ordering implements it, while current traces discriminate only
  distinct offsets.
- **Long-running MIDI clock alignment was not measured.** The bridge fixes the
  epoch mismatch that trapped events in the future; it does not characterize or
  compensate drift over a long session.
