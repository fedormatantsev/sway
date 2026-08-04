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
    AccessCtx, ChildrenIds, LayoutCtx, MeasureCtx, PaintCtx, PropertiesRef, RegisterCtx, Widget,
    WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;

use crate::snapshot::WorldSnapshot;

/// Height of the strip, in logical pixels.
pub const TRANSPORT_BAR_HEIGHT: f64 = 24.0;
/// Left padding and the gap between fields.
const PADDING: f64 = 12.0;
/// Fixed column width per field, so the position does not jitter the layout
/// four times a beat.
const FIELD_WIDTH: f64 = 120.0;

/// The transport readout.
pub struct TransportBar {
    labels: Vec<WidgetPod<Label>>,
    fields: Vec<String>,
    generation: u64,
    playing: bool,
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
}

/// The three strings a snapshot displays as.
///
/// A freewheeling transport says so in the tempo field rather than in a
/// fourth one: a performer needs to know the clock is gone *before* they
/// wonder why the visuals are sliding, and a `~` prefix reads at a glance.
fn fields_of(snap: &WorldSnapshot) -> Vec<String> {
    let transport = &snap.transport;
    vec![
        if transport.playing { "PLAY" } else { "STOP" }.to_string(),
        if transport.locked {
            format!("{:.1} BPM", transport.bpm)
        } else {
            format!("~{:.1} BPM", transport.bpm)
        },
        transport.position.clone(),
    ]
}

// --- MARK: WIDGETMUT
impl TransportBar {
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let fields = fields_of(snap);
        this.widget.playing = snap.transport.playing;
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
}

impl Widget for TransportBar {
    type Action = ();

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for label in &mut self.labels {
            ctx.register_child(label);
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
            (Axis::Horizontal, LenReq::MaxContent) => {
                Length::const_px(PADDING + self.labels.len() as f64 * FIELD_WIDTH)
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, label) in self.labels.iter_mut().enumerate() {
            let x = PADDING + index as f64 * FIELD_WIDTH;
            ctx.run_layout(label, Size::new(FIELD_WIDTH, TRANSPORT_BAR_HEIGHT));
            ctx.place_child(label, Point::new(x, 0.0));
        }
        ctx.set_clip_path(size.to_rect());
    }

    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        painter.fill_rect(
            Rect::new(0.0, 0.0, 4000.0, TRANSPORT_BAR_HEIGHT),
            Color::from_rgb8(30, 32, 38),
        );
        // A one-pixel accent under the state field, green while playing. The
        // strip has to be readable from across a room during a soundcheck.
        painter.fill_rect(
            Rect::new(0.0, TRANSPORT_BAR_HEIGHT - 2.0, PADDING, TRANSPORT_BAR_HEIGHT),
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

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        self.labels.iter().map(|label| label.id()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{TransportView, WorldSnapshot};
    use masonry::core::DefaultProperties;
    use masonry_testing::TestHarness;

    fn snapshot(playing: bool, bpm: f32, position: &str, locked: bool) -> WorldSnapshot {
        WorldSnapshot {
            transport: TransportView {
                playing,
                bpm,
                position: position.to_string(),
                locked,
            },
            ..Default::default()
        }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<TransportBar> {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), TransportBar::new().prepare());
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_snapshot(&mut bar, &snap);
        });
        harness
    }

    #[test]
    fn a_playing_transport_reads_out_state_tempo_and_position() {
        let harness = harness_with(snapshot(true, 128.02, "005.3.2", true));
        assert_eq!(
            harness.root_widget().fields(),
            vec!["PLAY".to_string(), "128.0 BPM".to_string(), "005.3.2".to_string()]
        );
    }

    #[test]
    fn a_stopped_transport_says_so() {
        let harness = harness_with(snapshot(false, 120.0, "001.1.1", false));
        assert_eq!(harness.root_widget().fields()[0], "STOP");
    }

    #[test]
    fn freewheeling_is_distinguishable_from_locked() {
        // A performer needs to know the clock is gone before they wonder why
        // the visuals are sliding.
        let locked = harness_with(snapshot(true, 120.0, "001.1.1", true));
        let free = harness_with(snapshot(true, 120.0, "001.1.1", false));
        assert_ne!(locked.root_widget().fields()[1], free.root_widget().fields()[1]);
    }

    #[test]
    fn an_unchanged_snapshot_rebuilds_nothing() {
        let snap = snapshot(true, 120.0, "001.1.1", true);
        let mut harness = harness_with(snap.clone());
        let before = harness.root_widget().generation();
        harness.edit_root_widget(|mut bar| {
            TransportBar::apply_snapshot(&mut bar, &snap);
        });
        assert_eq!(harness.root_widget().generation(), before);
    }
}
