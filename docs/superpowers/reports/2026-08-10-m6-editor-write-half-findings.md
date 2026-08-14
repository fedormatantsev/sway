# M6 — the editor write half: findings

**Date:** 2026-08-14
**Verdict:** GO — exit criterion met
**Plan:** [`2026-08-10-m6-editor-write-half.md`](../plans/2026-08-10-m6-editor-write-half.md)
**Spec:** [`2026-08-10-m6-editor-write-half-design.md`](../specs/2026-08-10-m6-editor-write-half-design.md)
**Roadmap:** M6 in [`2026-08-09-mvp-roadmap-design.md`](../specs/2026-08-09-mvp-roadmap-design.md)

## Question

Can a node be created, wired, edited, saved and reopened without leaving the
editor?

## Answer

Yes. `cargo test --workspace` on HEAD `836efb0`: **346 passed, 0 failed, 2
ignored** (`an_async_file_dialog_future_polls_pending_without_an_executor` in
`sway-app` — intentionally ignored, opens a real file dialog, run by hand
when adding/bumping `rfd`; and the pre-existing `field_wire!` doctest in
`sway-nodes`, unrelated to M6, ignored since M5). By eye: the human partner
walked all eight exit-criterion steps live, in one session, on HEAD `836efb0`,
without touching RON — right-click-create an `Lfo`, create a `Vec3`, drag
`Lfo`→`Vec3.y`, drag `Vec3`→a cube's `translation`, edit the `Lfo`'s `beats`
in the inspector and watch the cube's motion change, Save As to a new path,
quit, relaunch, Open that path, confirm the graph, the wiring and the edited
value all came back. Verbatim report: "all good" — every step worked,
including the full save/quit/relaunch/reopen round trip. (Automated GUI
click/drag coordination is unreliable in this sandbox — established at Tasks
8, 11 and 13 — so this walkthrough was run by the human partner directly,
consistent with the rest of this plan, not by an automated agent.)

## What was built

Sixteen tasks in six phases, one commit per task (plus named fix-round and
refactor commits where a phase or individual review sent a task back):

- `8ac8fdc` — Task 1: `sway-document` extracted from `sway-graph::project`;
  the component registry stays in `sway-graph`. Satisfies architecture §8's
  refactoring policy (M6 rewrites the save path regardless). No behavioural
  surprises; a stale 269-test baseline in the plan was corrected to the true
  271 by direct re-measurement.
- `d9ea5d0` — Task 2: the editor command channel (`EditorCommand` and its
  plumbing).
- `7fd4fa8` — Task 3: `Create` and `Delete` commands. The plan's one
  documented open conditional resolved cleanly: a characterization test
  showed Bevy already clears a wire from its consumer when the producer
  despawns, so `Delete` needed no explicit wire-cleanup loop.
- `bc1aa1a` — Task 4: `SetField` writes one field through reflection. Needed
  the brief's warned-of Step 3 restructure (read immutably before writing) —
  an implementer scratch test proved `reflect_mut` + deref alone marks
  `Changed` even with no actual write. The brief's literal `bevy_reflect`
  calls didn't match the pinned 0.19.0 (`DynamicEnum` lives at
  `bevy_reflect::enums::`, variant existence comes from `EnumInfo` not the
  runtime `Enum` trait, no `EntityWorldMut::reborrow`) and needed correction
  — the same "verify against the pinned checkout, don't guess an API" lesson
  M5's report called out. The `FieldValue::Enum` branch shipped with zero
  test coverage from its own brief; Phase 2 review judged it correct by
  inspection and deferred writing a test for it.
- `074cf3c` — Task 5: `Connect` and `Disconnect` commands. Matched the
  brief's `WireEntry` assumption exactly, no drift.
- `bdc6285` — Task 6: the canvas draws `EditorPos` entities; sockets gain
  identity (ordinals, `InletView`/`OutletView`). This also resolves M5's
  open "graph-canvas leaf-visibility gap" — `capture_nodes` now walks every
  `EditorPos` entity instead of `GraphOrder`, so a camera or light with no
  wires is no longer structurally invisible on the canvas. Phase 2 review's
  one Important finding (duplicated
  `ComponentDocRegistry`→`AppTypeRegistry`→`ReflectComponent` lookup between
  `Create` and `SetField`) was fixed in `c8881f5`, a generic
  `reflect_data_for<T>` helper generalized beyond the reviewer's literal
  suggestion.
- `6292c4f` — Task 7: `sway-editor` services `RenderRootSignal` instead of
  dropping it (palette/inspector/canvas layering plumbing). Run visually and
  screenshotted in this environment; no regression.
- `64e32a4` — Task 8: editable inspector fields — M6's first end-to-end
  write. Live-verified: editing `lfoA.beats` 8.0→2.0 visibly sped up the
  driven cube's bob. Surfaced and fixed a real bug: field text overlapped
  its input box's border, root-caused to `ROW_HEIGHT` (18.0) leaving no room
  for masonry's default theme padding+border once the layout pass subtracted
  them; raised to 32.0, the analytical minimum. A deferred minor from this
  task: selecting a tree row whose entity has no canvas node (e.g. `lfoA`)
  flickers the inspector for one frame then reverts, because
  `EditorUi::sync_selection` reconciles tree selection back to the canvas's
  selection every frame — pre-existing, untouched by this task's diff.
- `4dd1c42` — Task 9: claims `EditorPos` entities the editor created, so a
  palette-spawned node round-trips through save/reload like anything else.
- `2d3b69a` — Task 10: open and save by path, with the self-triggered hot
  reload suppressed.
- `a089e3c` — Task 11: real `rfd` file dialogs and toolbar Open/Save/Save As
  buttons. The by-eye round trip was deliberately deferred (same
  can't-automate-native-dialogs precedent as Task 8) and run live by the
  controller with the human partner afterward: launched the editor, edited
  `lfoA.beats`, Save As to a real path, quit, relaunched, Open'd it back —
  the edited value loaded; edited again, plain Save — no dialog, no visible
  reload glitch. Confirmed "all good". Separately: the committed
  `rfd_pollable` ignored test panics under `cargo test` (no running
  `NSApplication`, wrong thread) — a throwaway winit-event-loop probe
  confirmed the brief's actual load-bearing assumption (pollable-without-an-
  executor from inside a real `RedrawRequested` callback) holds where it
  matters, rather than falling back to a thread+channel alternative on the
  strength of an environment artifact.
- `9b033db` — Task 12: the component palette layer (standalone widget, not
  yet wired to the canvas). Found and fixed a bug in the brief's own test
  fixture: a `filter("ma")` assertion collided with substring matches in
  `"PbrMaterial"`/`"Remap"` (`"pbr-MA-terial"`, `"re-MA-p"`); the fixture was
  swapped for names without the collision. Judged a plan-text defect
  (illustrative data), not a plan-behaviour conflict.
- `0ac2c9a` (fix rounds `6b7cb0c`, `e12b84a`) — Task 13: create and delete
  nodes from the canvas, wiring the palette in. Individual review found 2
  Critical defects, both in code the brief specified verbatim: palette-pick
  never reached `GraphCanvas` (`ctx.create_layer` makes `Palette` a
  `LayerStack` *sibling* of `GraphCanvas`, not a descendant, so the pick
  action's bubble dead-ends and is discarded), and Delete/Backspace never
  fired (nothing called `ctx.request_focus()` anywhere, so masonry's
  text-event pass never targeted `GraphCanvas`). Both were masked by tests
  that called bypass seams instead of driving the real event path — green
  suite, dead feature in the real app. This was a genuine plan-text-vs-
  plan-intent conflict (the brief's own by-eye step requires the feature to
  work; its own code doesn't produce that), escalated to the human partner.
  Ruling (2026-08-14): fix it now, diverging from the brief's literal code —
  adopt masonry's own `SelectorMenu` `with_creator`/`mutate_later` pattern
  for the palette pick, add `ctx.request_focus()` for the delete key. Fix
  round 2 closed two gaps the phase review found in round 1's own fix:
  `palette_layer` going stale on click-outside dismiss (no `mutate_later` on
  dismiss, unlike masonry's own `SelectorMenu`), and the filter `TextInput`
  never receiving focus on open — fixed via `RenderRoot::focus_on` after
  tracing that masonry's pinned source makes both of the phase review's
  suggested fixes dead at this revision.
- `fee2e31` (fix round `2098064`) — Task 14: socket hit-testing and the
  rubber-band edge drag. Kept `ctx.request_focus()` in `NodeBox`'s `Down`
  handler despite the brief's abbreviated snippet omitting it — dropping it
  would have silently regressed Task 13's Delete-after-click flow, with
  nothing in this task's own tests to catch it. Added one real end-to-end
  press/release test beyond the brief's bypass-seam-only step, which on
  first run caught a genuine (pre-existing, out-of-scope) boundary defect:
  `outlet_socket_local`'s x sits exactly on `NodeBox`'s hit-test rect's
  exclusive edge, so a press at that exact pixel misses the widget —
  negligible real-world exposure, correctly left flagged rather than
  "fixed" since it's the brief's own explicit geometry. Individual review
  found 1 Important: `NodeBox`'s `PointerEvent::Cancel` during a socket drag
  never notified `GraphCanvas`, leaving the drag state stuck and the rubber
  band painted indefinitely — a genuine in-scope gap (the brief was silent
  on `Cancel` for this new gesture), fixed in round 1 with a dedicated
  `ConnectCanceled` action kept separate from `ConnectReleased` so Task 15
  could never misread a cancellation as a landing point.
- `836efb0` — Task 15: drag-to-connect with registry-driven legality.
  Command dispatch verified gated on real state transitions only
  (`Disconnect` only when `inlet.connected`, `Connect` only after
  `accepts_from` passes). Self-review proactively added a real
  press-drag-release-on-a-legal-inlet test to close a real-dispatch coverage
  gap the brief's own tests left open. One flaky, unrelated `sway-midi` test
  was investigated and ruled out (reproduces against unmodified HEAD too).
  Individually reviewed clean: 0 Critical, 0 Important.
- Task 16 (this commit): the exit criterion walked live, the two documents
  M6-5 invalidates amended, this report.

Against the exit criterion — "a node is created, wired, edited, saved and
reopened without leaving the editor" — every capability it names now exists
and was exercised live in one unbroken session with no RON editing.

## Deviations from the spec, recorded in their own commits

- **M6-8's `FileCommand`** became `FileRequest` and carries no path (Task
  11): a path-carrying variant is unbuildable by the widget that emits it —
  only `sway-app` owns `rfd`.
- **M6-6's socket hit-testing** is split between press and release (Task
  14): the spec implies the canvas resolves sockets, but masonry hit-tests
  children before parents and `NodeBox` marks every primary `Down` as
  handled, so the *press* is detected in `NodeBox` and only the *release* is
  resolved canvas-side. The behaviour matches the spec; the location is what
  masonry's dispatch allows.
- **M6-5 itself is a deviation from the roadmap**, not from this plan: roadmap
  D2 and architecture §7 said driven fields would be read-only in the editor;
  M6 does not implement that rule at all (see below).

## Surprises

- **A recurring class of masonry integration bug: layer/focus routing.**
  Tasks 13 and 14 each independently hit a case where the brief's literal
  code compiled, passed its own (bypass-seam) tests, and simply didn't work
  in the real app — a `create_layer`d widget's action bubbling dead-ending
  at `LayerStack`, and a missing `request_focus()` leaving text/key events
  undelivered. Both were only caught because an implementer or reviewer
  insisted on a real-dispatch test or a live run rather than trusting the
  green suite. This is now a two-for-two pattern in this plan, not a
  one-off — worth treating as a known risk class for any future masonry
  widget wiring, not just something already fixed.
- **`bevy_reflect` 0.19.0 API drift (Task 4)** repeated M5's lesson from a
  different angle: M5 needed to verify assumptions before implementing;
  here the brief's own snippets, written against an assumed API shape,
  didn't match the pinned version and needed correction during
  implementation. The fix was narrow and the review confirmed it
  semantically correct against the vendored source, but it's a reminder that
  "the plan was verified" and "the brief's code compiles as written" are not
  the same claim.
- **Task 13's escalation is this plan's only human-decision point** on
  diverging from brief-literal code; every other defect found (Task 4's API
  drift, Task 14's `Cancel` gap, Phase 2's duplicated lookup) was judged an
  in-scope implementation gap or plan-text-only defect and fixed without
  escalation, per the protocol this plan pre-agreed to (see plan
  Self-Review).

## What M6-5 changes in the documents

Roadmap D2 ("driven fields are read-only in the editor") and architecture
§7's read-only passage are both inaccurate as of M6 and have been amended
(this task, Step 3): every field is editable, wire-driven or not; editing a
driven field holds only until the next tick, when the wire overwrites it
again; a save still bakes in the instantaneous driven value, which is
harmless because the first tick after load overwrites it and nothing is
built against the file staying stable. Architecture §10's "Restore authored
value on disconnect" entry, which used to point at the read-only rule for
its own justification, is restated: there is no authored-value shadow to
restore from because a disconnected field simply keeps whatever the wire
last wrote — that was always true independent of the read-only rule, so the
"Out of MVP" line's status doesn't change, only its justification does.
`2026-08-09-mvp-roadmap-design.md`'s D2 is marked superseded by M6-5 and kept
for the historical record rather than deleted. `2026-07-25-sway-design.md`'s
M6 line and its own "Restore authored value on disconnect" bullet are
updated to match.

## What M7 inherits

- **The driven-axis question is now open, not decided.** Roadmap and
  `sway-design.md`'s M7 line still describe a gizmo with "driven axes
  inert" — that was going to reuse D2's read-only detection machinery. M6-5
  states this explicitly: "the gizmo was to refuse driven axes under the
  same rule. It now has no detection machinery to build on and must decide
  for itself." M7 has to either build its own detection (the exact cost M6-5
  judged not worth it for the inspector) or drop the "driven axes render
  inert" idea entirely and let the gizmo write through like the inspector
  does. This task's doc amendments deliberately did not touch M7's roadmap
  line — that's a decision for M7's own design pass, not something to
  prejudge here.
- **The masonry layer/focus routing pattern.** M7's pointer/key forwarding
  from the shell into Bevy over the viewport rect is exactly this kind of
  masonry dispatch-shape work. Given Tasks 13 and 14 each found a real,
  test-suite-invisible defect in this area, M7 should budget time to trace
  the real event path (as Task 13's fix and Task 14's `Cancel` fix did)
  rather than trust that a brief's literal snippet routes correctly.
- **The graph-canvas leaf-visibility gap is closed**, not inherited: M6-4
  (Task 6) switched `capture_nodes` to walk every `EditorPos` entity instead
  of `GraphOrder`, so M7's editor camera and any gizmo-adjacent nodes will
  already show up on the canvas without needing to be wired to anything
  first.

## What is not answered

- **The disconnect gesture's own press side has no real-dispatch test**
  (Phase 6 review, deferred minor): `NodeBox::socket_at_local`'s
  inlet-ordinal loop is only exercised via `socket_pressed_for_test`, not a
  real `Down` event, even though the release/canvas side
  (`each_inlet_socket_reports_its_own_ordinal`) does have real-dispatch
  coverage. The math is shared with and already proven correct at the
  canvas layer, so this is a coverage gap, not a known defect — but it's the
  one drag-to-connect path this plan didn't independently verify end to end.
- **The pre-existing tree-selection flicker is still open** (deferred at
  Task 8): selecting a tree row whose entity has no canvas node (e.g. an
  `Lfo` with no wires yet) flickers the inspector for one frame then
  reverts, because `EditorUi::sync_selection` reconciles tree selection back
  to the canvas's selection every frame. Predates M6, untouched by any task
  in this plan, does not block anything this milestone needed — but M7's
  "selection joins the tree↔canvas sync that already works" line means M7
  inherits this too and should decide whether to fix it.
- **No handling or test for a node deleted mid-drag** (Phase 6 review):
  paint's rubber-band draw is already guarded and Task 14's `Cancel` fix
  likely self-heals the drag state, but nothing exercises this specific
  interleaving.
- **`FieldValue::Enum`'s reflect logic (Task 4) has zero test coverage** from
  its own brief; Phase 2 review judged it correct by inspection against the
  vendored `bevy_reflect` source, not by a passing test.
- **`SOCKET_RADIUS * 2.5`'s duplication** — across `canvas.rs`/`node_box.rs`
  (accepted as intentional to avoid a circular module dependency) and again
  as a second bare literal within `socket_at_local` itself — has no inline
  comment marking it as a deliberate drift risk.
- **Cross-phase file growth** — `canvas.rs` (1923 lines), `snapshot.rs` (812
  lines), `inspector.rs` (530 lines) all grew substantially over this plan's
  six phases and were tracked but not restructured at every phase gate; a
  maintainability note for whoever next touches these files, not a defect.
- Everything already out of MVP scope stays out: variadic inlets
  (`Merge`/`Sum`), geometry operators and the geometry cook path, GPU-
  resident geometry operators.
