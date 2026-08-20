//! `SceneTree` -- the world hierarchy pane.
//!
//! Enumerates every entity in the world, grouped into Scene / Graph / Edges /
//! Other with a header per section (design §8). Rows are `Label` children
//! rather than painted text, for the same reason `NodeBox` uses one:
//! `imaging::Painter` takes only pre-shaped glyphs.
//!
//! The row set is rebuilt only when it differs from the previous frame, so a
//! steady-state world costs one comparison. `Portal` (Task 7) supplies
//! scrolling; this widget reports its full content height through `measure`
//! so `Portal` knows how far it can scroll. If measured entity counts ever
//! make the rebuild comparison too slow, `VirtualScroll` is the escape hatch
//! -- measure before reaching for it.

use bevy_ecs::entity::Entity;
use crossbeam_channel::Sender;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetId,
    WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;
use sway_graph::graph::{Graph, NodeId as GraphNodeId};

use crate::edit::EditorEdit;

use crate::reflect_ui::short_type_name;
use crate::views::NodeId;

/// Height of one row, in logical pixels.
pub const ROW_HEIGHT: f64 = 20.0;
/// Horizontal indent per depth level.
const INDENT: f64 = 14.0;
/// Left padding before the first indent level.
const PADDING: f64 = 8.0;
/// Natural width reported when nothing constrains this widget.
const NATURAL_WIDTH: f64 = 240.0;

/// What a [`SceneTree`] reports upward when a row is pressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneTreeAction {
    pub entity: Entity,
    /// `Some` when the row is a graph node, which is what lets a tree
    /// selection highlight a node box in the canvas.
    pub node_id: Option<NodeId>,
}

/// One laid-out row: either a section header or an entity.
struct Row {
    pod: WidgetPod<Label>,
    depth: usize,
    /// `None` for a section header, which is not selectable.
    entity: Option<Entity>,
    node_id: Option<NodeId>,
    /// The graph node this row stands for, when the tree is driven by the
    /// graph model. `None` for headers and for snapshot-driven rows.
    graph_node: Option<GraphNodeId>,
}

/// The world hierarchy pane.
pub struct SceneTree {
    rows: Vec<Row>,
    /// Bumped on every actual rebuild; lets a test assert that an unchanged
    /// snapshot did nothing.
    generation: u64,
    /// The world's answer, mirrored here for painting only.
    selected: Option<Entity>,

    // --- the graph model (design D11).
    /// Where graph edits go. A row press does not send one: selection is
    /// editor state, recorded in `pending_selection` for the editor to apply.
    graph_commands: Option<Sender<EditorEdit>>,
    graph_selected: Option<GraphNodeId>,
    graph_signature: Vec<String>,
    /// A selection the user asked for that the editor has not applied yet.
    pending_selection: Option<Option<GraphNodeId>>,
}

impl SceneTree {
    pub fn new(commands: Sender<EditorEdit>) -> Self {
        Self {
            rows: Vec::new(),
            generation: 0,
            selected: None,
            graph_commands: Some(commands),
            graph_selected: None,
            graph_signature: Vec::new(),
            pending_selection: None,
        }
    }

    /// Takes the selection the user asked for, if they asked for one.
    pub fn take_selection(this: &mut WidgetMut<'_, Self>) -> Option<Option<GraphNodeId>> {
        this.widget.pending_selection.take()
    }

    /// Points this pane at the graph command set. Once set,
    /// [`populate_from_graph`](Self::populate_from_graph) is the read path and
    /// a row press asks the graph to move its selection.
    pub fn set_graph_commands(this: &mut WidgetMut<'_, Self>, commands: Sender<EditorEdit>) {
        this.widget.graph_commands = Some(commands);
    }

    /// The graph node currently highlighted, if any.
    pub fn graph_selected(&self) -> Option<GraphNodeId> {
        self.graph_selected
    }

    /// The graph node each row stands for, headers included as `None`.
    pub fn graph_rows(&self) -> Vec<Option<GraphNodeId>> {
        self.rows.iter().map(|row| row.graph_node).collect()
    }

    /// Total rows, headers included.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// How many times the row set has actually been rebuilt.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The currently selected entity, if any.
    pub fn selected(&self) -> Option<Entity> {
        self.selected
    }

    /// The `WidgetId` of the `index`th selectable (non-header) row, for
    /// tests that drive a real click by id (see `tests::click_row`) rather
    /// than a hand-computed coordinate.
    pub fn row_id_for_test(&self, index: usize) -> WidgetId {
        self.rows
            .iter()
            .filter(|row| row.entity.is_some())
            .nth(index)
            .map(|row| row.pod.id())
            .expect("row index out of range")
    }

    fn content_height(&self) -> f64 {
        self.rows.len() as f64 * ROW_HEIGHT
    }
}

impl SceneTree {
    /// Rebuilds the row set from the live graph.
    ///
    /// One row per node, labelled by the short form of its kind's reflected
    /// type path -- there is no `Name` component and no entity hierarchy in
    /// the graph model, so the tree is a flat list in `NodeId` order, which is
    /// the same order the graph iterates and evaluates in.
    pub fn populate_from_graph(
        this: &mut WidgetMut<'_, Self>,
        graph: &Graph,
        selection: Option<GraphNodeId>,
    ) {
        if this.widget.graph_selected != selection {
            this.widget.graph_selected = selection;
            this.ctx.request_paint_only();
        }

        let rows: Vec<(GraphNodeId, String)> = graph
            .iter()
            .map(|(id, node)| (id, format!("{} {}", short_type_name(node.kind()), id)))
            .collect();
        let signature: Vec<String> = rows.iter().map(|(_, label)| label.clone()).collect();
        if signature == this.widget.graph_signature {
            return;
        }

        for row in this.widget.rows.drain(..) {
            this.ctx.remove_child(row.pod);
        }
        this.widget.rows.push(Row {
            pod: Label::new("GRAPH").prepare().to_pod(),
            depth: 0,
            entity: None,
            node_id: None,
            graph_node: None,
        });
        for (id, label) in rows {
            this.widget.rows.push(Row {
                pod: Label::new(label).prepare().to_pod(),
                depth: 0,
                entity: None,
                node_id: None,
                graph_node: Some(id),
            });
        }

        this.widget.graph_signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }
}

impl Widget for SceneTree {
    type Action = SceneTreeAction;

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
            let x = PADDING + row.depth as f64 * INDENT;
            let row_size = Size::new((size.width - x).max(0.0), ROW_HEIGHT);
            ctx.run_layout(&mut row.pod, row_size);
            ctx.place_child(&mut row.pod, Point::new(x, index as f64 * ROW_HEIGHT));
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Paints the selection band and the header backgrounds; the row text is
    /// each `Label` child's own job, painted after this.
    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let width = NATURAL_WIDTH.max(
            self.rows
                .iter()
                .map(|row| PADDING + row.depth as f64 * INDENT)
                .fold(0.0, f64::max),
        );
        for (index, row) in self.rows.iter().enumerate() {
            let band = Rect::new(
                0.0,
                index as f64 * ROW_HEIGHT,
                width,
                (index + 1) as f64 * ROW_HEIGHT,
            );
            let header = row.entity.is_none() && row.graph_node.is_none();
            let selected = (row.entity.is_some() && row.entity == self.selected)
                || (row.graph_node.is_some() && row.graph_node == self.graph_selected);
            if header {
                painter.fill_rect(band, Color::from_rgb8(44, 46, 54));
            } else if selected {
                painter.fill_rect(band, Color::from_rgb8(90, 120, 200));
            }
        }
    }

    fn on_pointer_event(
        &mut self,
        ctx: &mut EventCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        event: &PointerEvent,
    ) {
        let PointerEvent::Down(PointerButtonEvent {
            button: Some(PointerButton::Primary),
            state,
            ..
        }) = event
        else {
            return;
        };
        let local = ctx.local_position(state.position);
        let index = (local.y / ROW_HEIGHT).floor();
        let Some(row) = usize::try_from(index as i64)
            .ok()
            .and_then(|i| self.rows.get(i))
        else {
            return;
        };
        // A graph row records a selection for the editor to apply; there is
        // no `Entity` in the new model, and selection is the editor's own
        // state rather than something the graph holds.
        if self.graph_commands.is_some() {
            if let Some(node) = row.graph_node {
                self.pending_selection = Some(Some(node));
            }
            ctx.set_handled();
            return;
        }
        // A header is not selectable.
        let Some(entity) = row.entity else {
            ctx.set_handled();
            return;
        };
        // The world is the only owner of selection now (spec M7-5): a local
        // echo here would be a second opinion, and reconciling two opinions
        // every frame is what caused the M6 flicker. This widget just asks;
        // `apply_snapshot` is what actually moves the highlight, once the
        // world answers back.
        // old wire-model select; graph path handles this
        ctx.submit_action::<Self::Action>(SceneTreeAction {
            entity,
            node_id: row.node_id,
        });
        ctx.request_paint_only();
        ctx.set_handled();
    }

    fn accessibility_role(&self) -> Role {
        Role::Tree
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        self.rows.iter().map(|row| row.pod.id()).collect()
    }

    fn accepts_pointer_interaction(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod graph_model_tests {
    use super::SceneTree;
    use crate::edit::EditorEdit;
    use crate::test_kinds::source_and_gate;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_testing::TestHarness;
    use sway_graph::graph::{Graph, NodeId as GraphNodeId};

    fn harness(
        graph: &Graph,
    ) -> (
        TestHarness<SceneTree>,
        crossbeam_channel::Receiver<EditorEdit>,
    ) {
        harness_with(graph, None)
    }

    /// Selection is the editor's own state now, so a test says what is
    /// selected rather than writing it into the graph first.
    fn harness_with(
        graph: &Graph,
        selection: Option<GraphNodeId>,
    ) -> (
        TestHarness<SceneTree>,
        crossbeam_channel::Receiver<EditorEdit>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (legacy_tx, _legacy_rx) = crossbeam_channel::unbounded();
        let mut harness = TestHarness::create(
            DefaultProperties::default(),
            SceneTree::new(legacy_tx).prepare(),
        );
        harness.edit_root_widget(|mut tree| {
            SceneTree::set_graph_commands(&mut tree, tx);
            SceneTree::populate_from_graph(&mut tree, graph, selection);
        });
        (harness, rx)
    }

    #[test]
    fn every_graph_node_gets_a_row_under_one_header() {
        let (graph, source, gate) = source_and_gate();
        let (harness, _rx) = harness(&graph);

        assert_eq!(
            harness.root_widget().graph_rows(),
            vec![None, Some(source), Some(gate)],
        );
    }

    #[test]
    fn pressing_a_row_records_a_selection_rather_than_an_edit() {
        let (graph, source, _gate) = source_and_gate();
        let (mut harness, rx) = harness(&graph);

        // Row 1 is the first node; row 0 is the header.
        harness.mouse_move(masonry_core::kurbo::Point::new(
            20.0,
            super::ROW_HEIGHT * 1.5,
        ));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert!(
            rx.try_iter().next().is_none(),
            "selecting a node is not a scene edit",
        );
        assert_eq!(
            harness.edit_root_widget(|mut tree| SceneTree::take_selection(&mut tree)),
            Some(Some(source)),
        );
        assert_eq!(
            harness.root_widget().graph_selected(),
            None,
            "the pane only asks -- the editor's answer is what highlights",
        );
    }

    #[test]
    fn the_editors_answer_is_what_highlights_a_row() {
        let (graph, _source, gate) = source_and_gate();
        let (harness, _rx) = harness_with(&graph, Some(gate));

        assert_eq!(harness.root_widget().graph_selected(), Some(gate));
    }

    #[test]
    fn an_unchanged_graph_rebuilds_nothing() {
        let (graph, _source, _gate) = source_and_gate();
        let (mut harness, _rx) = harness(&graph);
        let first = harness.root_widget().generation();

        harness.edit_root_widget(|mut tree| {
            SceneTree::populate_from_graph(&mut tree, &graph, None);
        });

        assert_eq!(harness.root_widget().generation(), first);
    }

    #[test]
    fn a_header_press_selects_nothing() {
        let (graph, _source, _gate) = source_and_gate();
        let (mut harness, rx) = harness(&graph);

        harness.mouse_move(masonry_core::kurbo::Point::new(
            20.0,
            super::ROW_HEIGHT * 0.5,
        ));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(rx.try_iter().count(), 0);
    }
}
