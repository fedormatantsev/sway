//! `Inspector` -- the selected entity's authored values, read-only.
//!
//! Rows are `Label` children for the same reason `SceneTree`'s are:
//! `imaging::Painter` takes only pre-shaped glyphs. Editing is M7; this pane
//! exists to prove the reflect walk and to surface types that still want
//! editor `TypeData`.

use crossbeam_channel::Sender;
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
use sway_graph::EditorCommand;

use crate::snapshot::WorldSnapshot;

pub const ROW_HEIGHT: f64 = 18.0;
const PADDING: f64 = 8.0;
const NATURAL_WIDTH: f64 = 240.0;

struct Row {
    pod: WidgetPod<Label>,
    /// Component headers are indented less than their fields.
    header: bool,
}

pub struct Inspector {
    rows: Vec<Row>,
    signature: Vec<String>,
    generation: u64,
    /// Unused until Task 8 wires up field editing.
    #[allow(dead_code)]
    commands: Sender<EditorCommand>,
}

impl Inspector {
    pub fn new(commands: Sender<EditorCommand>) -> Self {
        Self { rows: Vec::new(), signature: Vec::new(), generation: 0, commands }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `(text, is_header)` for the current selection.
    fn lines(snap: &WorldSnapshot) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        if snap.inspector.entity.is_none() {
            lines.push(("nothing selected".to_string(), true));
            return lines;
        }
        for component in &snap.inspector.components {
            lines.push((component.name.to_string(), true));
            for field in &component.fields {
                lines.push((format!("{}  {}", field.name, field.value), false));
            }
        }
        if lines.is_empty() {
            lines.push(("no authored components".to_string(), true));
        }
        lines
    }

    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let lines = Self::lines(snap);
        let signature: Vec<String> = lines.iter().map(|(text, _)| text.clone()).collect();
        if signature == this.widget.signature {
            return;
        }

        for row in std::mem::take(&mut this.widget.rows) {
            this.ctx.remove_child(row.pod);
        }
        for (text, header) in &lines {
            let pod = WidgetPod::new(Label::new(text.clone()));
            this.widget.rows.push(Row { pod, header: *header });
        }
        this.widget.signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    fn content_height(&self) -> f64 {
        self.rows.len() as f64 * ROW_HEIGHT
    }
}

impl Widget for Inspector {
    type Action = ();

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for row in &mut self.rows {
            ctx.register_child(&mut row.pod);
        }
    }

    /// Reports the full content height on `MaxContent`, which is what
    /// `Portal` asks for when deciding how far it can scroll (its `layout`
    /// calls `compute_size` with `LenDef::MaxContent` on any unconstrained
    /// axis).
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
            (Axis::Vertical, LenReq::MaxContent) => Length::const_px(self.content_height()),
            (Axis::Horizontal, LenReq::MaxContent) => Length::const_px(NATURAL_WIDTH),
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        for (index, row) in self.rows.iter_mut().enumerate() {
            let x = if row.header { PADDING } else { PADDING * 2.0 };
            let row_size = Size::new((size.width - x).max(0.0), ROW_HEIGHT);
            ctx.run_layout(&mut row.pod, row_size);
            ctx.place_child(&mut row.pod, Point::new(x, index as f64 * ROW_HEIGHT));
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Fills the background the same colour `SceneTree` uses for its header
    /// rows; the row text is each `Label` child's own job, painted after
    /// this.
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let rect = Rect::new(0.0, 0.0, NATURAL_WIDTH, self.content_height());
        painter.fill_rect(rect, Color::from_rgb8(44, 46, 54));
    }

    fn accessibility_role(&self) -> Role {
        Role::List
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

    fn children_ids(&self) -> ChildrenIds {
        self.rows.iter().map(|row| row.pod.id()).collect()
    }
}
