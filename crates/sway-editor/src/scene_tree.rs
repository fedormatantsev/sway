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
use sway_graph::EditorCommand;

use crate::snapshot::{NodeId, TreeGroup, WorldSnapshot};

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
}

/// The world hierarchy pane.
pub struct SceneTree {
    rows: Vec<Row>,
    /// The `(entity, label, depth)` triples the current rows were built from,
    /// compared against the next snapshot to decide whether to rebuild.
    signature: Vec<(Option<Entity>, String, usize)>,
    /// Bumped on every actual rebuild; lets a test assert that an unchanged
    /// snapshot did nothing.
    generation: u64,
    /// The world's answer, mirrored here for painting only. Set from
    /// `snap.selection` in `apply_snapshot`, never by this widget itself --
    /// a row press asks the world to select via `commands` instead (spec
    /// M7-5). The world is the only owner; two opinions reconciled every
    /// frame is what caused the M6 flicker this replaces.
    selected: Option<Entity>,
    /// Where a row press sends `EditorCommand::Select`.
    commands: Sender<EditorCommand>,
}

impl SceneTree {
    pub fn new(commands: Sender<EditorCommand>) -> Self {
        Self {
            rows: Vec::new(),
            signature: Vec::new(),
            generation: 0,
            selected: None,
            commands,
        }
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

fn group_header(group: TreeGroup) -> &'static str {
    match group {
        TreeGroup::Scene => "SCENE",
        TreeGroup::Graph => "GRAPH",
        TreeGroup::Edges => "EDGES",
        TreeGroup::Other => "OTHER",
    }
}

/// The `(entity, label, depth)` signature a snapshot would produce, headers
/// included. Computed without touching the widget tree so it can be compared
/// against the current one before deciding to rebuild.
fn signature_of(snap: &WorldSnapshot) -> Vec<(Option<Entity>, String, usize)> {
    let mut out = Vec::with_capacity(snap.tree.len() + 4);
    let mut current: Option<TreeGroup> = None;
    for row in &snap.tree {
        if current != Some(row.group) {
            current = Some(row.group);
            out.push((None, group_header(row.group).to_string(), 0));
        }
        out.push((Some(row.entity), row.label.clone(), row.depth));
    }
    out
}

// --- MARK: WIDGETMUT
impl SceneTree {
    /// Rebuilds the row set from a snapshot, but only if it actually differs
    /// from the current one.
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        // Selection is the world's, not this widget's (spec M7-5). This must
        // run on *every* call, before the row-signature early return below --
        // not gated behind "did the rows change" -- since a selection can
        // change on its own, with the tree's contents staying exactly the
        // same (e.g. clicking a different row: the row set is identical,
        // only `snap.selection` moves). Getting this wrong makes selection
        // appear to work only when the tree's contents also change, which
        // rebuilds and repaints together -- a bug no signature-based test
        // would catch, only one that applies an unchanged-rows snapshot
        // twice with a different selection each time.
        if this.widget.selected != snap.selection {
            this.widget.selected = snap.selection;
            this.ctx.request_paint_only();
        }

        let signature = signature_of(snap);
        if signature == this.widget.signature {
            return;
        }

        for row in this.widget.rows.drain(..) {
            this.ctx.remove_child(row.pod);
        }

        let mut current: Option<TreeGroup> = None;
        for row in &snap.tree {
            if current != Some(row.group) {
                current = Some(row.group);
                this.widget.rows.push(Row {
                    pod: Label::new(group_header(row.group)).prepare().to_pod(),
                    depth: 0,
                    entity: None,
                    node_id: None,
                });
            }
            this.widget.rows.push(Row {
                pod: Label::new(row.label.clone()).prepare().to_pod(),
                depth: row.depth,
                entity: Some(row.entity),
                node_id: row.node_id,
            });
        }

        this.widget.signature = signature;
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
            if row.entity.is_none() {
                painter.fill_rect(band, Color::from_rgb8(44, 46, 54));
            } else if row.entity == self.selected {
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
        let _ = self.commands.send(EditorCommand::Select {
            entity: Some(entity),
        });
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
mod tests {
    use super::{ROW_HEIGHT, SceneTree};
    use crate::snapshot::{TreeGroup, TreeRow, WorldSnapshot};
    use bevy_ecs::entity::Entity;
    use crossbeam_channel::Sender;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::Point;
    use masonry_testing::TestHarness;
    use sway_graph::EditorCommand;

    fn entity(i: u32) -> Entity {
        Entity::from_raw_u32(i).expect("valid entity id")
    }

    fn row(i: u32, group: TreeGroup, depth: usize, label: &str) -> TreeRow {
        TreeRow {
            entity: entity(i),
            group,
            depth,
            label: label.to_string(),
            node_id: None,
        }
    }

    fn tree(rows: Vec<TreeRow>) -> WorldSnapshot {
        WorldSnapshot {
            tree: rows,
            ..Default::default()
        }
    }

    /// A single-entity tree: one "GRAPH" header plus one selectable row for
    /// `entity(1)`.
    fn one_row_snapshot() -> WorldSnapshot {
        tree(vec![row(1, TreeGroup::Graph, 0, "LFO #1")])
    }

    fn harness_with(
        commands: Sender<EditorCommand>,
        snap: WorldSnapshot,
    ) -> TestHarness<SceneTree> {
        let mut harness = TestHarness::create(
            DefaultProperties::default(),
            SceneTree::new(commands).prepare(),
        );
        harness.edit_root_widget(|mut tree| {
            SceneTree::apply_snapshot(&mut tree, &snap);
        });
        harness
    }

    /// Presses and releases the primary button on a row, addressed by
    /// `WidgetId` rather than a hand-computed coordinate.
    ///
    /// Not `TestHarness::mouse_click_on`: that helper panics unless the
    /// *target* widget itself reports `accepts_pointer_interaction`, and a
    /// row's `Label` deliberately doesn't (masonry's own
    /// `Label::accepts_pointer_interaction` is `false` -- rows exist for
    /// text shaping only, per this module's doc comment). Hit-testing at the
    /// row's own screen position still resolves to `SceneTree` itself, the
    /// same way a real click does (masonry's `find_widget_under_pointer`
    /// falls through a non-accepting child to the nearest accepting
    /// ancestor), so `mouse_move_to_unchecked` -- which only skips that one
    /// panic, not the actual event dispatch -- plus a real press/release is
    /// still full production dispatch, not a bypass seam.
    fn click_row(harness: &mut TestHarness<SceneTree>, row: masonry::core::WidgetId) {
        harness.mouse_move_to_unchecked(row);
        harness.mouse_button_press(Some(PointerButton::Primary));
        harness.mouse_button_release(Some(PointerButton::Primary));
    }

    #[test]
    fn a_header_is_inserted_wherever_the_group_changes() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let harness = harness_with(
            tx,
            tree(vec![
                row(0, TreeGroup::Scene, 0, "root"),
                row(1, TreeGroup::Scene, 1, "mesh"),
                row(2, TreeGroup::Graph, 0, "LFO #3"),
            ]),
        );

        // Three entity rows plus two headers.
        assert_eq!(harness.root_widget().row_count(), 5);
    }

    #[test]
    fn rows_track_the_snapshot_across_a_change() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut harness = harness_with(tx, tree(vec![row(0, TreeGroup::Scene, 0, "root")]));
        assert_eq!(harness.root_widget().row_count(), 2);

        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(
                &mut t,
                &tree(vec![
                    row(0, TreeGroup::Scene, 0, "root"),
                    row(1, TreeGroup::Scene, 1, "mesh"),
                ]),
            );
        });

        assert_eq!(harness.root_widget().row_count(), 3);
    }

    #[test]
    fn an_unchanged_snapshot_rebuilds_nothing() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let snap = tree(vec![row(0, TreeGroup::Scene, 0, "root")]);
        let mut harness = harness_with(tx, snap.clone());
        let before = harness.root_widget().generation();

        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(&mut t, &snap);
        });

        assert_eq!(harness.root_widget().generation(), before);
    }

    /// A regression guard for exactly the mistake the brief calls out: if
    /// `apply_snapshot` moved the `snap.selection` assignment after (or
    /// inside) the row-signature early return, this would fail -- the second
    /// `apply_snapshot` call below changes only `selection`, not the rows,
    /// so a signature-gated assignment would never run.
    #[test]
    fn selection_updates_even_when_the_row_signature_is_unchanged() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let snap = one_row_snapshot();
        let mut harness = harness_with(tx, snap.clone());
        assert_eq!(harness.root_widget().selected(), None);

        let mut reselected = snap;
        reselected.selection = Some(entity(1));
        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(&mut t, &reselected);
        });

        assert_eq!(harness.root_widget().selected(), Some(entity(1)));
    }

    #[test]
    fn a_press_selects_the_row_under_the_pointer() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness = harness_with(
            tx,
            tree(vec![
                row(0, TreeGroup::Scene, 0, "root"),
                row(1, TreeGroup::Scene, 1, "mesh"),
            ]),
        );

        // Row 0 is the "Scene" header; row 1 is `root`; row 2 is `mesh`.
        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 2.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Select {
                entity: Some(entity(1))
            }],
        );
    }

    #[test]
    fn a_press_on_a_header_selects_nothing() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness = harness_with(tx, tree(vec![row(0, TreeGroup::Scene, 0, "root")]));

        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 0.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(
            rx.try_iter().count(),
            0,
            "a header press must send no command"
        );
    }

    #[test]
    fn pressing_a_row_asks_the_world_to_select_it() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness = harness_with(tx, one_row_snapshot());
        let row = harness.root_widget().row_id_for_test(0);

        click_row(&mut harness, row);

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![EditorCommand::Select {
                entity: Some(entity(1))
            }],
        );
    }

    #[test]
    fn a_row_press_does_not_select_locally() {
        // The world is the only owner now. A local echo would be a second
        // opinion, and reconciling two opinions is what caused the flicker.
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut harness = harness_with(tx, one_row_snapshot());
        let row = harness.root_widget().row_id_for_test(0);

        click_row(&mut harness, row);

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
}
