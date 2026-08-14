//! The component palette: a filterable list of every authorable component,
//! opened by right-clicking the graph canvas. Spec M6, "Palette".
//!
//! A masonry `Layer`, modelled on `masonry::layers::SelectorMenu`: it dismisses
//! itself on a press outside its own border box, which is the behaviour every
//! popup in the pinned checkout has and the one users expect.
//!
//! It is handed a list of names by `WorldSnapshot.palette` and reports the
//! one that was picked; `GraphCanvas` turns that into an
//! `EditorCommand::Create` (Task 13). Reporting back is the other half of
//! the `SelectorMenu` model: `ctx.create_layer` (masonry_core) makes `Palette`
//! a *sibling* of the widget that opened it under masonry's internal
//! `LayerStack`, not that widget's descendant, so `EventCtx::submit_action`'s
//! ordinary parent-bubbling dead-ends at `LayerStack` and never reaches back.
//! `SelectorMenu` solves this by storing its opener's `WidgetId` and using
//! `ctx.mutate_later` (which targets a `WidgetId` directly, regardless of
//! tree position) instead of `submit_action`; `with_creator` below is that
//! same fix, and is what makes `GraphCanvas::finish_palette_pick` reachable
//! from a real pick. A `Palette` built without a creator (this module's own
//! tests) keeps reporting through `PaletteAction::Picked` for standalone
//! testability.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, EventCtx, Layer, LayoutCtx, MeasureCtx,
    PaintCtx, PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget,
    WidgetId, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::{Button, ButtonPress, TextAction, TextInput};
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

/// Height of the filter box and of each listed row, in logical pixels.
const ROW_HEIGHT: f64 = 22.0;
/// The palette's fixed width.
const WIDTH: f64 = 200.0;
/// At most this many rows are listed; the filter is how you reach the rest.
/// Without a cap, a registry of forty components would open a popup taller
/// than the window.
const MAX_ROWS: usize = 12;

/// A component type was picked from the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    Picked(&'static str),
}

pub struct Palette {
    /// Every authorable name, unfiltered, in registry order.
    names: Vec<&'static str>,
    filter: String,
    input: WidgetPod<TextInput>,
    /// The `TextArea` inside `input` — the child that actually submits the
    /// text actions, per `TextInput`'s own docs.
    input_area: WidgetId,
    /// One button per currently visible name, paired with the name it picks.
    /// Paired rather than re-derived, so a click can never be resolved against
    /// a filter that changed in between.
    rows: Vec<(&'static str, WidgetPod<Button>)>,
    /// The `GraphCanvas` this palette reports its pick back to, if any. See
    /// the module doc for why a plain `submit_action` can't reach it.
    creator: Option<WidgetId>,
}

// --- MARK: BUILDERS
impl Palette {
    pub fn new(names: Vec<&'static str>) -> Self {
        let input = TextInput::new("").with_placeholder("filter…");
        let input_area = input.area_pod().id();
        let mut palette = Self {
            names,
            filter: String::new(),
            input: WidgetPod::new(input),
            input_area,
            rows: Vec::new(),
            creator: None,
        };
        palette.rebuild_rows();
        palette
    }

    /// Sets the widget a pick is reported back to via `ctx.mutate_later`
    /// instead of `PaletteAction::Picked`. See the module doc.
    pub fn with_creator(mut self, creator: WidgetId) -> Self {
        self.creator = Some(creator);
        self
    }
}

// --- MARK: METHODS
impl Palette {
    /// The names matching the current filter, in registry order, capped at
    /// [`MAX_ROWS`].
    ///
    /// Case-insensitive substring, not fuzzy: `"ma"` finds `Math` and not
    /// `MeshAsset`. A fuzzy matcher would be nicer and is not what this is.
    pub fn visible(&self) -> Vec<&'static str> {
        let needle = self.filter.trim().to_lowercase();
        self.names
            .iter()
            .copied()
            .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
            .take(MAX_ROWS)
            .collect()
    }

    /// Sets the filter and rebuilds the row list. Pure state; the widget-tree
    /// side of the same change is [`apply_filter`](Self::apply_filter).
    pub fn set_filter(&mut self, filter: &str) {
        if self.filter == filter {
            return;
        }
        filter.clone_into(&mut self.filter);
        self.rebuild_rows();
    }

    /// The `WidgetId` of the `idx`th visible row, for tests and for the
    /// canvas's own assertions.
    pub fn row_id(&self, idx: usize) -> Option<WidgetId> {
        self.rows.get(idx).map(|(_, pod)| pod.id())
    }

    /// The filter box's `TextArea` id -- the one that actually receives
    /// keyboard focus and text events, per `TextInput`'s own docs. Reached by
    /// `EditorUi::drain_signals` once this palette's layer has actually been
    /// added to the tree, so it can be focused immediately (see that call
    /// site's doc comment for why the focus request can't happen any
    /// earlier).
    pub fn input_area_id(&self) -> WidgetId {
        self.input_area
    }

    fn rebuild_rows(&mut self) {
        self.rows = self
            .visible()
            .into_iter()
            .map(|name| (name, WidgetPod::new(Button::with_text(name))))
            .collect();
    }

    fn content_height(&self) -> f64 {
        (self.rows.len() + 1) as f64 * ROW_HEIGHT
    }
}

// --- MARK: WIDGETMUT
impl Palette {
    /// Sets the filter from outside the widget, telling masonry the child set
    /// changed. `set_filter` alone cannot do that — it has no context.
    pub fn apply_filter(this: &mut WidgetMut<'_, Self>, filter: &str) {
        if this.widget.filter == filter {
            return;
        }
        for (_, pod) in std::mem::take(&mut this.widget.rows) {
            this.ctx.remove_child(pod);
        }
        filter.clone_into(&mut this.widget.filter);
        this.widget.rebuild_rows();
        this.ctx.children_changed();
        this.ctx.request_layout();
    }
}

// --- MARK: IMPL WIDGET
impl Widget for Palette {
    type Action = PaletteAction;

    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        // A row was clicked: report the name paired with *that* pod, so the
        // answer cannot drift from what the user saw.
        if action.downcast_ref::<ButtonPress>().is_some()
            && let Some((name, _)) = self.rows.iter().find(|(_, pod)| pod.id() == source)
        {
            let name = *name;
            match self.creator {
                // `mutate_later` targets `creator` by `WidgetId` directly, so
                // this reaches `GraphCanvas` even though it is not an
                // ancestor of this layer -- see the module doc.
                Some(creator) => {
                    ctx.mutate_later(creator, move |mut target| {
                        let mut canvas = target.downcast::<crate::canvas::GraphCanvas>();
                        crate::canvas::GraphCanvas::finish_palette_pick(&mut canvas, name);
                    });
                }
                None => ctx.submit_action::<Self::Action>(PaletteAction::Picked(name)),
            }
            ctx.set_handled();
            return;
        }

        // The filter box changed. `Changed` (per keystroke) is the right
        // signal here, unlike in the inspector: filtering is free and
        // incremental, and waiting for Enter would make the box feel dead.
        if source == self.input_area
            && let Some(TextAction::Changed(text)) = action.downcast_ref::<TextAction>()
        {
            let text = text.clone();
            let id = ctx.widget_id();
            ctx.mutate_later(id, move |mut palette| {
                let mut palette = palette.downcast::<Self>();
                Self::apply_filter(&mut palette, &text);
            });
            ctx.set_handled();
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        ctx.register_child(&mut self.input);
        for (_, pod) in &mut self.rows {
            ctx.register_child(pod);
        }
    }

    fn measure(
        &mut self,
        _ctx: &mut MeasureCtx<'_>,
        _props: &PropertiesRef<'_>,
        axis: Axis,
        len_req: LenReq,
        _cross_length: Option<Length>,
    ) -> Length {
        match (axis, len_req) {
            (_, LenReq::FitContent(space)) => space,
            (_, LenReq::MinContent) => Length::ZERO,
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(WIDTH),
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(self.content_height()),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let row_size = Size::new(size.width, ROW_HEIGHT);
        ctx.run_layout(&mut self.input, row_size);
        ctx.place_child(&mut self.input, Point::ZERO);
        for (index, (_, pod)) in self.rows.iter_mut().enumerate() {
            ctx.run_layout(pod, row_size);
            ctx.place_child(pod, Point::new(0.0, (index + 1) as f64 * ROW_HEIGHT));
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        // An opaque backing, or the canvas and its edges show through the
        // gaps between the children.
        painter.fill_rect(
            Rect::new(0.0, 0.0, WIDTH, self.content_height()),
            Color::from_rgb8(44, 46, 54),
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::ListBox
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        std::iter::once(self.input.id())
            .chain(self.rows.iter().map(|(_, pod)| pod.id()))
            .collect()
    }

    /// Required for `ctx.create_layer` to accept this widget at all — it
    /// `debug_panic!`s when `as_layer` returns `None`.
    fn as_layer(&mut self) -> Option<&mut dyn Layer> {
        Some(self)
    }
}

// --- MARK: IMPL LAYER
impl Layer for Palette {
    /// Dismisses on a press outside the palette, exactly as `SelectorMenu`
    /// does. `capture_pointer_event` sees *every* pointer event in the window,
    /// including ones that never reach this widget's own hit box, which is why
    /// this is the layer hook rather than `on_pointer_event`.
    fn capture_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        let dismiss = match event {
            PointerEvent::Down(PointerButtonEvent { state, .. }) => {
                !ctx.border_box().contains(ctx.local_position(state.position))
            }
            PointerEvent::Cancel(..) => true,
            _ => false,
        };
        if dismiss {
            ctx.remove_layer(ctx.widget_id());
            // `SelectorMenu` clears its opener's bookkeeping on dismiss the
            // same way it does on pick (see its `capture_pointer_event`);
            // without this, `GraphCanvas::palette_layer` keeps the removed
            // layer's id and canvas position until the next right-click
            // silently overwrites it, and `palette_layer_id()` misreports
            // the palette as still open in the meantime.
            if let Some(creator) = self.creator {
                ctx.mutate_later(creator, move |mut target| {
                    let mut canvas = target.downcast::<crate::canvas::GraphCanvas>();
                    crate::canvas::GraphCanvas::dismiss_palette(&mut canvas);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::core::{DefaultProperties, PointerButton};
    use masonry_testing::TestHarness;

    fn names() -> Vec<&'static str> {
        vec!["Lfo", "Math", "MeshAsset", "DirectionalLight", "FloatOut"]
    }

    #[test]
    fn an_empty_filter_lists_everything() {
        let palette = Palette::new(names());
        assert_eq!(palette.visible(), names());
    }

    #[test]
    fn the_filter_is_a_case_insensitive_substring_match() {
        // The rule, stated once: lowercase both sides and ask for `contains`.
        // "ma" is inside "Math" and is *not* inside "MeshAsset" — the letters
        // are there but not adjacent, and this is not a fuzzy matcher.
        let mut palette = Palette::new(names());
        palette.set_filter("ma");
        assert_eq!(palette.visible(), vec!["Math"]);

        // Case-insensitive in both directions.
        palette.set_filter("MESH");
        assert_eq!(palette.visible(), vec!["MeshAsset"]);
    }

    #[test]
    fn the_filter_matches_anywhere_in_the_name_not_just_the_start() {
        let mut palette = Palette::new(names());
        palette.set_filter("light");
        assert_eq!(palette.visible(), vec!["DirectionalLight"]);
    }

    #[test]
    fn a_filter_matching_nothing_lists_nothing() {
        let mut palette = Palette::new(names());
        palette.set_filter("zzz");
        assert!(palette.visible().is_empty());
    }

    #[test]
    fn picking_a_row_emits_that_components_name() {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Palette::new(names()).prepare());
        let row_id = harness.root_widget().row_id(0).expect("five rows are listed");

        harness.mouse_click_on(row_id, Some(PointerButton::Primary));

        assert_eq!(
            harness.pop_action::<PaletteAction>().map(|(action, _)| action),
            Some(PaletteAction::Picked("Lfo")),
        );
    }

    #[test]
    fn picking_addresses_the_filtered_row_not_the_underlying_one() {
        // The defect this guards against: indexing the pick into `names`
        // rather than into `visible()`, so filtering to "FloatOut" and clicking
        // the only row would create an `Lfo`.
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Palette::new(names()).prepare());
        harness.edit_root_widget(|mut palette| {
            Palette::apply_filter(&mut palette, "float");
        });
        let row_id = harness.root_widget().row_id(0).expect("one row survives the filter");

        harness.mouse_click_on(row_id, Some(PointerButton::Primary));

        assert_eq!(
            harness.pop_action::<PaletteAction>().map(|(action, _)| action),
            Some(PaletteAction::Picked("FloatOut")),
        );
    }
}
