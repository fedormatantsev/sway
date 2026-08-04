# M3 transport and beat lock — findings

M3 delivered a 24-ppqn, windowed-regression transport; `Time<Transport>`;
freewheeling; beat-aware nodes; golden traces; an editor readout; and a
beat-locked demo graph. The evidence below is the committed golden traces and
their tests, plus the Task 3–11 reports. It distinguishes those controlled
results from live MIDI behaviour that was not measured.

## 1. Did windowed regression hold?

**Yes for the recorded traces.** `ClockEstimator` fits a least-squares line to
the most recent 48 pulse-index/timestamp pairs, locks after eight samples, and
refits on every accepted pulse. The 120→90 BPM golden trace reaches and remains
within ±1 BPM of 90 at tick 500: 140 ticks, or **1.167 s**, after its tick-360
tempo change. At 90 BPM, 48 pulses span about 1.33 s, so that is within one
window. The steady 120 BPM trace's worst deviation is **0.366 BPM** (119.634 at
tick 17), and it is 119.89905 BPM at tick 60. The estimator's direct jitter
test injects ±1 ms jitter across 96 pulses and requires residual error below
**1 BPM**; it passes.

The 48-pulse/two-beat window is the right length for the evidence collected:
it averages the stated jitter while giving a measured roughly-one-window tempo
settle. A shorter window or a PLL was not benchmarked. There is no evidence
that a PLL would improve this trade-off, and switching to one would replace a
gain-free, deterministic line fit with gain tuning and a new trace-baselining
exercise. No reversal is recorded.

One documentation correction for later work: `ClockEstimator`'s source comment
says this window settles “in under a second,” but 48 pulses at the trace's
post-change 90 BPM take about 1.33 s and the observed settle was 1.167 s.

## 2. Did the freewheel policy hold?

**Yes on the one-second recorded dropout.** In `transport-dropout`, beats move
from 4.0000267 at tick 240 to 5.9996037 at tick 360: **1.9996 beats** over
**1.000 s**, versus the expected two beats at 120 BPM. BPM remains frozen at
the final estimate, 119.974625, while pulses are absent. The trace resumes
near tick 380 and has no beat-position discontinuity.

The longest dropout tested is therefore **one second**; the trace establishes
freewheel drift of approximately **0.0004 beat** against the ideal two-beat
advance over that interval. It does not characterize longer-clock-loss drift.
The re-lock test also asserts that the position never runs backward and advances
less than one beat in the first 0.2 s after re-lock. The generation guard did
fire as designed: a long pulse gap resets `ClockEstimator`, increments its
generation, and `advance_transport` falls back to freewheel instead of
subtracting positions from fits with unrelated origins. The trace and
`the_clock_re_locks_after_a_dropout_without_jumping` both pass, so no visible
jump was produced by the tested re-lock path.

## 3. Was the min-filtered offset the right shape?

**The shape is appropriate, but a long-session claim was not demonstrated.**
`MidiClockOffset` takes the minimum of a bounded window of
`host_now - fixed_elapsed` samples, which matches the one-sided-noise premise:
fixed time can lag host time, but that does not make the true offset smaller.
It uses `OFFSET_WINDOW = 240`; the implementation report describes that as
about four seconds at 60 drains per second. `map_timestamp` preserves ordering
with `last_enqueued` and bounds look-ahead to 0.5 s.

Task 3 tests prove the window rolls forward under synthetic offset drift and
that a falling estimate cannot reorder the inbox. Task 11 did not add a
long-session measurement of MIDI response, so neither “held up” nor a
too-short/too-long verdict for 240 drains is supported. At this point the
window is **not shown to be the limiting factor**.

The hardware finding instead exposes a carried-forward limitation at a
different layer: when a source supplies no per-message hardware timestamps and
the frame rate remains below the 48 pulses/s rate of a 120-BPM MIDI clock,
multiple pulses collapse to a frame timestamp. The Task 11 duplicate guard
prevents the former divergence (the pre-fix reproduction reached 1026.89 BPM),
but such a source can now lock stably to the frame-rate-derived BPM (for
example, **30 fps → 75 BPM**, not 120). This is not a min-filter-window result,
and it remains unresolved.

## 4. What did the zero-inlet node break?

**Nothing in the engine, but it established an important schema case.**
`TransportTimeNode` is the first node whose `Inlets` struct has no fields.
The Task 6 compile-and-tick test passed without changes to `sway-graph`:
`derive_fields` produces an empty field list, `prefill_of` receives an empty
field slice, and `compile` sums zero inlet slots and leaves
`field_offsets`/`field_lens` empty. No code had to be taught an exception.

M4's RON schema must still represent this shape explicitly: absence of inlet
fields is a valid empty object, not an omitted node contract, and consumers
must not assume a first field or first slot exists.

## 5. `Events<Beat>` has no consumer

This remains the design question M3 opened rather than answered. `BeatTrigger`
correctly emits typed `Events<Beat>` and the golden trace verifies boundaries
and sub-tick offsets, but the demo deliberately leaves it unwired. `Envelope`
accepts `Events<NoteMsg>`; coercing beat pulses into it would make an
event-type-selector node by accident.

The next milestone that needs beat-driven behaviour must decide whether event
payloads want a common shape, whether `Envelope` should accept arbitrary event
types, or whether the missing primitive is a `Quantize` node that delays an
existing stream to beat boundaries. M3 supplies evidence for none of those
three API choices.

## 6. Mid-tick reposition quantization

`advance_transport` applies Start and Song Position Pointer repositions after
the tick advance. A Start arriving inside a tick therefore places musical zero
at the tick boundary, with a bounded error of **under 9 ms at 120 Hz**. The
transport test allows that one-tick quantization; the beat-trigger tests verify
mid-window beat-boundary offsets, not a sequencer display comparison.

No observation against a sequencer's own display was recorded. The current
bound is below a 120-Hz timestep and a second, sub-tick-aware advance path
would add control-flow complexity to correct an unobserved display difference.
It is reasonable to retain the quantization, but it remains carried forward
rather than proven imperceptible.

## What a later milestone would otherwise rediscover

- `Time<T>` is monotone: it advances by `Duration`, and rewind is not a valid
  transport operation. Start and Song Position Pointer move
  `Transport::origin_beats`; `beats_total()` remains monotone and `beats()` is
  elapsed minus that origin, clamped at zero.
- The first empty `Inlets` struct is already legal end-to-end. Do not add an
  artificial placeholder inlet when designing M4's serialized schema.
- The outer masonry `Split` uses
  `split_point_from_start(TRANSPORT_BAR_HEIGHT.px())` and is non-draggable.
  That makes the transport bar a fixed-height first pane, not a proportional
  split. The layout regression proves the viewport begins at or below the
  24-pixel bar.
- `bevy_reflect` did not reject the final `Transport`, `TransportState`,
  `Division`, or `Beat` shapes. The registration paths are nonetheless
  required: `GraphPlugin` registers `Transport` and `TransportState`;
  `BeatTrigger::register` registers `Division` and calls
  `register_events::<Beat>`. The actual compile surprise was
  `BeatTriggerState` ceasing to be a unit value when it gained private
  `prev_end`; fixtures must use `BeatTriggerState::default()`.
- `BeatTrigger` must carry the previous tick's exact `end` instead of
  reconstructing it as `end - delta`: independently rounded `Duration`→`f64`
  conversions otherwise double-count a boundary at a tick seam. A mid-play
  reposition leaves that stored boundary stale for one tick; it cannot flood
  (`MAX_PULSES_PER_TICK = 64`) but remains untested.
- Same-instant clock pulses must be ignored before index inference. The guard
  preserves the estimator generation and prevents a zero-elapsed sample from
  biasing both the fit and subsequent inferred indices; it does not recover
  individual timestamps that were never supplied.

## Measurement boundary

The transport suite uses **120 Hz**, and the estimator/transport advance path
now runs in that fixed-tick system. No new direct `graph_tick` timing or
per-tick estimator-cost measurement was taken with `--test-threads=1`; the
earlier M2b timing is not a second transport data point and is not used here to
select or revise the rate.
