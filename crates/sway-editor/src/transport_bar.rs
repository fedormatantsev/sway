//! `TransportBar` — the transport readout strip.
//!
//! M2c deliberately shipped no transport display, because inventing one
//! before the thing it displays is backwards. This is M3's fourth consumer of
//! the same per-frame `capture(&World)` snapshot, alongside the scene tree,
//! the viewport and the graph canvas.
//!
//! Children are `Label`s rather than painted text, for the reason `SceneTree`
//! gives: `imaging::Painter` takes only pre-shaped glyphs. Rows are rebuilt
//! only when the text actually changes, so a steady transport costs one
//! comparison per frame — and at 120 BPM the position field changes several
//! times a second, so that comparison is what stops this widget rebuilding
//! the world.

use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, LayoutCtx, MeasureCtx, PaintCtx,
    PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetId, WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::{Button, ButtonPress, Label};
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

use crate::{FileRequest, ViewRequest};

/// Height of the strip, in logical pixels.
pub const TRANSPORT_BAR_HEIGHT: f64 = 24.0;
/// Left padding and the gap between fields.
const PADDING: f64 = 12.0;
/// Fixed column width per field, so the position does not jitter the layout
/// four times a beat.
const FIELD_WIDTH: f64 = 120.0;
/// Fixed column width per file button.
const BUTTON_WIDTH: f64 = 72.0;

/// The transport readout.
pub struct TransportBar {
    labels: Vec<WidgetPod<Label>>,
    fields: Vec<String>,
    generation: u64,
    playing: bool,
    /// Open / Save / Camera, in that order. Built once; never rebuilt by a
    /// readout.
    buttons: [WidgetPod<Button>; 3],
    /// What the toolbar has asked for since the shell last drained it.
    requests: Vec<FileRequest>,
    /// What the toolbar has asked for since the shell last drained it.
    view_requests: Vec<ViewRequest>,
}

impl Default for TransportBar {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportBar {
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            fields: Vec::new(),
            generation: 0,
            playing: false,
            buttons: [
                WidgetPod::new(Button::with_text("Open")),
                WidgetPod::new(Button::with_text("Save")),
                WidgetPod::new(Button::with_text("Camera")),
            ],
            requests: Vec::new(),
            view_requests: Vec::new(),
        }
    }

    /// The three field strings, in display order. Exposed for tests.
    pub fn fields(&self) -> Vec<String> {
        self.fields.clone()
    }

    /// How many times the fields have actually been rebuilt.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn open_button_id(&self) -> WidgetId {
        self.buttons[0].id()
    }

    pub fn save_button_id(&self) -> WidgetId {
        self.buttons[1].id()
    }

    pub fn camera_button_id(&self) -> WidgetId {
        self.buttons[2].id()
    }
}

// --- MARK: WIDGETMUT
impl TransportBar {
    ///
    /// The graph model has no snapshot to carry the transport along with, and
    /// the transport was never graph state in the first place: it is a
    /// `sway-midi` resource the presenter reads at step 0 alongside `&Graph`.
    pub fn apply_transport(this: &mut WidgetMut<'_, Self>, transport: &sway_midi::Transport) {
        let fields = vec![
            if transport.playing { "PLAY" } else { "STOP" }.to_string(),
            if transport.locked {
                format!("{:.1} BPM", transport.bpm)
            } else {
                format!("~{:.1} BPM", transport.bpm)
            },
            sway_midi::MusicalTime::from_ppq(transport.ppq, transport.beats_per_bar).to_string(),
        ];
        this.widget.playing = transport.playing;
        if fields == this.widget.fields {
            return;
        }

        for label in this.widget.labels.drain(..) {
            this.ctx.remove_child(label);
        }
        for field in &fields {
            this.widget
                .labels
                .push(Label::new(field.clone()).prepare().to_pod());
        }

        this.widget.fields = fields;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    /// Drains what the toolbar has asked for. Called once per frame by the
    /// shell, through `EditorUi::take_file_requests`.
    pub fn take_file_requests(this: &mut WidgetMut<'_, Self>) -> Vec<FileRequest> {
        std::mem::take(&mut this.widget.requests)
    }

    /// Drains what the toolbar has asked for. Called once per frame by the
    /// shell, through `EditorUi::take_view_requests`.
    pub fn take_view_requests(this: &mut WidgetMut<'_, Self>) -> Vec<ViewRequest> {
        std::mem::take(&mut this.widget.view_requests)
    }
}

impl Widget for TransportBar {
    type Action = ();

    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        if action.downcast_ref::<ButtonPress>().is_none() {
            return;
        }
        match self.buttons.iter().position(|b| b.id() == source) {
            Some(0) => {
                self.requests.push(FileRequest::Open);
                ctx.set_handled();
            }
            Some(1) => {
                self.requests.push(FileRequest::Save);
                ctx.set_handled();
            }
            Some(2) => {
                self.view_requests.push(ViewRequest::ToggleCamera);
                ctx.set_handled();
            }
            _ => {}
        }
    }

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for label in &mut self.labels {
            ctx.register_child(label);
        }
        for button in &mut self.buttons {
            ctx.register_child(button);
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
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(TRANSPORT_BAR_HEIGHT),
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(
                PADDING
                    + self.labels.len() as f64 * FIELD_WIDTH
                    + self.buttons.len() as f64 * BUTTON_WIDTH,
            ),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, label) in self.labels.iter_mut().enumerate() {
            let x = PADDING + index as f64 * FIELD_WIDTH;
            ctx.run_layout(label, Size::new(FIELD_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(label, Point::new(x, 0.0));
        }
        let buttons_start = PADDING + self.labels.len() as f64 * FIELD_WIDTH;
        for (index, button) in self.buttons.iter_mut().enumerate() {
            let x = buttons_start + index as f64 * BUTTON_WIDTH;
            ctx.run_layout(button, Size::new(BUTTON_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(button, Point::new(x, 0.0));
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        painter.fill_rect(
            Rect::new(0.0, 0.0, 4000.0, TRANSPORT_BAR_HEIGHT),
            Color::from_rgb8(30, 32, 38),
        );
        // A one-pixel accent under the state field, green while playing. The
        // strip has to be readable from across a room during a soundcheck.
        painter.fill_rect(
            Rect::new(
                0.0,
                TRANSPORT_BAR_HEIGHT - 2.0,
                PADDING,
                TRANSPORT_BAR_HEIGHT,
            ),
            if self.playing {
                Color::from_rgb8(90, 200, 120)
            } else {
                Color::from_rgb8(90, 92, 100)
            },
        );
    }

    fn accessibility_role(&self) -> Role {
        Role::Label
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        self.labels
            .iter()
            .map(|label| label.id())
            .chain(self.buttons.iter().map(|button| button.id()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::core::DefaultProperties;
    use masonry_testing::TestHarness;

    fn transport(playing: bool, bpm: f64, ppq: f64, locked: bool) -> sway_midi::Transport {
        sway_midi::Transport {
            playing,
            bpm,
            ppq,
            locked,
            beats_per_bar: 4,
        }
    }

    fn harness_with(t: &sway_midi::Transport) -> TestHarness<TransportBar> {
        // Wider than `TestHarnessParams::DEFAULT_SIZE` (400px): three
        // `FIELD_WIDTH` columns plus three `BUTTON_WIDTH` buttons plus
        // `PADDING`. A strip clipped narrower than its own content is also
        // unclickable in the harness.
        let mut harness = TestHarness::create_with_size(
            DefaultProperties::default(),
            TransportBar::new().prepare(),
            (772, 100),
        );
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_transport(&mut bar, t);
        });
        harness
    }

    #[test]
    fn a_playing_transport_reads_out_state_tempo_and_position() {
        let harness = harness_with(&transport(true, 128.0, 17.5, true));
        assert_eq!(
            harness.root_widget().fields(),
            vec![
                "PLAY".to_string(),
                "128.0 BPM".to_string(),
                "005.2.3".to_string()
            ]
        );
    }

    #[test]
    fn a_stopped_transport_says_so() {
        let harness = harness_with(&transport(false, 120.0, 0.0, false));
        assert_eq!(harness.root_widget().fields()[0], "STOP");
    }

    #[test]
    fn freewheeling_is_distinguishable_from_locked() {
        // A performer needs to know the clock is gone before they wonder why
        // the visuals are sliding.
        let locked = harness_with(&transport(true, 120.0, 0.0, true));
        let free = harness_with(&transport(true, 120.0, 0.0, false));
        assert_ne!(
            locked.root_widget().fields()[1],
            free.root_widget().fields()[1]
        );
    }

    #[test]
    fn an_unchanged_readout_rebuilds_nothing() {
        let t = transport(true, 120.0, 0.0, true);
        let mut harness = harness_with(&t);
        let before = harness.root_widget().generation();
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_transport(&mut bar, &t);
        });
        assert_eq!(harness.root_widget().generation(), before);
    }

    #[test]
    fn the_save_button_emits_a_save_request() {
        use crate::FileRequest;
        let mut harness = harness_with(&transport(false, 120.0, 0.0, true));
        let save_id = harness.root_widget().save_button_id();

        harness.mouse_click_on(save_id, Some(masonry::core::PointerButton::Primary));

        harness.edit_root_widget(|mut bar| {
            assert_eq!(
                TransportBar::take_file_requests(&mut bar),
                vec![FileRequest::Save],
            );
        });
    }

    #[test]
    fn taking_the_requests_drains_them() {
        use crate::FileRequest;
        let mut harness = harness_with(&transport(false, 120.0, 0.0, true));
        let open_id = harness.root_widget().open_button_id();

        harness.mouse_click_on(open_id, Some(masonry::core::PointerButton::Primary));

        harness.edit_root_widget(|mut bar| {
            assert_eq!(
                TransportBar::take_file_requests(&mut bar),
                vec![FileRequest::Open],
            );
            assert!(
                TransportBar::take_file_requests(&mut bar).is_empty(),
                "the shell must not act on the same request twice",
            );
        });
    }

    #[test]
    fn the_camera_button_asks_the_shell_to_toggle() {
        use crate::ViewRequest;
        let mut harness = harness_with(&transport(false, 120.0, 0.0, true));
        let camera_id = harness.root_widget().camera_button_id();

        harness.mouse_click_on(camera_id, Some(masonry::core::PointerButton::Primary));

        harness.edit_root_widget(|mut bar| {
            assert_eq!(
                TransportBar::take_view_requests(&mut bar),
                vec![ViewRequest::ToggleCamera],
            );
        });
    }
}
