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
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ChildrenIds, EventCtx, LayoutCtx, MeasureCtx, PaintCtx, PointerButton,
    PointerButtonEvent, PointerEvent, PropertiesMut, PropertiesRef, RegisterCtx, Widget,
    WidgetMut, WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::Label;
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;
use sway_graph::NodeId;

use crate::snapshot::{TreeGroup, WorldSnapshot};

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
    selected: Option<Entity>,
}

impl Default for SceneTree {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneTree {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            signature: Vec::new(),
            generation: 0,
            selected: None,
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
        if this
            .widget
            .selected
            .is_some_and(|sel| !snap.tree.iter().any(|row| row.entity == sel))
        {
            this.widget.selected = None;
        }
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    /// Sets which entity is highlighted. Used by the selection sync in Task 8.
    pub fn set_selected(this: &mut WidgetMut<'_, Self>, entity: Option<Entity>) {
        if this.widget.selected == entity {
            return;
        }
        this.widget.selected = entity;
        this.ctx.request_paint_only();
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
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let width = NATURAL_WIDTH.max(
            self.rows
                .iter()
                .map(|row| PADDING + row.depth as f64 * INDENT)
                .fold(0.0, f64::max),
        );
        for (index, row) in self.rows.iter().enumerate() {
            let band = Rect::new(0.0, index as f64 * ROW_HEIGHT, width, (index + 1) as f64 * ROW_HEIGHT);
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
        let PointerEvent::Down(PointerButtonEvent { button: Some(PointerButton::Primary), state, .. }) =
            event
        else {
            return;
        };
        let local = ctx.local_position(state.position);
        let index = (local.y / ROW_HEIGHT).floor();
        let Some(row) = usize::try_from(index as i64).ok().and_then(|i| self.rows.get(i)) else {
            return;
        };
        // A header is not selectable.
        let Some(entity) = row.entity else {
            ctx.set_handled();
            return;
        };
        self.selected = Some(entity);
        ctx.submit_action::<Self::Action>(SceneTreeAction { entity, node_id: row.node_id });
        ctx.request_paint_only();
        ctx.set_handled();
    }

    fn accessibility_role(&self) -> Role {
        Role::Tree
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

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
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_core::kurbo::Point;
    use masonry_testing::TestHarness;

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
        WorldSnapshot { tree: rows, nodes: Vec::new(), edges: Vec::new() }
    }

    fn harness_with(snap: WorldSnapshot) -> TestHarness<SceneTree> {
        let mut harness =
            TestHarness::create(DefaultProperties::default(), SceneTree::new().prepare());
        harness.edit_root_widget(|mut tree| {
            SceneTree::apply_snapshot(&mut tree, &snap);
        });
        harness
    }

    #[test]
    fn a_header_is_inserted_wherever_the_group_changes() {
        let harness = harness_with(tree(vec![
            row(0, TreeGroup::Scene, 0, "root"),
            row(1, TreeGroup::Scene, 1, "mesh"),
            row(2, TreeGroup::Graph, 0, "LFO #3"),
        ]));

        // Three entity rows plus two headers.
        assert_eq!(harness.root_widget().row_count(), 5);
    }

    #[test]
    fn rows_track_the_snapshot_across_a_change() {
        let mut harness = harness_with(tree(vec![row(0, TreeGroup::Scene, 0, "root")]));
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
        let snap = tree(vec![row(0, TreeGroup::Scene, 0, "root")]);
        let mut harness = harness_with(snap.clone());
        let before = harness.root_widget().generation();

        harness.edit_root_widget(|mut t| {
            SceneTree::apply_snapshot(&mut t, &snap);
        });

        assert_eq!(harness.root_widget().generation(), before);
    }

    #[test]
    fn a_press_selects_the_row_under_the_pointer() {
        let mut harness = harness_with(tree(vec![
            row(0, TreeGroup::Scene, 0, "root"),
            row(1, TreeGroup::Scene, 1, "mesh"),
        ]));

        // Row 0 is the "Scene" header; row 1 is `root`; row 2 is `mesh`.
        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 2.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected(), Some(entity(1)));
    }

    #[test]
    fn a_press_on_a_header_selects_nothing() {
        let mut harness = harness_with(tree(vec![row(0, TreeGroup::Scene, 0, "root")]));

        harness.mouse_move(Point::new(20.0, ROW_HEIGHT * 0.5));
        harness.mouse_button_press(Some(PointerButton::Primary));

        assert_eq!(harness.root_widget().selected(), None);
    }
}
