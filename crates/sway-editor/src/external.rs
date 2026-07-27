//! Finding the Bevy viewport's window-space rectangle in masonry's paint output.
//!
//! `VisualLayerKind::External` is masonry's placeholder for content a host
//! renders itself. Its `bounds` are in layer-local coordinates and the layer's
//! `transform` maps them into window space -- the same convention the scene
//! layers use, and the reason `replay_into` takes the transform rather than
//! baking it in.
//!
//! Upstream documents this mode as pre-integration ("current hosts do not
//! realize these placeholders yet"), so this module is the host integration it
//! is waiting for, in the narrow form M1b needs: exactly one external layer,
//! the Bevy viewport.

use kurbo::Rect;
use masonry_core::app::{VisualLayerKind, VisualLayerPlan};

/// The window-space rectangle of the first external layer, if any.
///
/// Returns `None` when the widget tree contains no external boundary -- which
/// is a legitimate state (the show presenter, or an editor layout with the
/// viewport collapsed), not an error (controller dispatch ruling R2). The
/// caller draws no viewport quad.
///
/// If several external layers exist, the first one wins (controller dispatch
/// ruling R3) -- not an error, not a merge.
pub fn viewport_rect(plan: &VisualLayerPlan) -> Option<Rect> {
    plan.layers.iter().find_map(|layer| match layer.kind {
        // R1 (controller dispatch ruling): `transform_rect_bbox`, not two
        // hand-transformed corners. Under a rotation the transformed rect is
        // not axis-aligned, and the bounding box is the only honest answer.
        // A rotated viewport is not supported and does not need to be.
        VisualLayerKind::External { bounds } => Some(layer.transform.transform_rect_bbox(bounds)),
        VisualLayerKind::Scene(_) => None,
    })
}

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, NoAction, PaintCtx, PropertiesMut, PropertiesRef,
    RegisterCtx, Update, UpdateCtx, Widget,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry_core::kurbo::{Axis, Size};

/// A leaf widget that paints nothing itself and instead marks its subtree as
/// an external paint layer (`PaintLayerMode::External`) -- masonry's
/// placeholder for content a host renders outside masonry's own scene graph.
/// This is the Bevy viewport's seat in the widget tree: its layout box is
/// exactly the rectangle the Bevy-rendered point cloud should occupy on
/// screen, and [`viewport_rect`] reads that rectangle back out of the
/// [`VisualLayerPlan`](masonry_core::app::VisualLayerPlan) `redraw()`
/// produces.
///
/// # Continuous repaint (a real gap in `External`, not a hypothetical one)
///
/// `PaintCtx::set_paint_layer_mode` resets to `Inline` at the start of
/// *every* paint pass for a widget, and is only set back to `External` while
/// that widget's own `paint` method actually runs. `paint` only runs when the
/// widget has a pending paint request; masonry does not repaint idle widgets
/// on every frame just because the host calls `redraw()` again. A one-shot
/// placeholder would therefore appear in the very first `VisualLayerPlan`
/// (new widgets start dirty) and then silently vanish from every plan after
/// that, taking the viewport rect with it.
///
/// This widget keeps itself dirty forever by requesting an animation frame
/// on creation and re-requesting one (plus a repaint) every time one fires,
/// the same mechanism `masonry::widgets::Spinner` uses to animate. See
/// `EditorUi::redraw` for the other half: it pumps a `WindowEvent::AnimFrame`
/// ahead of every real `redraw()` call so the request is actually serviced,
/// since this host does not otherwise drive masonry's animation clock.
pub struct ViewportPlaceholder;

impl ViewportPlaceholder {
    /// Returns a fresh widget, ready for `.prepare()` (the usual
    /// `Widget::prepare` convenience, wrapping it in a `NewWidget`) and
    /// `.with_props(Dimensions::fixed(..))` to give it a real size.
    pub fn new() -> Self {
        Self
    }
}

impl Widget for ViewportPlaceholder {
    type Action = NoAction;

    fn register_children(&mut self, _ctx: &mut RegisterCtx<'_>) {}

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::WidgetAdded = event {
            ctx.request_anim_frame();
        }
    }

    fn on_anim_frame(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, _interval: u64) {
        // Keep the loop going and force another `paint` call so
        // `PaintLayerMode::External` gets re-asserted every frame.
        ctx.request_anim_frame();
        ctx.request_paint_only();
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        _axis: Axis,
        len_req: LenReq,
        cross_length: Option<Length>,
    ) -> Length {
        // Unreachable in practice: the widget is always given explicit
        // `Dimensions`, and per `Widget::measure`'s own docs that resolves
        // before `measure` would ever be called. Kept total and harmless
        // regardless.
        match len_req {
            LenReq::FitContent(space) => space,
            LenReq::MinContent | LenReq::MaxContent => cross_length.unwrap_or(Length::ZERO),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        // Matches `masonry/src/tests/paint.rs`'s only example of
        // `PaintLayerMode::External` (R4, controller dispatch ruling):
        // the placeholder clips to its full content box.
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
    use super::viewport_rect;
    use kurbo::{Affine, Rect};
    use masonry_core::app::{VisualLayer, VisualLayerKind, VisualLayerPlan};
    use masonry_core::core::{NewWidget, WidgetId};
    use masonry::widgets::Label;

    // Deviation from the brief: `WidgetId::next()` is `pub(crate)` in the
    // pinned masonry rev (c5950bc), not public as the brief's test code
    // assumed. The only public way to mint a `WidgetId` from outside the
    // masonry_core crate is to build a real widget and read its id back off
    // `NewWidget`.
    fn dummy_widget_id() -> WidgetId {
        NewWidget::new(Label::new("")).id()
    }

    fn plan(layers: Vec<VisualLayer>) -> VisualLayerPlan {
        VisualLayerPlan { layers }
    }

    fn external(bounds: Rect, transform: Affine) -> VisualLayer {
        VisualLayer { kind: VisualLayerKind::External { bounds }, transform, widget_id: dummy_widget_id() }
    }

    #[test]
    fn none_when_no_external_layer() {
        assert_eq!(viewport_rect(&plan(vec![])), None);
    }

    #[test]
    fn identity_transform_returns_bounds_unchanged() {
        let p = plan(vec![external(Rect::new(10.0, 20.0, 110.0, 80.0), Affine::IDENTITY)]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(10.0, 20.0, 110.0, 80.0)));
    }

    #[test]
    fn translation_moves_the_rect_into_window_space() {
        let p = plan(vec![external(
            Rect::new(0.0, 0.0, 100.0, 60.0),
            Affine::translate((25.0, 45.0)),
        )]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(25.0, 45.0, 125.0, 105.0)));
    }

    #[test]
    fn scale_and_translation_compose() {
        // Layer-local (0,0)-(100,60), scaled 2x then translated by (10,10).
        let p = plan(vec![external(
            Rect::new(0.0, 0.0, 100.0, 60.0),
            Affine::translate((10.0, 10.0)) * Affine::scale(2.0),
        )]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(10.0, 10.0, 210.0, 130.0)));
    }

    #[test]
    fn first_external_layer_wins_when_several_exist() {
        let p = plan(vec![
            external(Rect::new(0.0, 0.0, 10.0, 10.0), Affine::IDENTITY),
            external(Rect::new(50.0, 50.0, 60.0, 60.0), Affine::IDENTITY),
        ]);
        assert_eq!(viewport_rect(&p), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }
}
