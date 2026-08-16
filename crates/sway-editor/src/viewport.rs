//! `Viewport` -- the Bevy viewport's seat in the widget tree, and the only
//! widget that forwards input into the Bevy world.
//!
//! Replaces M1b's `ViewportPlaceholder`, which declined pointer interaction
//! entirely. Everything about the *painting* half is unchanged and still
//! load-bearing: `PaintLayerMode::External` leaves the hole the compositor
//! fills, the `request_anim_frame` loop keeps this widget in every
//! `VisualLayerPlan` (masonry does not repaint idle widgets, and a one-shot
//! placeholder silently vanishes after the first frame), and
//! `EditorUi::viewport_rect` reads the rect off this widget's own bounding
//! box rather than off the layer plan -- see that method's doc comment.
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
    TextEvent, Update, UpdateCtx, Widget,
};
use masonry::dpi::{LogicalPosition, PhysicalPosition};
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
                let logical: LogicalPosition<f64> = physical.to_logical(scale);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{Receiver, Sender};
    use masonry::core::keyboard::{Code, Key, KeyboardEvent};
    use masonry::core::{
        DefaultProperties, Modifiers, PointerButton, PointerButtonEvent, PointerEvent,
        PointerState, TextEvent,
    };
    use masonry::dpi::PhysicalPosition;
    use masonry_core::app::VisualLayerKind;
    use masonry_testing::{PRIMARY_MOUSE, TestHarness};
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
        harness.mouse_button_press(Some(PointerButton::Primary));

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
        //
        // `TestHarness::mouse_button_press` drives its own internal
        // `PointerState`, which carries no modifiers, and there is no
        // `keyboard_key_down`/`alt_key` helper at this masonry revision that
        // would arrange Alt being held first. So this builds a real
        // `PointerEvent::Down` by hand, `PointerState.modifiers` set
        // directly -- the same idiom `canvas.rs`'s test module uses for
        // scroll and gesture events -- and drives it through
        // `process_pointer_event`, not a bypass.
        let (mut harness, rx) = harness();
        let state = PointerState {
            position: PhysicalPosition { x: 100.0, y: 50.0 },
            modifiers: Modifiers::ALT,
            ..Default::default()
        };
        harness.process_pointer_event(PointerEvent::Down(PointerButtonEvent {
            pointer: PRIMARY_MOUSE,
            button: Some(PointerButton::Primary),
            state,
        }));

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
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_button_release(Some(PointerButton::Primary));

        harness.process_text_event(TextEvent::Keyboard(KeyboardEvent::key_down(
            Key::Character("e".into()),
            Code::KeyE,
        )));

        assert!(
            rx.try_iter()
                .any(|e| e == ViewportInput::Key { key: ViewportKey::Rotate }),
            "E must reach the world as a rotate-mode key",
        );
    }

    #[test]
    fn scroll_reports_the_sign_dolly_treats_as_zooming_in() {
        // `dolly()` (`sway-runtime`, `camera.rs`) documents "positive
        // `amount` dollies in", and `navigate_editor_camera` feeds it
        // `delta.y * 0.05` straight from `ViewportInput::Scroll` -- so a
        // positive `Scroll.delta.y` out of this widget must mean "zoom in"
        // for the two to agree. Nothing in `on_pointer_event`'s `Scroll` arm
        // inverts the sign (`to_pixel_delta`/`to_logical` are unit
        // conversions only), so a `LineDelta` with a positive y -- one wheel
        // tick "forward", the same input `canvas.rs`'s own
        // `scroll_line_delta_zooms_dpi_invariantly` test drives -- must
        // arrive as a positive `delta.y`, not a flipped one.
        use masonry::core::{PointerScrollEvent, ScrollDelta};

        let (mut harness, rx) = harness();
        let state = PointerState {
            position: PhysicalPosition { x: 100.0, y: 50.0 },
            ..Default::default()
        };
        harness.process_pointer_event(PointerEvent::Scroll(PointerScrollEvent {
            pointer: PRIMARY_MOUSE,
            delta: ScrollDelta::LineDelta(0.0, 1.0),
            state,
        }));

        let Some(ViewportInput::Scroll { delta, .. }) =
            rx.try_iter().find(|e| matches!(e, ViewportInput::Scroll { .. }))
        else {
            panic!("no Scroll reached the channel");
        };
        assert!(delta.y > 0.0, "a forward wheel tick must forward a positive delta.y; got {}", delta.y);
    }

    #[test]
    fn pinch_reports_the_sign_dolly_treats_as_zooming_in() {
        // Same pipeline, the pinch gesture: `dolly()` gets `delta * 4.0`
        // straight from `ViewportInput::Pinch`, and `on_pointer_event`'s
        // `Gesture(Pinch)` arm casts the value through unchanged (`delta: *delta
        // as f32`), so a positive `PointerGesture::Pinch` -- fingers spreading
        // apart, the same sign `canvas.rs`'s own `pinch_zooms_about_the_cursor`
        // test treats as zooming in -- must arrive as a positive
        // `ViewportInput::Pinch::delta`.
        use masonry::core::{PointerGesture, PointerGestureEvent};

        let (mut harness, rx) = harness();
        let state = PointerState {
            position: PhysicalPosition { x: 100.0, y: 50.0 },
            ..Default::default()
        };
        harness.process_pointer_event(PointerEvent::Gesture(PointerGestureEvent {
            pointer: PRIMARY_MOUSE,
            gesture: PointerGesture::Pinch(0.1),
            state,
        }));

        let Some(ViewportInput::Pinch { delta }) =
            rx.try_iter().find(|e| matches!(e, ViewportInput::Pinch { .. }))
        else {
            panic!("no Pinch reached the channel");
        };
        assert!(delta > 0.0, "fingers spreading apart must forward a positive delta; got {delta}");
    }

    #[test]
    fn a_cancel_is_forwarded() {
        // A drag whose capture is lost must be abandoned world-side. M6 Task
        // 14 shipped a stuck rubber band by omitting exactly this.
        let (mut harness, rx) = harness();
        harness.mouse_move((100.0, 50.0));
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.process_pointer_event(PointerEvent::Cancel(PRIMARY_MOUSE));

        assert!(rx.try_iter().any(|e| e == ViewportInput::Cancel));
    }

    #[test]
    fn the_viewport_is_still_an_external_paint_layer() {
        // The compositor's hole. Accepting pointer interaction must not have
        // cost us the reason this widget exists.
        let (mut harness, _rx) = harness();
        // `TestHarness::create_with` already performs one internal redraw to
        // seed the access tree, and that redraw consumes `request_paint`
        // (masonry resets `PaintLayerMode` to `Inline` at the top of every
        // paint pass, only restoring it while `paint` itself runs -- see
        // this widget's module doc). The anim-frame request `update` made on
        // `WidgetAdded` arrived too late to be serviced by that same
        // construction-time pass, so a fresh `redraw()` here would otherwise
        // observe `Inline`. Pumping one anim frame first services the
        // pending request and re-marks the widget dirty, exactly what
        // `EditorUi::redraw` does for the real host.
        harness.animate_ms(0);
        let (plan, _tree_update) = harness.redraw();
        assert!(
            plan.layers
                .iter()
                .any(|layer| matches!(layer.kind, VisualLayerKind::External { .. })),
            "the viewport must still leave an External layer for the compositor",
        );
    }
}
