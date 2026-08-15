# M7 — Viewport Interaction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Bevy viewport live — pointer and key input reach the world, an editor camera orbits, clicking a mesh selects it everywhere, and a transform gizmo edits it — so the scene is composed by dragging rather than by typing numbers.

**Architecture:** The masonry `Viewport` widget converts pointer/key events into plain `ViewportInput` data and sends it on a crossbeam channel, exactly as `GraphCanvas` already sends `EditorCommand`. `sway-runtime` drains that channel into a per-frame `ViewportEvents` buffer that three consumers read: an editor camera's orbit/pan/dolly, a `MeshRayCast` picker, and two gizmo systems. Selection stops living in widgets and becomes a `Selection` resource in the world that the snapshot reports. The gizmo is Bevy 0.19's own `transform_gizmo` — its renderer, state types and geometry helpers are all reused; only its two window-coupled input systems are replaced.

**Tech Stack:** Rust 2024, Bevy `=0.19.0` (default features, so `picking` and `bevy_gizmos_render` are already on), masonry pinned to git rev `c5950bc`, `crossbeam-channel`, `ui-events` 0.3 / `keyboard-types` 0.8.

**Spec:** [`docs/superpowers/specs/2026-08-15-m7-viewport-interaction-design.md`](../specs/2026-08-15-m7-viewport-interaction-design.md)

**Also read before starting:**
- [`docs/architecture.md`](../../architecture.md) — §5 (editor integration), §7 (graph state), §8 (crate layout).
- [`2026-08-10-m6-editor-write-half-findings.md`](../reports/2026-08-10-m6-editor-write-half-findings.md) — especially "What M7 inherits". The masonry layer/focus routing failure class described there is the single most likely way a task in this plan ships green tests and a dead feature.

## Global Constraints

- **Bevy is pinned at `=0.19.0`; masonry at git rev `c5950bcb03d4f3d187a20d1159f6aa276fd056bf`.** Do not bump either. Do not add a dependency to any workspace `Cargo.toml` without it being named by a task below.
- **Verify APIs against the pinned checkout, never from memory.** Both M5 and M6 lost time to guessed signatures. Vendored sources are at `~/.cargo/registry/src/index.crates.io-*/bevy_*-0.19.0/` and `~/.cargo/git/checkouts/xilem-*/c5950bc/`.
- **`sway-editor` must not depend on `bevy` (the facade), `bevy_render`, `wgpu`, `vello`, or `imaging_vello`.** This is the M1b invariant. It may use `bevy_ecs`, `bevy_math`, `bevy_reflect`, `bevy_transform`, `bevy_time`, `sway-graph`.
- **`sway-graph` must not depend on `bevy_render`, MIDI types, masonry, or the document format.**
- **Never write an equal value** (architecture §7): compare before writing any component a wire could also write.
- **Panics are startup-only.** A bad ray, a missing camera, an empty selection: return, log if useful, never panic.
- **One commit per task**, message in the imperative, ending with the `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>` trailer.
- **Test suite must stay green.** Two tests are ignored on purpose and stay ignored: `an_async_file_dialog_future_polls_pending_without_an_executor` (`sway-app`) and the `field_wire!` doctest (`sway-nodes`).
- **Real dispatch, not bypass seams.** Any widget behaviour a user performs with a pointer or a key must have at least one test that drives `harness.process_pointer_event` / `harness.process_text_event`, not just a `_for_test` helper.

## Baseline

Before Task 1, measure and record the current numbers — do not trust this document's:

```bash
cargo test --workspace 2>&1 | tail -20
```

At the time of writing the expected shape is ~346 passed, 0 failed, 2 ignored. If it differs, record the real number in the Task 1 commit message and use that as the baseline for every later "no regressions" check.

## File Structure

**New files**

| File | Responsibility |
|---|---|
| `crates/sway-graph/src/viewport_input.rs` | `ViewportInput` and its companion plain-data types; `ViewportInputRx`; `normalize_viewport_pos`. Editor↔world plumbing, no masonry, no render. |
| `crates/sway-editor/src/viewport.rs` | The `Viewport` widget: keeps the `External` paint-layer behaviour, converts masonry events to `ViewportInput`, sends them. |
| `crates/sway-runtime/src/viewport/mod.rs` | `EditorViewportPlugin`, `ViewportEvents`, the channel drain, system ordering. |
| `crates/sway-runtime/src/viewport/camera.rs` | `EditorCamera`, `orbit_transform`, navigation, `ViewportCamera` and the active-camera system. |
| `crates/sway-runtime/src/viewport/pick.rs` | Ray construction and the `MeshRayCast` click-to-select system. |
| `crates/sway-runtime/src/viewport/gizmo.rs` | `viewport_gizmo_hover`, `viewport_gizmo_drag`, focus/camera markers, mode keys. |

**Deleted**

| File | Why |
|---|---|
| `crates/sway-editor/src/external.rs` | `ViewportPlaceholder` is replaced by `Viewport`; keeping an inert twin would invite wiring the wrong one. |

**Modified**

| File | Change |
|---|---|
| `crates/sway-graph/src/lib.rs` | Export the viewport-input module and `Selection`. |
| `crates/sway-graph/src/command.rs` | `EditorCommand::Select`, and its arm in `apply_editor_command`. |
| `crates/sway-graph/src/ctx.rs` | `Selection` resource, beside `EditorPos`. |
| `crates/sway-editor/src/lib.rs` | `Viewport` in `graph_root`; senders threaded; `sync_selection` deleted; selection pushed from the snapshot. |
| `crates/sway-editor/src/snapshot.rs` | `WorldSnapshot::selection`; `capture` fills selection and inspector; gizmo meshes filtered out of `capture_tree`. |
| `crates/sway-editor/src/scene_tree.rs` | Takes a `Sender<EditorCommand>`, sends `Select`, stops owning selection. |
| `crates/sway-editor/src/canvas.rs` | Sends `Select`, stops owning selection. |
| `crates/sway-editor/src/transport_bar.rs` | A camera-toggle button and `ViewRequest`. |
| `crates/sway-app/src/presenter.rs` | Constructs `EditorUi` with both senders; simplified `apply_snapshot`. |
| `crates/sway-app/src/shell.rs` | `ShellConfig` carries the viewport-input sender; services `ViewRequest`. |
| `crates/sway-app/src/main.rs` | Second channel; `EditorViewportPlugin` under `--editor`. |
| `crates/sway-runtime/src/lib.rs` | `pub mod viewport;` and re-exports. |
| `crates/sway-runtime/Cargo.toml` | Adds `sway-graph`. |

---

## Phase 1 — The input path

### Task 1: `ViewportInput` and the channel

**Files:**
- Create: `crates/sway-graph/src/viewport_input.rs`
- Modify: `crates/sway-graph/src/lib.rs`
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces: `ViewportInput`, `ViewportButton`, `ViewportModifiers`, `ViewportKey`, `ViewportInputRx`, `normalize_viewport_pos(local: Vec2, size: Vec2) -> Vec2`.

- [ ] **Step 1: Write the failing test**

Create `crates/sway-graph/src/viewport_input.rs` with only the test module and a stub:

```rust
//! Viewport input, editor to world. Spec M7-1.

use bevy_ecs::resource::Resource;
use bevy_math::Vec2;
use crossbeam_channel::Receiver;

pub fn normalize_viewport_pos(_local: Vec2, _size: Vec2) -> Vec2 {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_normalizes_against_the_viewport_rect() {
        let size = Vec2::new(800.0, 400.0);
        assert_eq!(normalize_viewport_pos(Vec2::ZERO, size), Vec2::ZERO);
        assert_eq!(normalize_viewport_pos(size, size), Vec2::ONE);
        assert_eq!(
            normalize_viewport_pos(Vec2::new(400.0, 100.0), size),
            Vec2::new(0.5, 0.25),
        );
    }

    #[test]
    fn a_drag_outside_the_rect_is_not_clamped() {
        // `capture_pointer` keeps delivering moves past the edge, and orbit
        // reads deltas from them. Clamping here would stall the gesture at
        // the border.
        let size = Vec2::new(100.0, 100.0);
        assert_eq!(
            normalize_viewport_pos(Vec2::new(-50.0, 150.0), size),
            Vec2::new(-0.5, 1.5),
        );
    }

    #[test]
    fn a_zero_sized_viewport_yields_zero_rather_than_nan() {
        // A minimized window delivers (0, 0) here; M6 Task 4 hit the same
        // hazard in the shell. NaN would propagate into every ray this
        // milestone builds.
        let out = normalize_viewport_pos(Vec2::new(10.0, 10.0), Vec2::ZERO);
        assert!(out.is_finite(), "got {out:?}");
        assert_eq!(out, Vec2::ZERO);
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p sway-graph viewport_input`
Expected: FAIL — `not implemented` panic from `unimplemented!()`.

- [ ] **Step 3: Write the implementation**

Replace the stub, and add the data types, in the same file:

```rust
/// Which pointer button an event carries. `sway-graph` cannot name masonry's
/// `PointerButton`, and the world side has no business knowing masonry
/// exists, so the widget translates at the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportButton {
    Primary,
    Secondary,
}

/// The modifier keys held when an event was produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportModifiers {
    pub alt: bool,
    pub shift: bool,
    pub control: bool,
    pub meta: bool,
}

/// The only keys the viewport consumes. Everything else bubbles past it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportKey {
    Translate,
    Rotate,
    Scale,
}

/// One input event over the Bevy viewport.
///
/// Every `pos` is **normalized to the viewport rect**: `[0,1]²` with the
/// origin at the top-left, unclamped. Not logical window pixels and not
/// physical ones — see spec M7-1: `Camera::viewport_to_ndc` divides by
/// `logical_viewport_rect()`, which for a `RenderTarget::TextureView` is the
/// texture's own (physical) size, while masonry's coordinates are logical.
/// Normalizing here makes the world side `pos * camera.logical_viewport_size()`
/// with no scale factor anywhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportInput {
    Down {
        button: ViewportButton,
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    Move {
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    Up {
        button: ViewportButton,
        pos: Vec2,
    },
    /// The pointer capture was lost. Any drag in progress must be abandoned;
    /// M6 Task 14 shipped a stuck rubber-band by leaving this case out.
    Cancel,
    /// `delta` is in logical pixels, already reduced from masonry's
    /// line/page/pixel policy by the widget. Positive `y` dollies in.
    Scroll {
        delta: Vec2,
        pos: Vec2,
        modifiers: ViewportModifiers,
    },
    /// A trackpad pinch magnification delta.
    Pinch {
        delta: f32,
    },
    Key {
        key: ViewportKey,
    },
}

/// The receiving half, held by the world. Present only in an editor build.
#[derive(Resource)]
pub struct ViewportInputRx(pub Receiver<ViewportInput>);

/// Maps a widget-local position (logical pixels) into `[0,1]²` across the
/// viewport rect. Deliberately unclamped, and zero-safe.
pub fn normalize_viewport_pos(local: Vec2, size: Vec2) -> Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return Vec2::ZERO;
    }
    local / size
}
```

- [ ] **Step 4: Export it**

In `crates/sway-graph/src/lib.rs`, add the module beside `pub mod command;`:

```rust
pub mod viewport_input;
```

and beside the other `pub use` lines:

```rust
pub use viewport_input::{
    ViewportButton, ViewportInput, ViewportInputRx, ViewportKey, ViewportModifiers,
    normalize_viewport_pos,
};
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-graph`
Expected: PASS, with the three new tests included and no existing test broken.

- [ ] **Step 6: Commit**

```bash
git add crates/sway-graph/src/viewport_input.rs crates/sway-graph/src/lib.rs
git commit -m "feat(graph): viewport input as plain data on its own channel"
```

---

### Task 2: The `Viewport` widget

**Files:**
- Create: `crates/sway-editor/src/viewport.rs`
- Delete: `crates/sway-editor/src/external.rs`
- Modify: `crates/sway-editor/src/lib.rs` (module list and `VIEWPORT_TAG`'s type only; wiring is Task 3)
- Test: in-file `#[cfg(test)] mod tests`, using `masonry_testing::TestHarness`

**Interfaces:**
- Consumes: `sway_graph::{ViewportInput, ViewportButton, ViewportKey, ViewportModifiers, normalize_viewport_pos}`.
- Produces: `Viewport::new(input: Sender<ViewportInput>) -> Viewport`.

- [ ] **Step 1: Read the widget being replaced**

Read `crates/sway-editor/src/external.rs` end to end. Three things in it are load-bearing and must survive verbatim in the new widget, each for a reason recorded in its doc comments:

1. `paint` sets `PaintLayerMode::External` — this is what leaves the hole the compositor fills.
2. `update`'s `Update::WidgetAdded` arm calls `ctx.request_anim_frame()`, and `on_anim_frame` re-requests plus `request_paint_only()` — without this the widget vanishes from the `VisualLayerPlan` after the first frame and takes the viewport rect with it.
3. `layout` calls `ctx.set_clip_path(size.to_rect())`.

The one thing that changes is `accepts_pointer_interaction`, which returned `false` so hits would fall through to overlapping `NodeBox`es. Since M1b Task 5 the viewport is a `Split` *sibling* of the graph canvas, not a child at a hardcoded rect, so nothing overlaps it any more.

- [ ] **Step 2: Write the failing tests**

Create `crates/sway-editor/src/viewport.rs` containing the test module below plus a `Viewport` struct whose `Widget` impl is copied from `ViewportPlaceholder` (so the file compiles), with `accepts_pointer_interaction` still `false` and no event handlers. Every test must fail for a behavioural reason, not a compile error.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{Receiver, Sender};
    use masonry::core::{DefaultProperties, PointerButton};
    use masonry_testing::TestHarness;
    use sway_graph::{ViewportButton, ViewportInput, ViewportKey};

    fn harness() -> (TestHarness<Viewport>, Receiver<ViewportInput>) {
        let (tx, rx): (Sender<ViewportInput>, Receiver<ViewportInput>) =
            crossbeam_channel::unbounded();
        let harness = TestHarness::create_with_size(
            DefaultProperties::default(),
            Viewport::new(tx).prepare(),
            (400, 200),
        );
        (harness, rx)
    }

    #[test]
    fn a_press_reports_a_normalized_position() {
        let (mut harness, rx) = harness();
        harness.mouse_move((100.0, 50.0));
        harness.mouse_button_press(PointerButton::Primary);

        let event = rx.try_iter().find(|e| matches!(e, ViewportInput::Down { .. }));
        let Some(ViewportInput::Down { button, pos, .. }) = event else {
            panic!("no Down reached the channel");
        };
        assert_eq!(button, ViewportButton::Primary);
        // 100/400, 50/200 in a 400x200 harness.
        assert!((pos.x - 0.25).abs() < 1e-5, "pos.x = {}", pos.x);
        assert!((pos.y - 0.25).abs() < 1e-5, "pos.y = {}", pos.y);
    }

    #[test]
    fn alt_is_carried_through() {
        // Orbit and pan are Alt-gated (spec M7-3); if the modifier is dropped
        // the camera never moves and a plain click orbits instead of picking.
        let (mut harness, rx) = harness();
        harness.mouse_move((100.0, 50.0));
        harness.keyboard_key_down(alt_key());
        harness.mouse_button_press(PointerButton::Primary);

        let Some(ViewportInput::Down { modifiers, .. }) = rx
            .try_iter()
            .find(|e| matches!(e, ViewportInput::Down { .. }))
        else {
            panic!("no Down reached the channel");
        };
        assert!(modifiers.alt, "Alt must survive the boundary");
    }

    #[test]
    fn a_press_claims_focus_so_the_mode_keys_arrive() {
        // The M6 failure class, tested directly: Tasks 13 and 14 of M6 each
        // shipped a feature that could never fire because nothing called
        // `request_focus`. This drives the real text-event path.
        let (mut harness, rx) = harness();
        harness.mouse_move((100.0, 50.0));
        harness.mouse_button_press(PointerButton::Primary);
        harness.mouse_button_release(PointerButton::Primary);

        harness.process_text_event(TextEvent::Keyboard(KeyboardEvent::key_down(
            Key::Character("e".into()),
        )));

        assert!(
            rx.try_iter()
                .any(|e| e == ViewportInput::Key { key: ViewportKey::Rotate }),
            "E must reach the world as a rotate-mode key",
        );
    }

    #[test]
    fn a_cancel_is_forwarded() {
        // A drag whose capture is lost must be abandoned world-side. M6 Task
        // 14 shipped a stuck rubber band by omitting exactly this.
        let (mut harness, rx) = harness();
        harness.mouse_move((100.0, 50.0));
        harness.mouse_button_press(PointerButton::Primary);
        harness.process_pointer_event(PointerEvent::Cancel(PRIMARY_MOUSE));

        assert!(rx.try_iter().any(|e| e == ViewportInput::Cancel));
    }

    #[test]
    fn the_viewport_is_still_an_external_paint_layer() {
        // The compositor's hole. Accepting pointer interaction must not have
        // cost us the reason this widget exists.
        let (mut harness, _rx) = harness();
        let plan = harness.render_plan();
        assert!(
            plan.layers
                .iter()
                .any(|layer| matches!(layer.kind, VisualLayerKind::External)),
            "the viewport must still leave an External layer for the compositor",
        );
    }
}
```

Two mechanical details to resolve while writing this, both by reading the pinned masonry checkout at `~/.cargo/git/checkouts/xilem-*/c5950bc/` rather than guessing:

- `alt_key()` and the `TestHarness` methods used above (`mouse_move`, `mouse_button_press`, `keyboard_key_down`, `render_plan`). `masonry_testing`'s harness is already used by `transport_bar.rs`, `canvas.rs` and `scene_tree.rs` — copy the idioms from `crates/sway-editor/src/canvas.rs`'s test module, which drives `process_pointer_event` and `process_text_event` directly and constructs `PointerScrollEvent`/`PointerState` by hand. If a helper above does not exist under that name, drive the raw event instead; do not invent a `_for_test` bypass.
- `PRIMARY_MOUSE` — `canvas.rs`'s `a_cancel_during_a_socket_drag_clears_it` already uses this constant; import it from the same place.

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo test -p sway-editor viewport`
Expected: FAIL — the first four for want of any event handling (nothing reaches the channel); `the_viewport_is_still_an_external_paint_layer` should already PASS, which is the point of writing it now: it is the regression guard for Step 4.

- [ ] **Step 4: Write the implementation**

```rust
//! `Viewport` — the Bevy viewport's seat in the widget tree, and the only
//! widget that forwards input into the Bevy world.
//!
//! Replaces M1b's `ViewportPlaceholder`, which declined pointer interaction
//! entirely. Everything about the *painting* half is unchanged and still
//! load-bearing: `PaintLayerMode::External` leaves the hole the compositor
//! fills, the `request_anim_frame` loop keeps this widget in every
//! `VisualLayerPlan` (masonry does not repaint idle widgets, and a one-shot
//! placeholder silently vanishes after the first frame), and
//! `EditorUi::viewport_rect` reads the rect off this widget's own bounding
//! box rather than off the layer plan — see that method's doc comment.
//!
//! What is new is the input half. This widget owns no interaction state: no
//! drag anchor, no orbiting flag. It normalizes a position, packages a plain
//! `ViewportInput`, and sends it. The gesture is resolved in `sway-runtime`,
//! where the camera and the meshes are.

use crossbeam_channel::Sender;
use masonry::accesskit::{Node, Role};
use masonry::core::keyboard::Key;
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, Modifiers, NoAction, PaintCtx,
    PointerButton, PointerButtonEvent, PointerEvent, PointerGesture, PointerGestureEvent,
    PointerScrollEvent, PointerState, PointerUpdate, PropertiesMut, PropertiesRef, RegisterCtx,
    ScrollDelta, TextEvent, Update, UpdateCtx, Widget,
};
use masonry::dpi::PhysicalPosition;
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry_core::kurbo::{Axis, Point, Size};
use sway_graph::{
    ViewportButton, ViewportInput, ViewportKey, ViewportModifiers, normalize_viewport_pos,
};

pub struct Viewport {
    input: Sender<ViewportInput>,
    /// The widget's own size in logical pixels, recorded by `layout`.
    /// `EventCtx` exposes no size accessor at this masonry revision, and
    /// normalization needs one every event.
    size: Size,
}

impl Viewport {
    pub fn new(input: Sender<ViewportInput>) -> Self {
        Self { input, size: Size::ZERO }
    }

    fn normalized(&self, ctx: &EventCtx<'_>, position: PhysicalPosition<f64>) -> bevy_math::Vec2 {
        let local: Point = ctx.local_position(position);
        normalize_viewport_pos(
            bevy_math::Vec2::new(local.x as f32, local.y as f32),
            bevy_math::Vec2::new(self.size.width as f32, self.size.height as f32),
        )
    }

    fn send(&self, input: ViewportInput) {
        // A closed channel means the world is gone; the window is on its way
        // down and there is nothing useful to do about it.
        let _ = self.input.send(input);
    }
}

fn modifiers_of(state: &PointerState) -> ViewportModifiers {
    ViewportModifiers {
        alt: state.modifiers.contains(Modifiers::ALT),
        shift: state.modifiers.contains(Modifiers::SHIFT),
        control: state.modifiers.contains(Modifiers::CONTROL),
        meta: state.modifiers.contains(Modifiers::META),
    }
}

fn button_of(button: Option<PointerButton>) -> Option<ViewportButton> {
    match button {
        Some(PointerButton::Primary) => Some(ViewportButton::Primary),
        Some(PointerButton::Secondary) => Some(ViewportButton::Secondary),
        _ => None,
    }
}
```

The `Widget` impl — the painting half copied from `external.rs` unchanged, the input half new:

```rust
impl Widget for Viewport {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::WidgetAdded = event {
            ctx.request_anim_frame();
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _interval: u64) {
        ctx.request_anim_frame();
        ctx.request_paint_only();
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        match event {
            PointerEvent::Down(PointerButtonEvent { button, state, .. }) => {
                let Some(button) = button_of(*button) else {
                    return;
                };
                // Focus first: the mode keys (W/E/R) are delivered by
                // masonry's text-event pass, which only targets the focused
                // widget. M6 Tasks 13 and 14 both shipped features that
                // could never fire because nothing requested focus.
                ctx.request_focus();
                // Keeps `Move` coming when a drag leaves the rectangle,
                // which orbit and gizmo drags both rely on.
                ctx.capture_pointer();
                let pos = self.normalized(ctx, state.position);
                self.send(ViewportInput::Down { button, pos, modifiers: modifiers_of(state) });
                ctx.set_handled();
            }
            PointerEvent::Move(PointerUpdate { current, .. }) => {
                let pos = self.normalized(ctx, current.position);
                self.send(ViewportInput::Move { pos, modifiers: modifiers_of(current) });
            }
            PointerEvent::Up(PointerButtonEvent { button, state, .. }) => {
                let Some(button) = button_of(*button) else {
                    return;
                };
                let pos = self.normalized(ctx, state.position);
                self.send(ViewportInput::Up { button, pos });
                ctx.set_handled();
            }
            PointerEvent::Cancel(..) => {
                self.send(ViewportInput::Cancel);
            }
            PointerEvent::Scroll(PointerScrollEvent { delta, state, .. }) => {
                // Same line/page reduction `GraphCanvas::on_pointer_event`
                // performs, and for the same reason: masonry's policy is in
                // logical CSS pixels, `to_pixel_delta` yields physical, so
                // scale in and convert back.
                let scale = state.scale_factor.max(f64::EPSILON);
                let physical = delta.to_pixel_delta(
                    PhysicalPosition { x: 32.0 * scale, y: 32.0 * scale },
                    PhysicalPosition { x: 800.0 * scale, y: 800.0 * scale },
                );
                let logical = physical.to_logical(scale);
                let pos = self.normalized(ctx, state.position);
                self.send(ViewportInput::Scroll {
                    delta: bevy_math::Vec2::new(logical.x as f32, logical.y as f32),
                    pos,
                    modifiers: modifiers_of(state),
                });
                ctx.set_handled();
            }
            PointerEvent::Gesture(PointerGestureEvent {
                gesture: PointerGesture::Pinch(delta),
                ..
            }) => {
                self.send(ViewportInput::Pinch { delta: *delta as f32 });
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn on_text_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &TextEvent,
    ) {
        let TextEvent::Keyboard(key_event) = event else {
            return;
        };
        if !key_event.state.is_down() {
            return;
        }
        let Key::Character(character) = &key_event.key else {
            return;
        };
        // Only the three gizmo-mode keys are consumed; everything else is
        // left unhandled so it bubbles, exactly as `NodeBox` leaves text
        // events for `GraphCanvas`.
        let key = match character.as_str() {
            c if c.eq_ignore_ascii_case("w") => ViewportKey::Translate,
            c if c.eq_ignore_ascii_case("e") => ViewportKey::Rotate,
            c if c.eq_ignore_ascii_case("r") => ViewportKey::Scale,
            _ => return,
        };
        self.send(ViewportInput::Key { key });
        ctx.set_handled();
    }

    fn accepts_pointer_interaction(&self) -> bool {
        true
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        len_req: LenReq,
        cross_length: Option<Length>,
    ) -> Length {
        match len_req {
            LenReq::FitContent(space) => space,
            LenReq::MinContent | LenReq::MaxContent => cross_length.unwrap_or(Length::ZERO),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        self.size = size;
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, _painter: &mut Painter<'_>) {
        ctx.set_paint_layer_mode(masonry::core::PaintLayerMode::External);
    }

    fn accessibility_role(&self) -> Role {
        Role::GenericContainer
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        ChildrenIds::new()
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-editor viewport`
Expected: PASS, all five.

If `a_press_reports_a_normalized_position` fails with a position of `(0, 0)` while the press clearly landed, the cause is `self.size` still being `Size::ZERO` because `layout` has not run in the harness — call `harness.render_plan()` (or whatever forces a layout pass at this masonry revision) before the press, and say so in a comment.

- [ ] **Step 6: Delete `external.rs` and repoint the module list**

In `crates/sway-editor/src/lib.rs`: delete `pub mod external;`, add `pub mod viewport;`, change the import to `use crate::viewport::Viewport;`, and change `VIEWPORT_TAG`'s type to `WidgetTag<Viewport>`. Keep the doc comment on `VIEWPORT_TAG` — the reason it exists (masonry reports the wrong transform for `External` layers under offsetting ancestors) is unchanged. Then:

```bash
git rm crates/sway-editor/src/external.rs
```

`graph_root` will not compile yet — `Viewport::new` needs a sender it does not have. That is Task 3; to keep this task's commit compiling, pass `crossbeam_channel::unbounded().0` at the call site with a `// Task 3 threads the real sender` comment.

- [ ] **Step 7: Run the whole crate's tests**

Run: `cargo test -p sway-editor`
Expected: PASS. In particular `viewport_rect_reflects_its_position_inside_nested_splits` must still pass — it is the guard that replacing the widget did not move the compositor's hole.

- [ ] **Step 8: Commit**

```bash
git add -A crates/sway-editor/src/
git commit -m "feat(editor): Viewport widget forwards pointer and key input"
```

---

### Task 3: Plumb the channel and drain it in the world

**Files:**
- Create: `crates/sway-runtime/src/viewport/mod.rs`
- Modify: `crates/sway-runtime/src/lib.rs`, `crates/sway-runtime/Cargo.toml`, `crates/sway-editor/src/lib.rs`, `crates/sway-app/src/presenter.rs`, `crates/sway-app/src/shell.rs`, `crates/sway-app/src/main.rs`
- Test: in-file tests in `viewport/mod.rs`

**Interfaces:**
- Consumes: `sway_graph::{ViewportInput, ViewportInputRx}`; `Viewport::new` from Task 2.
- Produces: `EditorViewportPlugin`, `ViewportEvents(pub Vec<ViewportInput>)`, `ViewportSystems` (a `SystemSet`), and `EditorUi::new(size, scale_factor, commands, viewport_input)`.

- [ ] **Step 1: Add the dependency**

In `crates/sway-runtime/Cargo.toml`, under `[dependencies]`:

```toml
sway-graph.workspace = true
```

This is the first `sway-runtime` → `sway-graph` edge. It is the layering the architecture already describes (runtime sits above the engine) and is recorded in the spec's document amendments.

- [ ] **Step 2: Write the failing test**

Create `crates/sway-runtime/src/viewport/mod.rs`:

```rust
//! Viewport interaction: the world half. Spec M7.

use bevy::prelude::*;
use sway_graph::{ViewportInput, ViewportInputRx};

/// This frame's viewport input, replaced wholesale each `PreUpdate`.
///
/// One drain, several readers: the camera, the picker and the gizmo all need
/// the same events, and a channel can only be drained once.
#[derive(Resource, Default)]
pub struct ViewportEvents(pub Vec<ViewportInput>);

/// Everything M7 adds, ordered.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum ViewportSystems {
    /// Fills `ViewportEvents`. `PreUpdate`.
    Drain,
    /// Reads them and moves the editor camera. `PreUpdate`, after `Drain`.
    Camera,
    /// Gizmo drag. `PostUpdate`, before transform propagation.
    GizmoDrag,
    /// Gizmo hover and click-to-select. `PostUpdate`, after propagation.
    Pick,
}

pub fn drain_viewport_input(rx: Option<Res<ViewportInputRx>>, mut events: ResMut<ViewportEvents>) {
    events.0.clear();
    let Some(rx) = rx else {
        return;
    };
    events.0.extend(rx.0.try_iter());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_drain_serves_every_reader_for_a_frame() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(ViewportInputRx(rx))
            .init_resource::<ViewportEvents>()
            .add_systems(Update, drain_viewport_input);

        tx.send(ViewportInput::Cancel).unwrap();
        app.update();
        assert_eq!(app.world().resource::<ViewportEvents>().0.len(), 1);

        // Nothing sent this frame: the buffer must empty, or a click would
        // fire again every frame forever.
        app.update();
        assert!(app.world().resource::<ViewportEvents>().0.is_empty());
    }

    #[test]
    fn a_world_with_no_receiver_drains_nothing_and_does_not_panic() {
        // A show build has no editor channel at all.
        let mut app = App::new();
        app.init_resource::<ViewportEvents>()
            .add_systems(Update, drain_viewport_input);
        app.update();
        assert!(app.world().resource::<ViewportEvents>().0.is_empty());
    }
}
```

- [ ] **Step 3: Run and watch it fail**

Run: `cargo test -p sway-runtime viewport`
Expected: FAIL to compile — `crates/sway-runtime/src/lib.rs` has no `viewport` module yet.

- [ ] **Step 4: Add the module and the plugin**

In `crates/sway-runtime/src/lib.rs` add `pub mod viewport;` and re-export `pub use viewport::EditorViewportPlugin;`. Then add the plugin to `viewport/mod.rs`:

```rust
/// Everything the editor's viewport needs in the world. Added by `sway-app`
/// only under `--editor`; a show build never sees it, so nothing here can
/// affect what happens on stage.
pub struct EditorViewportPlugin;

impl Plugin for EditorViewportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewportEvents>().add_systems(
            PreUpdate,
            drain_viewport_input.in_set(ViewportSystems::Drain),
        );
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-runtime viewport`
Expected: PASS, both.

- [ ] **Step 6: Thread the sender from `main` to the widget**

Four edits, all mechanical, each mirroring how `Sender<EditorCommand>` already travels:

`crates/sway-editor/src/lib.rs` — `graph_root` takes a second sender and hands it to `Viewport::new`; `EditorUi::new` takes it and passes it through:

```rust
fn graph_root(
    commands: Sender<EditorCommand>,
    viewport_input: Sender<ViewportInput>,
) -> NewWidget<dyn Widget> {
    // ...
    let viewport = Viewport::new(viewport_input).prepare().with_tag(VIEWPORT_TAG);
    // ...
}

impl EditorUi {
    pub fn new(
        size: PhysicalSize<u32>,
        scale_factor: f64,
        commands: Sender<EditorCommand>,
        viewport_input: Sender<ViewportInput>,
    ) -> Self {
        // ... `graph_root(commands.clone(), viewport_input)` ...
    }
}
```

`crates/sway-app/src/presenter.rs` — `EditorPresenter::new` grows the same parameter and forwards it to `EditorUi::new`.

`crates/sway-app/src/shell.rs` — `ShellConfig` grows `pub viewport_input: Sender<ViewportInput>`, and `resumed` passes `config.viewport_input` into `EditorPresenter::new`.

`crates/sway-app/src/main.rs` — a second channel beside the existing one, the receiver inserted with the other editor-only resources, the plugin added, and the sender handed to the shell:

```rust
let (editor_tx, editor_rx) = crossbeam_channel::unbounded();
let (viewport_tx, viewport_rx) = crossbeam_channel::unbounded();

// inside build_app, in the existing `if editor { ... }`:
if editor {
    app.insert_resource(sway_graph::Authoring)
        .insert_resource(sway_graph::EditorRx(editor_rx))
        .insert_resource(sway_graph::ViewportInputRx(viewport_rx))
        .add_plugins(sway_runtime::EditorViewportPlugin);
}

// and at the bottom:
shell::run(shell::ShellConfig {
    editor,
    build_app,
    commands: editor_tx,
    viewport_input: viewport_tx,
});
```

Also fix Task 2's placeholder: the `crossbeam_channel::unbounded().0` stub in `graph_root` is now the real parameter.

- [ ] **Step 7: Prove it end to end, by eye**

Temporarily add to `EditorViewportPlugin::build`, after the drain system:

```rust
.add_systems(PreUpdate, (|events: Res<ViewportEvents>| {
    for event in &events.0 {
        info!("viewport input: {event:?}");
    }
}).after(ViewportSystems::Drain))
```

Run: `cargo run -p sway-app -- --editor`

Move the pointer over the 3D viewport, click, drag, scroll, and press W. The log must show `Down`/`Move`/`Up`/`Scroll`/`Key` events with `pos` values inside `[0,1]`. Check specifically that:

- clicking the **graph canvas** produces no viewport events (masonry's hit-testing is the only gate on which pane owns a click);
- `pos` reads about `(0.5, 0.5)` at the centre of the viewport pane, on this Retina display — if it reads `(0.25, 0.25)` or `(1.0, 1.0)`, normalization is against the wrong rect;
- W arrives **without** first clicking any other pane, but does require one click in the viewport (that is the focus request working).

Then delete the temporary logging system before committing.

- [ ] **Step 8: Full suite and commit**

Run: `cargo test --workspace`
Expected: PASS, no regression against the baseline.

```bash
git add -A
git commit -m "feat(runtime): drain viewport input into a per-frame buffer"
```

---

## Phase 2 — The editor camera

### Task 4: `EditorCamera` and its navigation maths

**Files:**
- Create: `crates/sway-runtime/src/viewport/camera.rs`
- Modify: `crates/sway-runtime/src/viewport/mod.rs` (`pub mod camera;`)
- Test: in-file

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `EditorCamera { pivot: Vec3, yaw: f32, pitch: f32, distance: f32 }`, `orbit_transform(&EditorCamera) -> Transform`, `orbit(&mut EditorCamera, delta: Vec2)`, `pan(&mut EditorCamera, delta: Vec2)`, `dolly(&mut EditorCamera, amount: f32)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_camera() -> EditorCamera {
        EditorCamera::default()
    }

    #[test]
    fn the_camera_looks_at_its_pivot_from_its_distance() {
        let cam = default_camera();
        let tf = orbit_transform(&cam);
        assert!(
            (tf.translation.distance(cam.pivot) - cam.distance).abs() < 1e-4,
            "expected to sit {} from the pivot, sat {}",
            cam.distance,
            tf.translation.distance(cam.pivot),
        );
        // Looking at the pivot means forward points from eye to pivot.
        let forward = tf.forward().as_vec3();
        let to_pivot = (cam.pivot - tf.translation).normalize();
        assert!((forward - to_pivot).length() < 1e-4, "{forward:?} vs {to_pivot:?}");
    }

    #[test]
    fn orbiting_turns_the_camera_without_moving_the_pivot_or_the_distance() {
        let mut cam = default_camera();
        let before = orbit_transform(&cam).translation;
        orbit(&mut cam, Vec2::new(0.25, 0.0));
        let after = orbit_transform(&cam);
        assert_ne!(before, after.translation);
        assert!((after.translation.distance(cam.pivot) - cam.distance).abs() < 1e-4);
        assert_eq!(cam.pivot, Vec3::ZERO);
    }

    #[test]
    fn pitch_stops_just_short_of_the_poles() {
        // At exactly ±90° the look-at basis is degenerate and the view rolls
        // over; every orbit camera clamps for this reason.
        let mut cam = default_camera();
        orbit(&mut cam, Vec2::new(0.0, -100.0));
        assert!(cam.pitch < std::f32::consts::FRAC_PI_2);
        assert!(orbit_transform(&cam).translation.is_finite());

        orbit(&mut cam, Vec2::new(0.0, 200.0));
        assert!(cam.pitch > -std::f32::consts::FRAC_PI_2);
        assert!(orbit_transform(&cam).translation.is_finite());
    }

    #[test]
    fn panning_moves_the_pivot_across_the_view_not_along_the_world_axes() {
        // Pan must feel the same whatever direction the camera faces, which
        // means it moves along the camera's own right/up, not X/Y.
        let mut cam = default_camera();
        cam.yaw = std::f32::consts::FRAC_PI_2;
        let right = orbit_transform(&cam).right().as_vec3();
        pan(&mut cam, Vec2::new(0.1, 0.0));
        let moved = (cam.pivot - Vec3::ZERO).normalize();
        assert!(moved.dot(right).abs() > 0.99, "moved {moved:?}, right {right:?}");
    }

    #[test]
    fn panning_scales_with_distance() {
        // The same drag should cover the same fraction of the screen whether
        // you are close in or far out.
        let mut near = default_camera();
        near.distance = 1.0;
        let mut far = default_camera();
        far.distance = 100.0;
        pan(&mut near, Vec2::new(0.1, 0.0));
        pan(&mut far, Vec2::new(0.1, 0.0));
        assert!(far.pivot.length() > near.pivot.length() * 10.0);
    }

    #[test]
    fn dollying_never_reaches_or_passes_the_pivot() {
        let mut cam = default_camera();
        for _ in 0..1000 {
            dolly(&mut cam, 10.0);
        }
        assert!(cam.distance >= MIN_DISTANCE, "distance {}", cam.distance);
        assert!(cam.distance.is_finite());
    }
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-runtime camera`
Expected: FAIL to compile — none of these functions exist.

- [ ] **Step 3: Write the implementation**

```rust
//! The editor's own camera. Spec M7-3.
//!
//! Navigation is a pure function of four numbers, which is what makes it
//! testable with no window, no app and no render device.

use bevy::prelude::*;

/// How far a full-viewport drag turns the camera. Deltas arrive normalized
/// to the viewport rect (spec M7-1), so this is radians per viewport width —
/// a full sweep turns the camera all the way round.
const ORBIT_SENSITIVITY: f32 = std::f32::consts::TAU;
/// Pan distance per viewport width, per unit of `distance`.
const PAN_SENSITIVITY: f32 = 2.0;
/// Dolly is multiplicative so it feels the same at every scale.
const DOLLY_RATE: f32 = 0.15;
/// The pivot can be approached but never reached.
pub const MIN_DISTANCE: f32 = 0.05;
/// Just inside the poles, where the look-at basis degenerates.
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.001;

/// The editor's viewpoint, as opposed to `SceneCamera`, which is what the
/// show looks through.
///
/// Carries no `EditorPos` and no `DocId` on purpose: `capture_nodes` walks
/// every `EditorPos` entity and `to_document` walks every `DocId` carrier, so
/// this camera is invisible to the graph canvas and to the saved file without
/// either of them needing a special case.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
#[require(Camera3d)]
pub struct EditorCamera {
    pub pivot: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Default for EditorCamera {
    fn default() -> Self {
        Self {
            pivot: Vec3::ZERO,
            yaw: 0.0,
            pitch: -0.4,
            distance: 8.0,
        }
    }
}

/// Where the camera sits and what it looks at.
pub fn orbit_transform(cam: &EditorCamera) -> Transform {
    let offset = Vec3::new(
        cam.distance * cam.pitch.cos() * cam.yaw.sin(),
        -cam.distance * cam.pitch.sin(),
        cam.distance * cam.pitch.cos() * cam.yaw.cos(),
    );
    Transform::from_translation(cam.pivot + offset).looking_at(cam.pivot, Vec3::Y)
}

/// Alt + primary drag. `delta` is a normalized-viewport delta.
pub fn orbit(cam: &mut EditorCamera, delta: Vec2) {
    cam.yaw -= delta.x * ORBIT_SENSITIVITY;
    cam.pitch = (cam.pitch - delta.y * ORBIT_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
}

/// Alt + secondary drag. Moves the pivot across the view plane.
pub fn pan(cam: &mut EditorCamera, delta: Vec2) {
    let tf = orbit_transform(cam);
    let scale = cam.distance * PAN_SENSITIVITY;
    cam.pivot += tf.right().as_vec3() * (-delta.x * scale) + tf.up().as_vec3() * (delta.y * scale);
}

/// Scroll or pinch. Positive dollies in.
pub fn dolly(cam: &mut EditorCamera, amount: f32) {
    cam.distance = (cam.distance * (-amount * DOLLY_RATE).exp()).max(MIN_DISTANCE);
    if !cam.distance.is_finite() {
        cam.distance = MIN_DISTANCE;
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-runtime camera`
Expected: PASS, all six. If `panning_moves_the_pivot_across_the_view_not_along_the_world_axes` fails on sign, fix the sign in `pan` — the convention is that the content follows the pointer, so the pivot moves opposite to the drag on X.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-runtime/src/viewport/
git commit -m "feat(runtime): editor camera orbit, pan and dolly maths"
```

---

### Task 5: Drive the camera from viewport input

**Files:**
- Modify: `crates/sway-runtime/src/viewport/camera.rs`, `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file, driving a real `App`

**Interfaces:**
- Consumes: `ViewportEvents`, `ViewportSystems`, the maths from Task 4.
- Produces: `spawn_editor_camera` (Startup), `navigate_editor_camera` (PreUpdate, `ViewportSystems::Camera`).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod nav_tests {
    use super::*;
    use crate::viewport::{ViewportEvents, drain_viewport_input};
    use sway_graph::{ViewportButton, ViewportInput, ViewportModifiers};

    fn alt() -> ViewportModifiers {
        ViewportModifiers { alt: true, ..Default::default() }
    }

    fn app_with_camera() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportEvents>()
            .add_systems(Update, navigate_editor_camera);
        app.world_mut().spawn((EditorCamera::default(), Transform::default()));
        app
    }

    fn feed(app: &mut App, events: Vec<ViewportInput>) {
        app.world_mut().resource_mut::<ViewportEvents>().0 = events;
        app.update();
    }

    #[test]
    fn alt_drag_orbits() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Move { pos: Vec2::new(0.75, 0.5), modifiers: alt() },
        ]);
        let cam = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_ne!(cam.yaw, EditorCamera::default().yaw);
    }

    #[test]
    fn a_plain_drag_does_not_move_the_camera() {
        // Without Alt the gesture belongs to picking and the gizmo. If this
        // regresses, every click drags the view instead of selecting.
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: ViewportModifiers::default() },
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: ViewportModifiers::default() },
        ]);
        let cam = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_move_with_no_press_is_ignored() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: alt() },
        ]);
        let cam = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(cam, EditorCamera::default());
    }

    #[test]
    fn a_cancel_ends_the_gesture() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Cancel,
        ]);
        let before = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        feed(&mut app, vec![
            ViewportInput::Move { pos: Vec2::new(0.9, 0.9), modifiers: alt() },
        ]);
        let after = *app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap();
        assert_eq!(before, after, "a cancelled drag must not keep orbiting");
    }

    #[test]
    fn scroll_and_pinch_both_dolly() {
        let mut app = app_with_camera();
        feed(&mut app, vec![ViewportInput::Scroll {
            delta: Vec2::new(0.0, 10.0),
            pos: Vec2::splat(0.5),
            modifiers: ViewportModifiers::default(),
        }]);
        let scrolled = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap().distance;
        assert_ne!(scrolled, EditorCamera::default().distance);

        feed(&mut app, vec![ViewportInput::Pinch { delta: 0.5 }]);
        let pinched = app.world_mut().query::<&EditorCamera>().single(app.world()).unwrap().distance;
        assert_ne!(pinched, scrolled);
    }

    #[test]
    fn navigating_writes_the_transform() {
        let mut app = app_with_camera();
        feed(&mut app, vec![
            ViewportInput::Down { button: ViewportButton::Primary, pos: Vec2::new(0.5, 0.5), modifiers: alt() },
            ViewportInput::Move { pos: Vec2::new(0.75, 0.5), modifiers: alt() },
        ]);
        let (cam, tf) = app
            .world_mut()
            .query::<(&EditorCamera, &Transform)>()
            .single(app.world())
            .unwrap();
        assert_eq!(*tf, orbit_transform(cam));
    }
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-runtime nav_tests`
Expected: FAIL to compile — `navigate_editor_camera` does not exist.

- [ ] **Step 3: Write the implementation**

Add to `camera.rs`:

```rust
/// Which navigation gesture is in progress, and where the pointer was last
/// seen. Lives here rather than in the widget: the widget is stateless by
/// design (spec M7-2), because the gesture is resolved where the camera is.
#[derive(Default)]
pub struct NavigationDrag {
    mode: Option<NavigationMode>,
    last: Vec2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavigationMode {
    Orbit,
    Pan,
}

/// Spawns the one editor camera. `Startup`, editor builds only.
pub fn spawn_editor_camera(mut commands: Commands) {
    let cam = EditorCamera::default();
    commands.spawn((cam, orbit_transform(&cam)));
}

/// Turns this frame's viewport events into camera motion.
pub fn navigate_editor_camera(
    events: Res<crate::viewport::ViewportEvents>,
    mut drag: Local<NavigationDrag>,
    mut cameras: Query<(&mut EditorCamera, &mut Transform)>,
) {
    if events.0.is_empty() {
        return;
    }
    let Ok((mut cam, mut transform)) = cameras.single_mut() else {
        return;
    };

    let mut changed = false;
    for event in &events.0 {
        match event {
            ViewportInput::Down { button, pos, modifiers } if modifiers.alt => {
                drag.mode = Some(match button {
                    ViewportButton::Primary => NavigationMode::Orbit,
                    ViewportButton::Secondary => NavigationMode::Pan,
                });
                drag.last = *pos;
            }
            ViewportInput::Move { pos, .. } => {
                let Some(mode) = drag.mode else {
                    continue;
                };
                let delta = *pos - drag.last;
                drag.last = *pos;
                match mode {
                    NavigationMode::Orbit => orbit(&mut cam, delta),
                    NavigationMode::Pan => pan(&mut cam, delta),
                }
                changed = true;
            }
            ViewportInput::Up { .. } | ViewportInput::Cancel => drag.mode = None,
            ViewportInput::Scroll { delta, .. } => {
                dolly(&mut cam, delta.y * 0.05);
                changed = true;
            }
            ViewportInput::Pinch { delta } => {
                dolly(&mut cam, *delta * 4.0);
                changed = true;
            }
            _ => {}
        }
    }

    if changed {
        // Never write an equal value (architecture §7).
        let next = orbit_transform(&cam);
        if *transform != next {
            *transform = next;
        }
    }
}
```

Note the `Down` arm's guard: a `Down` **without** Alt deliberately falls through to `_ => {}`, leaving `drag.mode` as it was — which is `None` in every real sequence, because an Alt-less press cannot have started one.

Register both systems in `EditorViewportPlugin::build`:

```rust
.add_systems(Startup, camera::spawn_editor_camera)
.add_systems(
    PreUpdate,
    camera::navigate_editor_camera
        .in_set(ViewportSystems::Camera)
        .after(ViewportSystems::Drain),
)
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-runtime`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-runtime/src/viewport/
git commit -m "feat(runtime): navigate the editor camera from viewport input"
```

---

### Task 6: One active camera, chosen by a resource

**Files:**
- Modify: `crates/sway-runtime/src/viewport/camera.rs`, `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file

**Interfaces:**
- Consumes: `EditorCamera`; `sway_nodes::SceneCamera` — **not** as a dependency (`sway-runtime` must not depend on `sway-nodes`); see Step 3.
- Produces: `ViewportCamera { Editor, Scene }`, `apply_active_camera`.

- [ ] **Step 1: Read `retarget_cameras` first**

Read `crates/sway-runtime/src/headless.rs:97-120`. It points **every** camera at the viewport texture each `Update` and does not touch `is_active`. That is what makes this task necessary: two cameras rendering to one target means the last one drawn wins.

Confirm before writing code (spec "Verify before implementing" #4) that `Camera::is_active = false` stops a camera both rendering and clearing. Read `bevy_render-0.19.0/src/camera/camera.rs`'s extraction — `extract_cameras` skips cameras where `!camera.is_active`. Record what you find in the commit message.

- [ ] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod active_camera_tests {
    use super::*;

    /// Stands in for `SceneCamera`, which lives in `sway-nodes` — a crate
    /// `sway-runtime` deliberately does not depend on. See `apply_active_camera`.
    #[derive(Component)]
    struct TestSceneCamera;

    #[test]
    fn exactly_one_of_the_two_cameras_is_active_in_either_position() {
        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .add_systems(Update, apply_active_camera);
        let editor = app.world_mut().spawn((EditorCamera::default(), Camera::default(), ViewportCameraRole::Editor)).id();
        let scene = app.world_mut().spawn((Camera::default(), ViewportCameraRole::Scene)).id();

        app.update();
        assert!(app.world().get::<Camera>(editor).unwrap().is_active);
        assert!(!app.world().get::<Camera>(scene).unwrap().is_active);

        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.update();
        assert!(!app.world().get::<Camera>(editor).unwrap().is_active);
        assert!(app.world().get::<Camera>(scene).unwrap().is_active);
    }

    #[test]
    fn a_camera_with_no_role_is_left_alone() {
        // The gizmo renderer spawns its own overlay camera (spec M7-8). If
        // this system deactivated every camera it did not recognise, the
        // gizmo would vanish from the screen.
        let mut app = App::new();
        app.init_resource::<ViewportCamera>()
            .add_systems(Update, apply_active_camera);
        let overlay = app.world_mut().spawn(Camera { order: 1, ..Default::default() }).id();

        app.update();
        assert!(
            app.world().get::<Camera>(overlay).unwrap().is_active,
            "an unrelated camera must keep rendering",
        );
    }
}
```

- [ ] **Step 3: Write the implementation**

The wrinkle: `SceneCamera` lives in `sway-nodes`, and `sway-runtime` must not depend on it (the dependency runs the other way — `sway-app` composes both). So the role is expressed by a marker component that `sway-runtime` owns and `sway-app` attaches:

```rust
/// Which camera the viewport shows.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewportCamera {
    #[default]
    Editor,
    Scene,
}

/// Tags a camera as one of the two the toggle switches between.
///
/// A marker rather than a query over `EditorCamera` and `sway_nodes::SceneCamera`
/// because `sway-runtime` does not depend on `sway-nodes` — `sway-app` composes
/// the two. It is also what keeps the gizmo renderer's own overlay camera out
/// of this system's reach.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportCameraRole {
    Editor,
    Scene,
}

pub fn apply_active_camera(
    active: Res<ViewportCamera>,
    mut cameras: Query<(&ViewportCameraRole, &mut Camera)>,
) {
    for (role, mut camera) in &mut cameras {
        let should_be_active = matches!(
            (*active, role),
            (ViewportCamera::Editor, ViewportCameraRole::Editor)
                | (ViewportCamera::Scene, ViewportCameraRole::Scene)
        );
        // Never write an equal value: `Camera` is extracted every frame and
        // a needless write dirties it.
        if camera.is_active != should_be_active {
            camera.is_active = should_be_active;
        }
    }
}

/// Attaches `ViewportCameraRole::Scene` to any camera the document authored.
/// Runs every `Update` because a camera can arrive with a reload.
pub fn tag_scene_cameras(
    mut commands: Commands,
    cameras: Query<Entity, (With<Camera>, Without<ViewportCameraRole>, Without<EditorCamera>)>,
    gizmo_overlay: Query<Entity, With<Camera>>,
) {
    let _ = gizmo_overlay;
    for entity in &cameras {
        commands.entity(entity).insert(ViewportCameraRole::Scene);
    }
}
```

`tag_scene_cameras` as written would also tag the gizmo renderer's overlay camera, which must stay active. Resolve it in the way the pinned source allows: `GizmoOverlayCamera` is private, so query on what is public about it instead — it is the only camera carrying `RenderLayers::layer(15)` (`bevy_gizmos_render`'s `GIZMO_RENDER_LAYER`), and it is spawned in `Startup`. Filter with `Without<RenderLayers>` if the scene cameras never carry one, and verify by reading `transform_gizmo_render.rs`'s spawn block. State the chosen discriminator in a comment; add a test that a camera carrying `RenderLayers::layer(15)` is not tagged.

`spawn_editor_camera` also inserts `ViewportCameraRole::Editor`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-runtime`
Expected: PASS, including the new overlay-camera test.

- [ ] **Step 5: Register and commit**

Add to `EditorViewportPlugin::build`:

```rust
.init_resource::<ViewportCamera>()
.add_systems(Update, (camera::tag_scene_cameras, camera::apply_active_camera).chain())
```

```bash
git add crates/sway-runtime/src/viewport/
git commit -m "feat(runtime): one active viewport camera, chosen by resource"
```

---

### Task 7: The camera toggle in the toolbar

**Files:**
- Modify: `crates/sway-editor/src/transport_bar.rs`, `crates/sway-editor/src/lib.rs`, `crates/sway-app/src/presenter.rs`, `crates/sway-app/src/shell.rs`
- Test: in `transport_bar.rs`

**Interfaces:**
- Consumes: `ViewportCamera` from Task 6.
- Produces: `ViewRequest::ToggleCamera`, `EditorUi::take_view_requests() -> Vec<ViewRequest>`.

- [ ] **Step 1: Write the failing test**

In `transport_bar.rs`'s test module, modelled exactly on `the_save_button_emits_a_save_request`:

```rust
#[test]
fn the_camera_button_asks_the_shell_to_toggle() {
    use crate::ViewRequest;
    let mut harness = harness_with(snapshot(false, 120.0, "001.1.1", true));
    let camera_id = harness.root_widget().camera_button_id();

    harness.mouse_click_on(camera_id, Some(masonry::core::PointerButton::Primary));

    harness.edit_root_widget(|mut bar| {
        assert_eq!(
            TransportBar::take_view_requests(&mut bar),
            vec![ViewRequest::ToggleCamera],
        );
    });
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test -p sway-editor camera_button`
Expected: FAIL to compile — no `ViewRequest`, no `camera_button_id`.

- [ ] **Step 3: Implement**

In `crates/sway-editor/src/lib.rs`, beside `FileRequest`:

```rust
/// A view change the shell performs, asked for by the toolbar. Separate from
/// [`FileRequest`] because it touches the world rather than the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRequest {
    ToggleCamera,
}
```

In `transport_bar.rs`: extend `buttons` from `[WidgetPod<Button>; 3]` to `[WidgetPod<Button>; 4]` with `Button::with_text("Camera")` last, add `view_requests: Vec<ViewRequest>`, add `camera_button_id()`, extend `on_action`'s `match` with `Some(3) => { self.view_requests.push(ViewRequest::ToggleCamera); ctx.set_handled(); return; }`, and add `take_view_requests`. `measure` and `layout` already size from `self.buttons.len()`, so they need no edit — confirm that by re-reading them.

In `EditorUi`, add `take_view_requests` mirroring `take_file_requests`. In `EditorPresenter`, forward it. In `shell.rs`'s `Running::redraw`, service it next to the file requests:

```rust
for request in presenter.take_view_requests() {
    match request {
        sway_editor::ViewRequest::ToggleCamera => {
            let world = self.app.world_mut();
            if let Some(mut active) = world.get_resource_mut::<sway_runtime::viewport::ViewportCamera>() {
                *active = match *active {
                    sway_runtime::viewport::ViewportCamera::Editor => {
                        sway_runtime::viewport::ViewportCamera::Scene
                    }
                    sway_runtime::viewport::ViewportCamera::Scene => {
                        sway_runtime::viewport::ViewportCamera::Editor
                    }
                };
            }
        }
    }
}
```

Borrow note: `take_view_requests` borrows `self.presenter` mutably and the block above borrows `self.app` mutably. Collect the requests into a `Vec` first and let the borrow end, exactly as the existing file-request loop does.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-editor`
Expected: PASS. The pre-existing transport-bar tests must still pass — note `harness_with` sizes the harness at 700px for three buttons; a fourth needs 772px, so widen it and say why in the comment that is already there.

- [ ] **Step 5: Verify by eye — the phase's exit criterion**

Run: `cargo run -p sway-app -- --editor`

- Alt+drag in the viewport orbits the cube; Alt+right-drag pans; scroll dollies.
- The "Camera" button switches to the document's own camera framing and back.
- Dragging in the graph canvas still pans the canvas, and nothing in the viewport moves.

Screenshot the orbited view for the commit message.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(editor): toolbar toggle between the editor and scene cameras"
```

---

## Phase 3 — Selection in the world

### Task 8: `Selection` and `EditorCommand::Select`

**Files:**
- Modify: `crates/sway-graph/src/ctx.rs`, `crates/sway-graph/src/command.rs`, `crates/sway-graph/src/lib.rs`
- Test: in `command.rs`'s test module

**Interfaces:**
- Consumes: nothing.
- Produces: `Selection(pub Option<Entity>)`, `EditorCommand::Select { entity: Option<Entity> }`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn select_sets_the_selection_resource() {
    let mut world = World::new();
    world.init_resource::<Selection>();
    let entity = world.spawn_empty().id();

    apply_editor_command(&mut world, &EditorCommand::Select { entity: Some(entity) });

    assert_eq!(world.resource::<Selection>().0, Some(entity));
}

#[test]
fn selecting_nothing_clears_it() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    world.insert_resource(Selection(Some(entity)));

    apply_editor_command(&mut world, &EditorCommand::Select { entity: None });

    assert_eq!(world.resource::<Selection>().0, None);
}

#[test]
fn deleting_the_selected_entity_clears_the_selection() {
    // Otherwise the inspector and the gizmo both keep pointing at a dead
    // entity, and the gizmo would draw at a stale transform.
    let mut world = World::new();
    world.init_resource::<Selection>();
    let entity = world.spawn(EditorPos(Vec2::ZERO)).id();
    apply_editor_command(&mut world, &EditorCommand::Select { entity: Some(entity) });

    apply_editor_command(&mut world, &EditorCommand::Delete { entity });

    assert_eq!(world.resource::<Selection>().0, None);
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-graph select`
Expected: FAIL to compile — no `Selection`, no `Select` variant.

- [ ] **Step 3: Implement**

In `ctx.rs`, beside `EditorPos`:

```rust
/// The entity the editor is currently pointed at.
///
/// One owner for three views: the scene tree, the graph canvas and the
/// viewport all render from this and all write to it through
/// `EditorCommand::Select`. Before M7 the tree and the canvas each held
/// their own answer and reconciled every frame, which is what made a
/// tree-row selection flicker back when the entity had no canvas node.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection(pub Option<Entity>);
```

`Selection` is deliberately **not** `Reflect`/authorable: it is session state, not document state, so it never reaches a saved file.

In `command.rs`, add the variant to `EditorCommand` and the arm to `apply_editor_command`:

```rust
EditorCommand::Select { entity } => {
    // A selection naming a despawned entity is a no-op rather than a
    // stale pointer.
    let entity = entity.filter(|e| world.get_entity(*e).is_ok());
    let Some(mut selection) = world.get_resource_mut::<Selection>() else {
        return;
    };
    if selection.0 != entity {
        selection.0 = entity;
    }
}
```

and, at the end of the existing `Delete` arm, after the despawn:

```rust
if let Some(mut selection) = world.get_resource_mut::<Selection>()
    && selection.0 == Some(*entity)
{
    selection.0 = None;
}
```

Export `Selection` from `lib.rs` (`pub use ctx::{EditorPos, Selection, TickCtx};`), and `init_resource::<Selection>()` in `WiresPlugin::build` so no caller has to remember it.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-graph`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-graph/src/
git commit -m "feat(graph): selection lives in the world"
```

---

### Task 9: The snapshot reports the selection

**Files:**
- Modify: `crates/sway-editor/src/snapshot.rs`, `crates/sway-app/src/presenter.rs`
- Test: in `snapshot.rs`

**Interfaces:**
- Consumes: `sway_graph::Selection`.
- Produces: `WorldSnapshot::selection: Option<Entity>`; `capture` fills `inspector` itself.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn capture_reports_the_selection_and_inspects_it_in_one_pass() {
    // Before M7 the presenter had to ask the widget tree who was selected,
    // then call `inspect` separately. With selection in the world, capture
    // can answer both.
    let mut world = test_world();
    let entity = world.spawn((Lfo::default(), EditorPos(Vec2::ZERO))).id();
    world.insert_resource(Selection(Some(entity)));

    let snap = capture(&world);

    assert_eq!(snap.selection, Some(entity));
    assert!(
        !snap.inspector.components.is_empty(),
        "a selected entity must arrive already inspected",
    );
}

#[test]
fn an_empty_selection_inspects_nothing() {
    let world = test_world();
    let snap = capture(&world);
    assert_eq!(snap.selection, None);
    assert_eq!(snap.inspector, InspectorView::default());
}
```

Use whatever fixture the surrounding tests already use to build a world with the registries in place (`crate::test_graph`), rather than inventing a new one.

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test -p sway-editor capture_reports_the_selection`
Expected: FAIL to compile — `WorldSnapshot` has no `selection`.

- [ ] **Step 3: Implement**

Add `pub selection: Option<Entity>` to `WorldSnapshot` (it derives `Default`, so the field costs nothing at the other construction sites), and in `capture`:

```rust
pub fn capture(world: &World) -> WorldSnapshot {
    let selection = world
        .get_resource::<Selection>()
        .and_then(|selection| selection.0);
    WorldSnapshot {
        tree: capture_tree(world),
        nodes: capture_nodes(world),
        edges: capture_edges(world),
        diagnostics: world.get_resource::<GraphDiagnostics>().cloned().unwrap_or_default(),
        transport: capture_transport(world),
        inspector: selection.map(|entity| inspect(world, entity)).unwrap_or_default(),
        palette: capture_palette(world),
        selection,
    }
}
```

Then simplify `EditorPresenter::apply_snapshot` to just:

```rust
fn apply_snapshot(&mut self, app: &App) {
    self.editor.apply_snapshot(&sway_editor::snapshot::capture(app.world()));
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-editor && cargo test -p sway-app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A crates/sway-editor/src/snapshot.rs crates/sway-app/src/presenter.rs
git commit -m "feat(editor): the snapshot carries the world's selection"
```

---

### Task 10: Widgets stop owning selection

**Files:**
- Modify: `crates/sway-editor/src/scene_tree.rs`, `crates/sway-editor/src/canvas.rs`, `crates/sway-editor/src/lib.rs`
- Test: in each of the three

**Interfaces:**
- Consumes: `WorldSnapshot::selection`, `EditorCommand::Select`.
- Produces: `SceneTree::new(commands: Sender<EditorCommand>)`; `EditorUi::sync_selection` deleted.

- [ ] **Step 1: Write the failing tests**

In `scene_tree.rs`:

```rust
#[test]
fn pressing_a_row_asks_the_world_to_select_it() {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut harness = harness_with(tx, one_row_snapshot());
    let row = harness.root_widget().row_id_for_test(0);

    harness.mouse_click_on(row, Some(masonry::core::PointerButton::Primary));

    assert_eq!(
        rx.try_iter().collect::<Vec<_>>(),
        vec![EditorCommand::Select { entity: Some(entity(1)) }],
    );
}

#[test]
fn a_row_press_does_not_select_locally() {
    // The world is the only owner now. A local echo would be a second
    // opinion, and reconciling two opinions is what caused the flicker.
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut harness = harness_with(tx, one_row_snapshot());
    let row = harness.root_widget().row_id_for_test(0);

    harness.mouse_click_on(row, Some(masonry::core::PointerButton::Primary));

    assert_eq!(harness.root_widget().selected(), None);
}

#[test]
fn the_snapshot_is_what_highlights_a_row() {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut snap = one_row_snapshot();
    snap.selection = Some(entity(1));
    let harness = harness_with(tx, snap);
    assert_eq!(harness.root_widget().selected(), Some(entity(1)));
}
```

In `lib.rs`'s test module, the regression test for the flicker M6 left open:

```rust
#[test]
fn a_tree_only_selection_survives_repeated_snapshots() {
    // M6's open bug: selecting a row whose entity has no canvas node
    // (an `Lfo` with no wires) reverted after one frame, because
    // `sync_selection` reconciled the tree back to the canvas's empty
    // answer every frame. With the world owning selection there is nothing
    // left to reconcile.
    let (tx, _rx) = crossbeam_channel::unbounded();
    let (vtx, _vrx) = crossbeam_channel::unbounded();
    let mut ui = EditorUi::new(PhysicalSize::new(800, 600), 1.0, tx, vtx);

    let entity = Entity::from_raw_u32(3).expect("valid entity id");
    let mut snap = WorldSnapshot {
        tree: vec![TreeRow {
            entity,
            group: TreeGroup::Graph,
            depth: 0,
            label: "LFO #1".to_string(),
            node_id: None,
        }],
        ..Default::default()
    };
    snap.selection = Some(entity);

    ui.apply_snapshot(&snap);
    ui.redraw();
    ui.apply_snapshot(&snap);
    ui.redraw();

    let selected = ui
        .root
        .edit_widget_with_tag(crate::SCENE_TREE_TAG, |tree| tree.widget.selected());
    assert_eq!(selected, Some(entity), "the selection must not revert");
}
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-editor selection`
Expected: FAIL — `SceneTree::new` takes no sender, `WorldSnapshot.selection` is not read by either widget, and `sync_selection` still reconciles.

- [ ] **Step 3: Implement**

Three coordinated edits:

`SceneTree` — take a `Sender<EditorCommand>` in `new` (store it), and in the row-press handler replace `self.selected = Some(entity);` with a send, keeping the action submission for anything else that listens:

```rust
let _ = self.commands.send(EditorCommand::Select { entity: Some(entity) });
ctx.submit_action::<Self::Action>(SceneTreeAction { entity, node_id: row.node_id });
ctx.request_paint_only();
ctx.set_handled();
```

In `SceneTree::apply_snapshot`, replace the "clear a selection whose row vanished" block with an unconditional push from the snapshot:

```rust
// Selection is the world's, not this widget's (spec M7-5).
this.widget.selected = snap.selection;
```

Note this must run on **every** `apply_snapshot`, not only when the row signature changed — so it goes *before* the early return, not after it. Getting this wrong makes selection appear only when the tree's contents change, which no test above would catch on its own; add an assertion for it.

`GraphCanvas` — in `select_from_action` and `clear_selection`, send the command instead of mutating `self.selected`; in `apply_snapshot`, set the selection from `snap.selection` by translating the entity through the existing slot map (`slots.iter().find(|(_, slot)| slot.entity == entity)`), using the existing `NodeBox::set_selected` calls so the highlight still moves.

`EditorUi` — delete `sync_selection`, `node_ids`, `last_snapshot_node_id` and `selected_entity`, and its call in `redraw`. `apply_snapshot` now pushes the snapshot into all four panes and nothing else.

Delete `selecting_a_node_box_highlights_its_tree_row` and `selecting_a_graph_node_row_highlights_its_node_box` in `lib.rs` — they test the mechanism being removed. Their intent is preserved by the per-widget tests above, plus the phase's by-eye check.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-editor`
Expected: PASS.

- [ ] **Step 5: Verify by eye — the phase's exit criterion**

Run: `cargo run -p sway-app -- --editor`

- Clicking a node in the graph canvas highlights its scene-tree row and fills the inspector.
- Clicking a scene-tree row highlights its node box.
- Clicking an `Lfo` row that has no wires **stays selected** — the flicker M6 recorded is gone.
- Editing a field in the inspector still works (it reads `snap.inspector`, which now comes from the world's selection).

- [ ] **Step 6: Commit**

```bash
git add -A crates/sway-editor/src/
git commit -m "refactor(editor): the world owns selection; sync_selection is gone"
```

---

## Phase 4 — Picking

### Task 11: The viewport ray

**Files:**
- Create: `crates/sway-runtime/src/viewport/pick.rs`
- Modify: `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file

**Interfaces:**
- Consumes: `ViewportCamera`, `ViewportCameraRole`.
- Produces: `viewport_ray(camera: &Camera, camera_transform: &GlobalTransform, pos: Vec2) -> Option<Ray3d>`.

- [ ] **Step 1: Verify the coordinate convention (spec verify-list #1)**

Read `~/.cargo/registry/src/index.crates.io-*/bevy_camera-0.19.0/src/camera.rs`:

- `viewport_to_world` (line ~647) takes `viewport_position: Vec2` and calls `viewport_to_ndc`.
- `viewport_to_ndc` (line ~799) divides by `logical_viewport_rect()`.

Then find what `logical_viewport_rect`/`logical_viewport_size` resolve to for `RenderTarget::TextureView` — follow `NormalizedRenderTarget::get_render_target_info` and check what scale factor a manual texture view reports. Write down the answer in `viewport_ray`'s doc comment. If the scale factor is 1.0, `pos * camera.logical_viewport_size()` is exact and this task is trivial; if it is not, the multiplication must use whatever `logical_viewport_size` actually returns — which is exactly why M7-1 normalizes at the boundary instead of sending pixels.

- [ ] **Step 2: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at +Z looking down -Z, with a known viewport size.
    fn test_camera() -> (Camera, GlobalTransform) {
        // Build it through a real `App` so `Camera::computed` is populated —
        // `viewport_to_world` reads `computed.clip_from_view`, which is
        // filled by Bevy's camera systems, not by `Camera::default()`.
        todo!("see Step 3")
    }

    #[test]
    fn the_centre_of_the_viewport_casts_down_the_camera_forward_axis() {
        let (camera, transform) = test_camera();
        let ray = viewport_ray(&camera, &transform, Vec2::splat(0.5)).expect("a ray");
        let forward = transform.forward().as_vec3();
        assert!(
            ray.direction.as_vec3().dot(forward) > 0.999,
            "centre ray {:?} should point along {forward:?}",
            ray.direction,
        );
    }

    #[test]
    fn the_left_and_right_edges_cast_to_opposite_sides() {
        let (camera, transform) = test_camera();
        let left = viewport_ray(&camera, &transform, Vec2::new(0.0, 0.5)).expect("a ray");
        let right = viewport_ray(&camera, &transform, Vec2::new(1.0, 0.5)).expect("a ray");
        let right_axis = transform.right().as_vec3();
        assert!(left.direction.as_vec3().dot(right_axis) < 0.0);
        assert!(right.direction.as_vec3().dot(right_axis) > 0.0);
    }

    #[test]
    fn a_camera_with_no_viewport_size_yields_no_ray_rather_than_a_panic() {
        let camera = Camera::default();
        let ray = viewport_ray(&camera, &GlobalTransform::default(), Vec2::splat(0.5));
        assert!(ray.is_none());
    }
}
```

- [ ] **Step 3: Build the fixture, then implement**

`test_camera` needs a `Camera` whose `computed` fields are populated, which only Bevy's own systems do. Build it from a real headless app: `crate::headless::build_app` with a tiny `ViewportTexture`, spawn a `Camera3d` at a known transform, run `app.update()` a few times, then read `(&Camera, &GlobalTransform)` back out. `crates/sway-runtime/src/headless.rs`'s own test already builds exactly this scaffolding — copy its shape, including the `GpuContext::new(None)` line and the 4×4 texture.

If that proves too heavy for a unit test, the fallback is to assert on `Camera::viewport_to_world`'s inputs instead of its outputs: test that `viewport_ray` multiplies `pos` by `logical_viewport_size()` and forwards it unchanged, and leave the geometry to the headless integration test in Task 12. Say in a comment which of the two you did and why.

```rust
//! Click-to-select. Spec M7-6.

use bevy::camera::Camera;
use bevy::math::Ray3d;
use bevy::prelude::*;

/// Builds a world-space ray from a normalized viewport position.
///
/// `pos` is `[0,1]²` from the top-left (spec M7-1);
/// `Camera::viewport_to_world` wants viewport pixels, and
/// `logical_viewport_size` is what "viewport pixels" means for this camera's
/// own target — see Step 1's finding for what that resolves to for a
/// `RenderTarget::TextureView`.
///
/// `None` covers a camera with no viewport size yet (the first frame, or a
/// zero-sized target) and a degenerate projection. Both are routine, not
/// errors.
pub fn viewport_ray(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    pos: Vec2,
) -> Option<Ray3d> {
    let size = camera.logical_viewport_size()?;
    camera.viewport_to_world(camera_transform, pos * size).ok()
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-runtime pick`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-runtime/src/viewport/
git commit -m "feat(runtime): build a world ray from a normalized viewport position"
```

---

### Task 12: Click-to-select with `MeshRayCast`

**Files:**
- Modify: `crates/sway-runtime/src/viewport/pick.rs`, `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file, driving a real headless app

**Interfaces:**
- Consumes: `viewport_ray`, `ViewportEvents`, `ViewportCamera`, `Sender<EditorCommand>`… no — see below: the picker writes `Selection` directly.
- Produces: `pick_on_click` (PostUpdate, `ViewportSystems::Pick`).

- [ ] **Step 1: Decide the write path, and record why**

The tree and the canvas send `EditorCommand::Select` because they are on the far side of a channel. The picker is already a world system holding `&mut World`-shaped access, so it writes `Selection` directly — the same reasoning M7-8 gives for the gizmo writing `Transform` directly. Put that sentence in the system's doc comment; a reviewer will otherwise ask why there are two paths.

- [ ] **Step 2: Write the failing test**

```rust
#[cfg(test)]
mod click_tests {
    use super::*;
    use crate::viewport::{ViewportCamera, ViewportCameraRole, ViewportEvents};
    use sway_graph::{Selection, ViewportButton, ViewportInput, ViewportModifiers};

    /// A cube at the origin, a camera looking at it, in a real render-capable
    /// app — `MeshRayCast` needs `Assets<Mesh>` and the `Aabb` that Bevy's own
    /// systems compute, so a bare `World` will not do.
    fn app_with_a_cube() -> (App, Entity) {
        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(64, 64);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = crate::headless::build_app(&gpu, &viewport, size);
        app.add_plugins(crate::viewport::EditorViewportPlugin);
        app.finish();
        app.cleanup();

        let cube = {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let handle = meshes.add(Cuboid::new(2.0, 2.0, 2.0));
            app.world_mut().spawn((Mesh3d(handle), Transform::default())).id()
        };
        app.world_mut().spawn((
            Camera3d::default(),
            ViewportCameraRole::Scene,
            Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        // Several updates: `Aabb` is computed by a PostUpdate system and the
        // camera's projection is filled in by Bevy's camera systems.
        for _ in 0..4 {
            app.update();
        }
        (app, cube)
    }

    fn click(app: &mut App, pos: Vec2) {
        app.world_mut().resource_mut::<ViewportEvents>().0 = vec![
            ViewportInput::Down {
                button: ViewportButton::Primary,
                pos,
                modifiers: ViewportModifiers::default(),
            },
        ];
        app.update();
    }

    #[test]
    fn clicking_a_mesh_selects_it() {
        let (mut app, cube) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        click(&mut app, Vec2::splat(0.5));
        assert_eq!(app.world().resource::<Selection>().0, Some(cube));
    }

    #[test]
    fn clicking_empty_space_clears_the_selection() {
        let (mut app, cube) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.world_mut().resource_mut::<Selection>().0 = Some(cube);
        click(&mut app, Vec2::new(0.02, 0.02));
        assert_eq!(app.world().resource::<Selection>().0, None);
    }

    #[test]
    fn an_alt_click_navigates_instead_of_picking() {
        let (mut app, cube) = app_with_a_cube();
        *app.world_mut().resource_mut::<ViewportCamera>() = ViewportCamera::Scene;
        app.world_mut().resource_mut::<ViewportEvents>().0 = vec![ViewportInput::Down {
            button: ViewportButton::Primary,
            pos: Vec2::splat(0.5),
            modifiers: ViewportModifiers { alt: true, ..Default::default() },
        }];
        app.update();
        assert_eq!(app.world().resource::<Selection>().0, None);
        let _ = cube;
    }
}
```

`sway-gpu` is already a `sway-runtime` dependency, so the fixture needs no manifest change. If `GpuContext::new(None)` cannot run in the environment executing the tests, mark these `#[ignore]` with the reason — but do not delete them, and run them by hand before the phase gate.

- [ ] **Step 3: Run and watch them fail**

Run: `cargo test -p sway-runtime click_tests`
Expected: FAIL — no `pick_on_click`.

- [ ] **Step 4: Implement**

```rust
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};

/// Selects the mesh under a plain primary press.
///
/// Writes `Selection` directly rather than sending `EditorCommand::Select`:
/// the tree and the canvas send commands because they live on the far side
/// of a channel, while this is already a world system — the same reasoning
/// the gizmo uses for writing `Transform` (spec M7-8).
///
/// `MeshRayCast` is used as a bare `SystemParam`. `MeshPickingPlugin` is
/// deliberately not added: it exists to run `bevy_picking`'s own pointer
/// backend, which needs `bevy_winit` — disabled here. Its `SystemParam`
/// fields are `Res<Assets<Mesh>>`, three `Local`s and two `Query`s, none of
/// which that plugin initialises (spec M7-6).
pub fn pick_on_click(
    events: Res<crate::viewport::ViewportEvents>,
    active: Res<crate::viewport::ViewportCamera>,
    cameras: Query<(&Camera, &GlobalTransform, &crate::viewport::ViewportCameraRole)>,
    gizmo_state: Option<Res<bevy::gizmos::transform_gizmo::TransformGizmoState>>,
    mut ray_cast: MeshRayCast,
    mut selection: ResMut<sway_graph::Selection>,
) {
    // A drag on a gizmo handle is not a pick. Task 15 makes this reachable;
    // until then `active` is always false.
    if gizmo_state.is_some_and(|state| state.active) {
        return;
    }

    for event in &events.0 {
        let ViewportInput::Down { button: ViewportButton::Primary, pos, modifiers } = event else {
            continue;
        };
        if modifiers.alt {
            // Alt+drag is navigation (spec M7-3).
            continue;
        }

        let Some((camera, camera_transform)) = cameras.iter().find_map(|(camera, tf, role)| {
            matches!(
                (*active, role),
                (ViewportCamera::Editor, ViewportCameraRole::Editor)
                    | (ViewportCamera::Scene, ViewportCameraRole::Scene)
            )
            .then_some((camera, tf))
        }) else {
            continue;
        };

        let Some(ray) = viewport_ray(camera, camera_transform, *pos) else {
            continue;
        };

        // The gizmo's own handle meshes are `Mesh3d` entities sitting right
        // under the cursor whenever a gizmo is up (spec M7-8, consequence 1).
        let filter = |entity: Entity| !is_gizmo_mesh(entity);
        let settings = MeshRayCastSettings::default()
            .with_filter(&filter)
            .always_early_exit();

        let hit = ray_cast.cast_ray(ray, &settings).first().map(|(entity, _)| *entity);
        if selection.0 != hit {
            selection.0 = hit;
        }
    }
}
```

`is_gizmo_mesh` cannot be a closure over a `Query` and a `MeshRayCast` at once without fighting the borrow checker — resolve it by collecting the gizmo mesh entities into a `HashSet` from a `Query<Entity, Or<(With<TransformGizmoRoot>, With<TransformGizmoMeshMarker>)>>` before the loop, and closing over that set. Until Task 13 spawns them the set is empty, which is why these tests pass before the gizmo exists.

Register it:

```rust
.add_systems(
    PostUpdate,
    pick::pick_on_click
        .in_set(ViewportSystems::Pick)
        .after(bevy::transform::TransformSystems::Propagate),
)
```

`PostUpdate` after propagation, so the camera's `GlobalTransform` reflects any orbiting done earlier in the same frame.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p sway-runtime`
Expected: PASS.

- [ ] **Step 6: Verify by eye — the phase's exit criterion**

Run: `cargo run -p sway-app -- --editor`

Click the cube in the viewport: its scene-tree row highlights, its node box highlights, and the inspector fills. Click empty space: all three clear. Alt+drag still orbits and selects nothing.

- [ ] **Step 7: Commit**

```bash
git add crates/sway-runtime/src/viewport/
git commit -m "feat(runtime): click a mesh in the viewport to select it"
```

---

## Phase 5 — The gizmo

### Task 13: Turn Bevy's gizmo renderer on

**Files:**
- Create: `crates/sway-runtime/src/viewport/gizmo.rs`
- Modify: `crates/sway-runtime/src/viewport/mod.rs`, `crates/sway-editor/src/snapshot.rs`
- Test: in `gizmo.rs` and `snapshot.rs`

**Interfaces:**
- Consumes: `sway_graph::Selection`, `ViewportCamera`.
- Produces: `follow_selection` (keeps `TransformGizmoFocus` on the selection), `mark_gizmo_camera`, and the two `init_resource` calls.

- [ ] **Step 1: Verify the renderer switches on from resources alone (spec verify-list #2)**

Read `~/.cargo/registry/src/index.crates.io-*/bevy_gizmos_render-0.19.0/src/lib.rs:100-125` and `transform_gizmo_render.rs`. Confirm three things and record them in the commit message:

1. `TransformGizmoRenderPlugin` is added by `GizmoPlugin::build` whenever `PbrPlugin` is present — so this app already has it and must not add it again.
2. `spawn_gizmo_meshes` runs in `Startup` behind `resource_exists::<TransformGizmoSettings>`, so both resources must be inserted at plugin-build time, before `Startup` runs.
3. What `spawn_gizmo_meshes` puts on the overlay camera (`transform_gizmo_render.rs:334-347`): `Camera3d::default()`, `Camera { order: 1, .. }`, `RenderLayers::layer(15)`. Note whether `clear_color` is left at its default — spec verify-list #3.

- [ ] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::gizmos::transform_gizmo::{TransformGizmoFocus, TransformGizmoSettings};
    use sway_graph::Selection;

    #[test]
    fn the_selection_carries_the_gizmo_focus() {
        let mut app = App::new();
        app.init_resource::<Selection>()
            .add_systems(Update, follow_selection);
        let a = app.world_mut().spawn(Transform::default()).id();
        let b = app.world_mut().spawn(Transform::default()).id();

        app.world_mut().resource_mut::<Selection>().0 = Some(a);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(a).is_some());

        app.world_mut().resource_mut::<Selection>().0 = Some(b);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(a).is_none(), "only one focus at a time");
        assert!(app.world().get::<TransformGizmoFocus>(b).is_some());

        app.world_mut().resource_mut::<Selection>().0 = None;
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(b).is_none());
    }

    #[test]
    fn an_entity_with_no_transform_gets_no_focus() {
        // Selecting an `Lfo` must not put a gizmo anywhere.
        let mut app = App::new();
        app.init_resource::<Selection>()
            .add_systems(Update, follow_selection);
        let lfo = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Selection>().0 = Some(lfo);
        app.update();
        assert!(app.world().get::<TransformGizmoFocus>(lfo).is_none());
    }

    #[test]
    fn the_plugin_initialises_what_the_renderer_needs() {
        let mut app = App::new();
        app.add_plugins(crate::viewport::EditorViewportPlugin);
        assert!(app.world().get_resource::<TransformGizmoSettings>().is_some());
        assert!(app.world().get_resource::<TransformGizmoState>().is_some());
        assert!(
            !app.is_plugin_added::<bevy::gizmos::transform_gizmo::TransformGizmoPlugin>(),
            "its two systems need a Window this app does not have (spec M7-8)",
        );
    }
}
```

In `snapshot.rs`:

```rust
#[test]
fn gizmo_handle_meshes_stay_out_of_the_scene_tree() {
    // The renderer spawns ~10 mesh entities carrying `Transform`, and
    // `capture_tree` walks every `Transform` entity.
    let mut world = test_world();
    world.spawn((Transform::default(), TransformGizmoRoot));
    world.spawn((
        Transform::default(),
        TransformGizmoMeshMarker { axis: TransformGizmoAxis::X, mode: TransformGizmoMode::Translate },
    ));

    let snap = capture(&world);

    assert!(
        snap.tree.is_empty(),
        "the gizmo's own meshes must not appear as scene rows: {:?}",
        snap.tree,
    );
}
```

This test forces a decision: `sway-editor` would need `bevy_gizmos` to name those marker types, and `sway-editor` must not depend on `bevy_render` (which `bevy_gizmos` pulls in). Resolve it **without** the dependency — filter on what is already visible to `sway-editor`. The options, in order of preference:

1. `sway-runtime` inserts a marker of its own (`ViewportOnly`) on the gizmo entities and `sway-editor` filters on that — but `sway-editor` cannot name a `sway-runtime` type either.
2. `sway-graph` owns a `HiddenFromEditor` marker component; `sway-runtime` inserts it on every `TransformGizmoRoot`/`TransformGizmoMeshMarker` entity as they appear, and `capture_tree`/`capture_nodes` skip it. Both crates already depend on `sway-graph`.

Take option 2 unless reading the code shows a reason not to; adjust the test above to spawn `HiddenFromEditor` instead, and put the marker's justification in its doc comment.

- [ ] **Step 3: Run and watch them fail**

Run: `cargo test -p sway-runtime gizmo && cargo test -p sway-editor gizmo_handle`
Expected: FAIL — nothing exists yet.

- [ ] **Step 4: Implement**

```rust
//! The transform gizmo. Spec M7-8.
//!
//! Bevy 0.19 ships a complete one in `bevy_gizmos::transform_gizmo`, and its
//! renderer is already in this app — `GizmoPlugin::build` adds
//! `TransformGizmoRenderPlugin` whenever `PbrPlugin` is present, gated only on
//! `TransformGizmoSettings` existing. What this module supplies is the half
//! that cannot be reused: `transform_gizmo_hover` and `transform_gizmo_drag`
//! are private and both take `Single<&Window, With<PrimaryWindow>>` plus
//! `ButtonInput<MouseButton>`, and this app has no Bevy window at all. Their
//! geometry, however, is public — `intersect_plane`, `axis_direction`,
//! `point_to_segment_dist` and the rest — so only the input plumbing is
//! rewritten here, against normalized viewport coordinates.

use bevy::gizmos::transform_gizmo::{
    TransformGizmoCamera, TransformGizmoFocus, TransformGizmoSettings, TransformGizmoSpace,
    TransformGizmoState,
};
use bevy::prelude::*;
use sway_graph::Selection;

/// Keeps `TransformGizmoFocus` on the selection, and only there.
pub fn follow_selection(
    mut commands: Commands,
    selection: Res<Selection>,
    focused: Query<Entity, With<TransformGizmoFocus>>,
    transforms: Query<(), With<Transform>>,
) {
    // Only an entity with a `Transform` can carry a gizmo: selecting an
    // `Lfo` must leave the viewport alone.
    let wanted = selection.0.filter(|entity| transforms.get(*entity).is_ok());
    for entity in &focused {
        if Some(entity) != wanted {
            commands.entity(entity).remove::<TransformGizmoFocus>();
        }
    }
    if let Some(entity) = wanted
        && focused.get(entity).is_err()
    {
        commands.entity(entity).insert(TransformGizmoFocus);
    }
}

/// Puts `TransformGizmoCamera` on whichever camera is currently rendering.
///
/// Not optional here: the marker may be omitted only when the world holds
/// exactly one camera, and this world holds three — the editor camera, the
/// document's scene camera, and the gizmo renderer's own overlay camera.
pub fn mark_gizmo_camera(
    mut commands: Commands,
    active: Res<crate::viewport::ViewportCamera>,
    cameras: Query<(Entity, &crate::viewport::ViewportCameraRole)>,
    marked: Query<Entity, With<TransformGizmoCamera>>,
) {
    let wanted = cameras.iter().find_map(|(entity, role)| {
        matches!(
            (*active, role),
            (crate::viewport::ViewportCamera::Editor, crate::viewport::ViewportCameraRole::Editor)
                | (crate::viewport::ViewportCamera::Scene, crate::viewport::ViewportCameraRole::Scene)
        )
        .then_some(entity)
    });
    for entity in &marked {
        if Some(entity) != wanted {
            commands.entity(entity).remove::<TransformGizmoCamera>();
        }
    }
    if let Some(entity) = wanted
        && marked.get(entity).is_err()
    {
        commands.entity(entity).insert(TransformGizmoCamera);
    }
}
```

In `EditorViewportPlugin::build`:

```rust
// Switches `TransformGizmoRenderPlugin`'s systems on; both must exist
// before `Startup`, when `spawn_gizmo_meshes` runs.
.init_resource::<TransformGizmoState>()
.insert_resource(TransformGizmoSettings {
    space: TransformGizmoSpace::World,
    // Nothing here owns a cursor to confine, and the setting reaches for
    // `CursorOptions` on a window that does not exist.
    confine_cursor: false,
    ..Default::default()
})
.add_systems(Update, (gizmo::follow_selection, gizmo::mark_gizmo_camera))
```

Plus the `HiddenFromEditor` marker decided in Step 2: add it to `sway-graph`, insert it from a `sway-runtime` system on any `TransformGizmoRoot`/`TransformGizmoMeshMarker` entity that lacks it, and filter it out in `capture_tree` and `capture_nodes`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Verify by eye**

Run: `cargo run -p sway-app -- --editor`

Click the cube. Coloured translate handles must appear at it, drawn over the scene, holding a constant screen size as you dolly. Check specifically for spec verify-list #3: the scene must still be visible behind the gizmo. If the viewport goes black except for handles, the overlay camera is clearing the target — fix it by writing `ClearColorConfig::None` onto the camera carrying `RenderLayers::layer(15)`, in a system of ours, and say so in the commit message.

Then select an `Lfo` node in the canvas: no handles anywhere.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(runtime): draw bevy's transform gizmo on the selection"
```

---

### Task 14: Mode keys and hover

**Files:**
- Modify: `crates/sway-runtime/src/viewport/gizmo.rs`, `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file

**Interfaces:**
- Consumes: `ViewportEvents`, `ViewportKey`, the public helpers in `bevy::gizmos::transform_gizmo`.
- Produces: `set_gizmo_mode`, `viewport_gizmo_hover`, `cursor_in_viewport_pixels`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_mode_keys_switch_modes() {
    let mut app = App::new();
    app.init_resource::<ViewportEvents>()
        .init_resource::<TransformGizmoSettings>()
        .add_systems(Update, set_gizmo_mode);

    for (key, expected) in [
        (ViewportKey::Rotate, TransformGizmoMode::Rotate),
        (ViewportKey::Scale, TransformGizmoMode::Scale),
        (ViewportKey::Translate, TransformGizmoMode::Translate),
    ] {
        app.world_mut().resource_mut::<ViewportEvents>().0 = vec![ViewportInput::Key { key }];
        app.update();
        assert_eq!(app.world().resource::<TransformGizmoSettings>().mode, expected);
    }
}

#[test]
fn hovering_an_axis_reports_it() {
    // A gizmo at the origin, a camera on +Z: a cursor to the right of centre
    // must land on the X handle and nothing else.
    let (mut app, _focus) = app_with_a_focused_gizmo();
    hover(&mut app, Vec2::new(0.62, 0.5));
    assert_eq!(
        app.world().resource::<TransformGizmoState>().hovered_axis,
        Some(TransformGizmoAxis::X),
    );
}

#[test]
fn hovering_empty_space_reports_nothing() {
    let (mut app, _focus) = app_with_a_focused_gizmo();
    hover(&mut app, Vec2::new(0.05, 0.95));
    assert_eq!(app.world().resource::<TransformGizmoState>().hovered_axis, None);
}

#[test]
fn hover_is_frozen_during_a_drag() {
    // Bevy's own hover system returns early when `state.active`; ours must
    // too, or the axis would change under the cursor mid-drag.
    let (mut app, _focus) = app_with_a_focused_gizmo();
    app.world_mut().resource_mut::<TransformGizmoState>().active = true;
    app.world_mut().resource_mut::<TransformGizmoState>().hovered_axis = Some(TransformGizmoAxis::Y);
    hover(&mut app, Vec2::new(0.62, 0.5));
    assert_eq!(
        app.world().resource::<TransformGizmoState>().hovered_axis,
        Some(TransformGizmoAxis::Y),
    );
}
```

`app_with_a_focused_gizmo` reuses Task 12's `app_with_a_cube` fixture with `TransformGizmoFocus` on the cube and a known camera pose. `hover` sets `ViewportEvents` to a single `Move` at that position and calls `app.update()`.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-runtime gizmo`
Expected: FAIL — the systems do not exist.

- [ ] **Step 3: Implement**

Read `bevy_gizmos-0.19.0/src/transform_gizmo.rs:282-395` (`transform_gizmo_hover`) and port it, changing exactly two things: where the cursor comes from, and that its `Single<&Window>` becomes nothing at all.

```rust
/// The cursor in the viewport pixel space `world_to_viewport` reports in.
///
/// Bevy's own gizmo reads `window.cursor_position()`; there is no window
/// here, so the normalized position from the widget is scaled by the
/// camera's own viewport size — the same conversion `viewport_ray` does.
fn cursor_in_viewport_pixels(camera: &Camera, pos: Vec2) -> Option<Vec2> {
    Some(pos * camera.logical_viewport_size()?)
}

pub fn set_gizmo_mode(
    events: Res<crate::viewport::ViewportEvents>,
    mut settings: ResMut<TransformGizmoSettings>,
) {
    for event in &events.0 {
        let ViewportInput::Key { key } = event else {
            continue;
        };
        let mode = match key {
            ViewportKey::Translate => TransformGizmoMode::Translate,
            ViewportKey::Rotate => TransformGizmoMode::Rotate,
            ViewportKey::Scale => TransformGizmoMode::Scale,
        };
        if settings.mode != mode {
            settings.mode = mode;
        }
    }
}

/// Which handle is under the cursor. A port of Bevy's private
/// `transform_gizmo_hover` with the window removed; the geometry is Bevy's
/// own `point_to_segment_dist` / `point_to_ring_screen_dist`.
pub fn viewport_gizmo_hover(
    events: Res<crate::viewport::ViewportEvents>,
    focus: Query<&GlobalTransform, With<TransformGizmoFocus>>,
    cameras: Query<(&Camera, &GlobalTransform), With<TransformGizmoCamera>>,
    settings: Res<TransformGizmoSettings>,
    mut state: ResMut<TransformGizmoState>,
) {
    if state.active {
        return;
    }
    let Some(pos) = events.0.iter().rev().find_map(|event| match event {
        ViewportInput::Move { pos, .. } | ViewportInput::Down { pos, .. } => Some(*pos),
        _ => None,
    }) else {
        return;
    };
    // ... the ported body: resolve focus + camera, compute `scale` from
    // `settings.screen_scale_factor` and camera distance, then for each of
    // X/Y/Z compute the screen distance per `settings.mode` and keep the
    // nearest within `settings.axis_hit_distance`.
}
```

Port the body faithfully rather than inventing thresholds: `AXIS_START_OFFSET`, `settings.axis_length`, `settings.rotate_ring_radius`, `settings.axis_hit_distance` and `screen_scale_factor` are all public and already tuned to the meshes the renderer draws. Skip the `View` axis handle only if it does not fall out for free; if it does, keep it.

Register:

```rust
.add_systems(
    PostUpdate,
    (gizmo::set_gizmo_mode, gizmo::viewport_gizmo_hover)
        .chain()
        .after(bevy::transform::TransformSystems::Propagate)
        .before(ViewportSystems::Pick),
)
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p sway-runtime`
Expected: PASS. If `hovering_an_axis_reports_it` picks the wrong axis, print the three computed distances before adjusting anything — the likeliest cause is a cursor conversion, not the geometry, since the geometry is Bevy's.

- [ ] **Step 5: Commit**

```bash
git add crates/sway-runtime/src/viewport/gizmo.rs crates/sway-runtime/src/viewport/mod.rs
git commit -m "feat(runtime): gizmo mode keys and handle hover"
```

---

### Task 15: Dragging a handle writes the transform

**Files:**
- Modify: `crates/sway-runtime/src/viewport/gizmo.rs`, `crates/sway-runtime/src/viewport/mod.rs`
- Test: in-file

**Interfaces:**
- Consumes: everything above.
- Produces: `viewport_gizmo_drag` (PostUpdate, before `TransformSystems::Propagate`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn dragging_the_x_handle_moves_along_x_only() {
    let (mut app, cube) = app_with_a_focused_gizmo();
    press_on_axis(&mut app, TransformGizmoAxis::X, Vec2::new(0.62, 0.5));
    drag_to(&mut app, Vec2::new(0.72, 0.5));

    let tf = app.world().get::<Transform>(cube).unwrap();
    assert!(tf.translation.x.abs() > 0.01, "x did not move: {:?}", tf.translation);
    assert!(tf.translation.y.abs() < 1e-4, "y moved: {:?}", tf.translation);
    assert!(tf.translation.z.abs() < 1e-4, "z moved: {:?}", tf.translation);
}

#[test]
fn a_release_ends_the_drag() {
    let (mut app, cube) = app_with_a_focused_gizmo();
    press_on_axis(&mut app, TransformGizmoAxis::X, Vec2::new(0.62, 0.5));
    drag_to(&mut app, Vec2::new(0.72, 0.5));
    release(&mut app);
    let after_release = *app.world().get::<Transform>(cube).unwrap();

    drag_to(&mut app, Vec2::new(0.9, 0.5));

    assert_eq!(*app.world().get::<Transform>(cube).unwrap(), after_release);
    assert!(!app.world().resource::<TransformGizmoState>().active);
}

#[test]
fn a_cancel_ends_the_drag_too() {
    // Same hazard M6 Task 14 found on the canvas: without this the state
    // stays `active` forever and picking never works again.
    let (mut app, _cube) = app_with_a_focused_gizmo();
    press_on_axis(&mut app, TransformGizmoAxis::X, Vec2::new(0.62, 0.5));
    feed(&mut app, vec![ViewportInput::Cancel]);
    assert!(!app.world().resource::<TransformGizmoState>().active);
}

#[test]
fn rotate_mode_turns_the_object_without_moving_it() {
    let (mut app, cube) = app_with_a_focused_gizmo();
    app.world_mut().resource_mut::<TransformGizmoSettings>().mode = TransformGizmoMode::Rotate;
    let before = *app.world().get::<Transform>(cube).unwrap();
    press_on_axis(&mut app, TransformGizmoAxis::Y, ring_point_for_y());
    drag_to(&mut app, ring_point_for_y() + Vec2::new(0.06, 0.0));

    let after = app.world().get::<Transform>(cube).unwrap();
    assert_ne!(after.rotation, before.rotation);
    assert_eq!(after.translation, before.translation);
}

#[test]
fn a_drag_on_a_handle_does_not_also_select_something() {
    // `pick_on_click` runs after this system and skips while a drag is
    // active. If it did not, grabbing a handle would reselect whatever mesh
    // the ray happened to hit behind it.
    let (mut app, cube) = app_with_a_focused_gizmo();
    press_on_axis(&mut app, TransformGizmoAxis::X, Vec2::new(0.62, 0.5));
    assert_eq!(app.world().resource::<Selection>().0, Some(cube));
}

#[test]
fn a_parented_object_moves_the_same_distance_as_an_unparented_one() {
    // The gizmo displays at `GlobalTransform` and writes local `Transform`;
    // the demo document's own cube is parented, so a version that forgot the
    // parent's inverse would be visibly wrong on the first real run.
    let (mut app, cube) = app_with_a_focused_gizmo();
    let parent = app
        .world_mut()
        .spawn(Transform::from_xyz(5.0, 0.0, 0.0).with_scale(Vec3::splat(2.0)))
        .id();
    app.world_mut().entity_mut(cube).insert(ChildOf(parent));
    app.update();

    press_on_axis(&mut app, TransformGizmoAxis::X, /* recomputed for the moved cube */ Vec2::new(0.62, 0.5));
    drag_to(&mut app, Vec2::new(0.72, 0.5));

    let world_x = app.world().get::<GlobalTransform>(cube).unwrap().translation().x;
    assert!(world_x > 5.0, "the child must move in world space: {world_x}");
}
```

The last test's cursor positions have to be recomputed once the cube sits at the parent's offset — do that by projecting the cube's `GlobalTransform` through `Camera::world_to_viewport` in the test helper rather than by hardcoding numbers, and reuse that helper in `press_on_axis` for the others too.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p sway-runtime gizmo`
Expected: FAIL — `viewport_gizmo_drag` does not exist.

- [ ] **Step 3: Implement**

Port `bevy_gizmos-0.19.0/src/transform_gizmo.rs:396-637` (`transform_gizmo_drag`) with three substitutions:

1. `window.cursor_position()` → `cursor_in_viewport_pixels` from Task 14.
2. `mouse.just_pressed(MouseButton::Left)` → a `ViewportInput::Down` with `Primary` and no Alt in this frame's `ViewportEvents`; `mouse.pressed` → the drag state's own `active`; `just_released` → `Up` or `Cancel`.
3. The `CursorOptions` / `CursorGrabMode` block → deleted. There is no cursor to confine, `confine_cursor` is already `false`, and `Local<CursorGrabMode>` has nothing to save.

Everything else — `intersect_plane`, `translation_plane_normal`, `axis_direction`, `gizmo_rotation`, `effective_space`, the plane choices per mode, the local-vs-world write through the parent — is called, not rewritten. Where Bevy's version calls the private `snap_value`, skip it: snapping is out of scope and its settings stay `None`.

Register it, and add the ordering the `a_drag_on_a_handle_does_not_also_select_something` test depends on:

```rust
.add_systems(
    PostUpdate,
    gizmo::viewport_gizmo_drag
        .in_set(ViewportSystems::GizmoDrag)
        .before(bevy::transform::TransformSystems::Propagate),
)
```

`pick_on_click` already returns early when `TransformGizmoState.active`, and runs after propagation — so the drag claims the press first, in the same frame.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS, no regression against the Task 1 baseline.

- [ ] **Step 5: Walk the exit criterion by eye**

This is M7's exit criterion. Run it in one unbroken session, and have the human partner drive it — automated GUI click/drag is established as unreliable in this sandbox (M6 Tasks 8, 11, 13).

Run: `cargo run -p sway-app -- --editor`

1. Alt+drag to frame the cube.
2. Click it: tree row, node box and inspector all follow.
3. Drag the X handle: the cube moves, and the inspector's `Transform` updates as it goes.
4. Press E: rotation rings. Drag one: the cube turns.
5. Press R: scale handles. Drag one: the cube scales.
6. Save As to a new path. Quit. Relaunch. Open that path.
7. The cube is where it was left.

Also confirm the negative case the spec accepts: with the demo's `Vec3`→`translation` wire connected, dragging the cube's translate handle springs back on the next tick (spec M7-7). That is the designed behaviour, not a bug — note it in the commit message so it is not "fixed" later by accident.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(runtime): drag a gizmo handle to transform the selection"
```

---

## Phase 6 — Documents

### Task 16: Amend the documents and write the findings

**Files:**
- Modify: `docs/architecture.md`, `docs/superpowers/specs/2026-07-25-sway-design.md`, `docs/superpowers/specs/2026-08-09-mvp-roadmap-design.md`
- Create: `docs/superpowers/reports/2026-08-15-m7-viewport-interaction-findings.md`

- [ ] **Step 1: Amend the roadmap**

In `2026-08-09-mvp-roadmap-design.md`:
- M7's bullet "A translate/rotate/scale gizmo, analytic ray-vs-handle, writing `Transform`. Driven axes render inert." — strike the last sentence per M7-7, and note that the gizmo is Bevy's own with only its input half replaced (M7-8).
- The open question "**`MeshRayCast` outside its plugin**" — mark resolved, with the finding: its `SystemParam` is `Res<Assets<Mesh>>`, three `Local`s and two `Query`s, none plugin-initialised, and `picking` is already on via bevy's default `3d` feature. The hand-rolled ray-vs-AABB fallback was not built.

- [ ] **Step 2: Amend the roadmap summary**

In `2026-07-25-sway-design.md`:
- The M7 line: drop "with driven axes inert"; mention the editor camera is not persisted.
- The open question about `MeshRayCast`: resolved, same wording as above.
- Update the **Status** line to "M5, M6, M7 complete".

- [ ] **Step 3: Amend the architecture**

In `docs/architecture.md`:
- §7's "Whether a future gizmo (M7) follows the same rule is open — M6-5 leaves it for M7 to decide." → settled: the gizmo writes through, exactly as the inspector does; a drag on a wire-driven field holds for one tick.
- §5's ownership table: add a row for selection — owner **`sway-graph`** (`Selection` resource), read by the editor through the snapshot.
- §8: record `sway-runtime`'s new dependency on `sway-graph`, and that `sway-runtime` owns the editor viewport (camera, picking, gizmo input) in an editor-only plugin.

- [ ] **Step 4: Write the findings report**

Create `docs/superpowers/reports/2026-08-15-m7-viewport-interaction-findings.md`, following the shape of `2026-08-10-m6-editor-write-half-findings.md`: Question, Answer (with the real `cargo test --workspace` numbers and who walked the exit criterion), What was built (one bullet per task with its commit hash), Deviations from the spec, Surprises, What M8 inherits, What is not answered.

Items already known to belong in "What is not answered", to be confirmed or removed as the work actually goes:

- Whether the overlay camera's clear behaviour needed the `ClearColorConfig::None` fix (Task 13, Step 6).
- Which discriminator was used to keep the overlay camera out of `tag_scene_cameras` (Task 6).
- Whether `viewport_ray`'s unit tests could use a real camera or fell back to asserting on inputs (Task 11, Step 3).
- The M6-inherited items M7 deliberately did not touch: the disconnect gesture's press-side real-dispatch test, `FieldValue::Enum`'s missing coverage, the `SOCKET_RADIUS * 2.5` duplication, and the growth of `canvas.rs` / `snapshot.rs`.
- Everything in the spec's "Out of scope for M7".

- [ ] **Step 5: Final verification**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS, with a count at or above the Task 1 baseline plus this plan's new tests, and the same two intentional ignores.

- [ ] **Step 6: Commit**

```bash
git add -A docs/
git commit -m "docs: M7 findings, and the amendments it settles"
```

---

## Self-Review

**Spec coverage.** M7-1 → Tasks 1, 3. M7-2 → Task 2. M7-3 → Tasks 4, 5. M7-4 → Tasks 6, 7. M7-5 → Tasks 8, 9, 10. M7-6 → Tasks 11, 12. M7-7 → Task 15's Step 5 (the negative case is verified deliberately, since "does nothing visible" is the accepted behaviour). M7-8 → Tasks 13, 14, 15. The four verify-before-implementing items → Task 11 Step 1 (#1), Task 13 Step 1 (#2 and #3), Task 6 Step 1 (#4). The document amendments → Task 16.

**Two places this plan deliberately does not pin the code.** Task 14's `viewport_gizmo_hover` body and Task 15's `viewport_gizmo_drag` body are described as faithful ports of named, line-numbered private functions rather than transcribed. Transcribing ~240 lines of upstream source into a plan would be a copy that silently rots against the pinned crate, and the tests around them specify the behaviour that matters. Every substitution the port must make *is* enumerated.

**Known ordering dependency.** `pick_on_click` (Task 12) reads `TransformGizmoState` before it exists (Task 13) — hence the `Option<Res<..>>`. When Task 13 lands, that becomes a live guard. Both tasks' tests cover their own side, and Task 15's `a_drag_on_a_handle_does_not_also_select_something` covers the join.

**Type consistency check.** `ViewportInput`, `ViewportButton`, `ViewportModifiers`, `ViewportKey`, `ViewportInputRx`, `normalize_viewport_pos` (Task 1) are used under exactly those names in Tasks 2, 3, 5, 12, 14, 15. `ViewportEvents`, `ViewportSystems`, `EditorViewportPlugin` (Task 3) likewise in 5, 6, 12, 13, 14, 15. `EditorCamera`, `orbit_transform`, `orbit`, `pan`, `dolly`, `MIN_DISTANCE` (Task 4) in Task 5. `ViewportCamera`, `ViewportCameraRole` (Task 6) in Tasks 7, 12, 13. `Selection` and `EditorCommand::Select` (Task 8) in Tasks 9, 10, 12, 13. `viewport_ray` (Task 11) in Tasks 12, 14. `cursor_in_viewport_pixels` (Task 14) in Task 15. `HiddenFromEditor` is introduced in Task 13 Step 2 as an explicit decision with its consumer named in the same step.
