//! `Inspector` -- the selected entity's authored values, editable.
//!
//! Rows are `Label` children for headers and read-only values, for the same
//! reason `SceneTree`'s are: `imaging::Painter` takes only pre-shaped
//! glyphs. An editable field gets the widget its `FieldKind` calls for, and
//! committing an edit sends exactly one `EditorCommand::SetField`.

use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use crossbeam_channel::Sender;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, LayoutCtx, MeasureCtx, PaintCtx,
    PropertiesMut, PropertiesRef, RegisterCtx, Update, UpdateCtx, Widget, WidgetId, WidgetMut,
    WidgetPod,
};
use masonry::imaging::Painter;
use masonry::layout::{LenReq, Length};
use masonry::widgets::{
    Checkbox, CheckboxToggled, Label, SelectionChanged, Selector, TextAction, TextInput,
};
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;
use sway_graph::{EditorCommand, FieldValue};

use crate::snapshot::{FieldKind, WorldSnapshot};

// `TextInput` and `Selector` carry the theme's default border+padding
// (`Padding::from_vh(6px, 12px)` plus a 1px border on each side — see
// `masonry::theme::default_theme`'s `TextInput`/`Selector` entries). Masonry's
// layout pass treats the size handed to `run_layout` as the *border box* and
// subtracts padding+border before giving the widget its content space, so
// this has to clear the 15px-font/1.2-line-height content (18px) plus 12px
// padding plus 2px border, or the text overflows past its shrunk content box
// and paints across the border.
pub const ROW_HEIGHT: f64 = 32.0;
const PADDING: f64 = 8.0;
const NATURAL_WIDTH: f64 = 240.0;

/// One rendered row. A header or a read-only value is a `Label`; an editable
/// field is the widget its `FieldKind` calls for.
enum RowKind {
    Header(WidgetPod<Label>),
    ReadOnly(WidgetPod<Label>),
    Text {
        label: WidgetPod<Label>,
        input: WidgetPod<TextInput>,
        input_area: WidgetId,
    },
    Bool {
        label: WidgetPod<Label>,
        toggle: WidgetPod<Checkbox>,
    },
    Enum {
        label: WidgetPod<Label>,
        selector: WidgetPod<Selector>,
    },
}

struct Row {
    kind: RowKind,
    /// Which component and field this row edits. `None` for headers.
    target: Option<(&'static str, String, FieldKind)>,
}

pub struct Inspector {
    rows: Vec<Row>,
    signature: Vec<String>,
    generation: u64,
    entity: Option<Entity>,
    commands: Sender<EditorCommand>,
    /// Text a `Text` row has typed but not yet committed (`TextAction::Changed`
    /// without a following `Entered`), keyed by `(component, field)` rather
    /// than row index -- a row's index can shift under `apply_snapshot`
    /// while an edit is pending. Flushed by [`Inspector::commit_pending`],
    /// which fires on `Update::ChildFocusChanged(false)` (design spec:
    /// "committing on Enter and on blur").
    pending: HashMap<(&'static str, String), String>,
}

impl Inspector {
    pub fn new(commands: Sender<EditorCommand>) -> Self {
        Self {
            rows: Vec::new(),
            signature: Vec::new(),
            generation: 0,
            entity: None,
            commands,
            pending: HashMap::new(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Rows that accept input. The rest are headers and unclassified values.
    pub fn editable_row_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| !matches!(row.kind, RowKind::Header(_) | RowKind::ReadOnly(_)))
            .count()
    }

    /// The widget id a row actually receives focus on, if `row_index` names
    /// one -- see [`focus_id_of_row`]. Lets a test drive a real
    /// `TestHarness::focus_on`/blur through the same id masonry itself
    /// tracks, the same shape `Palette::row_id` gives `mouse_click_on`.
    pub fn row_focus_id(&self, row_index: usize) -> Option<WidgetId> {
        self.rows
            .get(row_index)
            .and_then(|row| focus_id_of_row(&row.kind))
    }

    /// Parses `text` against the row's `FieldKind` and sends a `SetField`.
    /// A value that does not parse sends nothing -- the field simply snaps back
    /// on the next snapshot.
    fn commit(&mut self, row_index: usize, text: &str) {
        let Some(row) = self.rows.get(row_index) else {
            return;
        };
        let (Some(entity), Some((component, field, kind))) = (self.entity, row.target.clone())
        else {
            return; // a header row, or nothing selected
        };
        // Whether this commit succeeds or not, the pending keystroke (if any)
        // for this field has been acted on -- don't replay it again on the
        // next blur.
        self.pending.remove(&(component, field.clone()));
        let value = match kind {
            FieldKind::Float => match text.trim().parse::<f32>() {
                Ok(v) => FieldValue::Float(v),
                Err(_) => return,
            },
            FieldKind::Int => match text.trim().parse::<i64>() {
                Ok(v) => FieldValue::Int(v),
                Err(_) => return,
            },
            FieldKind::Bool => FieldValue::Bool(text == "true"),
            FieldKind::Enum(_) => FieldValue::Enum(text.to_string()),
            FieldKind::Str => FieldValue::Str(text.to_string()),
            FieldKind::Vec3 => {
                let parts: Vec<f32> = text
                    .split(',')
                    .filter_map(|p| p.trim().parse::<f32>().ok())
                    .collect();
                if parts.len() != 3 {
                    return;
                }
                FieldValue::Vec3(bevy_math::Vec3::new(parts[0], parts[1], parts[2]))
            }
            FieldKind::Opaque => return,
        };

        // Send-failure is not an error worth reporting: the only way the
        // receiver is gone is that the app is shutting down.
        let _ = self.commands.send(EditorCommand::SetField {
            entity,
            component,
            field,
            value,
        });
    }

    /// Test seam for `commit`, which is otherwise only reachable through a
    /// real text-input action.
    pub fn commit_for_test(this: &mut WidgetMut<'_, Self>, row_index: usize, text: &str) {
        this.widget.commit(row_index, text);
    }

    /// Commits every field with an uncommitted keystroke (a `TextAction::Changed`
    /// with no following `Entered`). Design spec: text rows commit "on Enter and
    /// on blur"; this is the blur half, invoked from `Update::ChildFocusChanged(false)`.
    ///
    /// Masonry only delivers `ChildFocusChanged` to a widget when *that
    /// widget's own* aggregate has-focus-in-subtree status flips. Since
    /// `Inspector` is a common ancestor of every row, it only sees this
    /// transition when focus leaves the whole panel (click away, or select
    /// something elsewhere in the UI) -- not when focus moves from one row
    /// to another row inside the panel (each row's own `TextInput`/`TextArea`
    /// handles that transition internally, with no hook this widget can
    /// observe). Flushing every pending field on whole-panel blur still
    /// covers the reported bug (edit a field, click away, the edit is
    /// lost) and, because `pending` is keyed by field rather than row index
    /// and survives `apply_snapshot`, correctly catches every field edited
    /// since the last commit, not just the one that happened to hold focus
    /// last.
    fn commit_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        for ((component, field), text) in std::mem::take(&mut self.pending) {
            if let Some(index) = self.rows.iter().position(|row| {
                row.target
                    .as_ref()
                    .is_some_and(|(c, f, _)| *c == component && *f == field)
            }) {
                self.commit(index, &text);
            }
        }
    }

    /// Important #3 (final review): a full rebuild tears down every row's
    /// widget -- dropping focus and any in-progress, uncommitted text --
    /// even when only an unrelated field's value changed (e.g. selecting an
    /// `Oscillator`, whose `#[require]`d `FloatOut` changes every tick while the
    /// transport runs). A full in-place reconcile of every row is a larger
    /// rewrite than this pass can land safely; instead, the one row (if
    /// any) that currently holds focus is carried over untouched -- its
    /// widget, id, and whatever the user has typed into it survive --
    /// provided the field it targets is still present with the same
    /// `FieldKind` in the new snapshot. This is a deliberate reduced scope:
    /// unfocused rows are still rebuilt on every value change.
    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let signature = signature_of(snap);
        if signature == this.widget.signature {
            return;
        }

        let focused_id = this.ctx.focus_target_id();
        let focused_target = focused_id.and_then(|id| {
            this.widget
                .rows
                .iter()
                .find(|row| focus_id_of_row(&row.kind) == Some(id))
                .and_then(|row| row.target.clone())
        });

        let mut preserved: Option<Row> = None;
        for row in std::mem::take(&mut this.widget.rows) {
            if preserved.is_none() && focused_target.is_some() && row.target == focused_target {
                preserved = Some(row);
                continue;
            }
            remove_row(&mut this.ctx, row);
        }

        this.widget.entity = snap.inspector.entity;

        if snap.inspector.entity.is_none() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("nothing selected"))),
                target: None,
            });
        }
        for component in &snap.inspector.components {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new(component.name))),
                target: None,
            });
            for field in &component.fields {
                let target = Some((component.name, field.name.clone(), field.kind.clone()));
                if preserved.as_ref().and_then(|row| row.target.clone()) == target {
                    this.widget
                        .rows
                        .push(preserved.take().expect("checked above"));
                    continue;
                }
                let label = WidgetPod::new(Label::new(field.name.clone()));
                let kind = match &field.kind {
                    FieldKind::Bool => RowKind::Bool {
                        label,
                        toggle: WidgetPod::new(Checkbox::new(field.value == "true", "")),
                    },
                    // An enum with no variants cannot happen (`enum_kind` reads
                    // them off `TypeInfo`), but `Selector::new` debug-panics on
                    // an empty list, so it is rendered read-only rather than
                    // trusted.
                    FieldKind::Enum(variants) if !variants.is_empty() => RowKind::Enum {
                        label,
                        selector: WidgetPod::new(
                            Selector::new(variants.clone()).with_selected_option(
                                variants.iter().position(|v| *v == field.value).unwrap_or(0),
                            ),
                        ),
                    },
                    FieldKind::Opaque | FieldKind::Enum(_) => RowKind::ReadOnly(WidgetPod::new(
                        Label::new(format!("{}  {}", field.name, field.value)),
                    )),
                    // Float, Int, Str and Vec3 all commit as text; `commit`
                    // parses each against its own kind.
                    _ => {
                        let text_input = TextInput::new(&field.value);
                        let input_area = text_input.area_pod().id();
                        RowKind::Text {
                            label,
                            input: WidgetPod::new(text_input),
                            input_area,
                        }
                    }
                };
                this.widget.rows.push(Row { kind, target });
            }
        }
        if this.widget.rows.is_empty() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("no authored components"))),
                target: None,
            });
        }
        // The preserved row's field no longer exists in the new snapshot
        // (its component/entity was deleted, or the selection changed) --
        // it was never reused above, so it still needs tearing down.
        if let Some(row) = preserved.take() {
            remove_row(&mut this.ctx, row);
        }

        this.widget.signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    fn content_height(&self) -> f64 {
        self.rows.len() as f64 * ROW_HEIGHT
    }

    /// Which row an action came from, the text it carries, and whether it
    /// commits immediately (`Entered`, a checkbox toggle, an enum pick) or
    /// only marks the field dirty for a later blur commit (`Changed`, which
    /// fires per keystroke).
    fn resolve_action(&self, action: &ErasedAction, source: WidgetId) -> Option<RowEvent> {
        for (index, row) in self.rows.iter().enumerate() {
            match &row.kind {
                RowKind::Text {
                    input, input_area, ..
                } => {
                    // The action comes from the TextArea inside the TextInput,
                    // not from the TextInput itself.
                    if input.id() != source && *input_area != source {
                        continue;
                    }
                    return match action.downcast_ref::<TextAction>()? {
                        TextAction::Entered(text) => Some(RowEvent::Commit(index, text.clone())),
                        TextAction::Changed(text) => Some(RowEvent::Pending(index, text.clone())),
                    };
                }
                RowKind::Bool { toggle, .. } if toggle.id() == source => {
                    let CheckboxToggled(checked) = action.downcast_ref::<CheckboxToggled>()?;
                    return Some(RowEvent::Commit(index, checked.to_string()));
                }
                RowKind::Enum { selector, .. } if selector.id() == source => {
                    let changed = action.downcast_ref::<SelectionChanged>()?;
                    return Some(RowEvent::Commit(index, changed.selected_content.clone()));
                }
                _ => {}
            }
        }
        None
    }
}

/// What `resolve_action` found: an action that commits (`SetField` sent
/// right away) or one that only records a pending, not-yet-committed edit
/// (Important #2: text rows also commit on blur).
enum RowEvent {
    Commit(usize, String),
    Pending(usize, String),
}

/// The widget id that actually receives keyboard/text focus for this row, if
/// any. For `Text` rows this is the inner `TextArea`, not the `TextInput`
/// itself -- masonry delivers focus to the child, matching `resolve_action`'s
/// own `input_area` comparison above.
fn focus_id_of_row(kind: &RowKind) -> Option<WidgetId> {
    match kind {
        RowKind::Text { input_area, .. } => Some(*input_area),
        RowKind::Bool { toggle, .. } => Some(toggle.id()),
        RowKind::Enum { selector, .. } => Some(selector.id()),
        RowKind::Header(_) | RowKind::ReadOnly(_) => None,
    }
}

/// Detaches one row's widget(s) from the tree. Shared between `apply_snapshot`'s
/// teardown of every unpreserved row and its cleanup of a preserved row whose
/// field turned out not to exist in the new snapshot after all.
fn remove_row(ctx: &mut masonry::core::MutateCtx<'_>, row: Row) {
    match row.kind {
        RowKind::Header(pod) | RowKind::ReadOnly(pod) => ctx.remove_child(pod),
        RowKind::Text { label, input, .. } => {
            ctx.remove_child(label);
            ctx.remove_child(input);
        }
        RowKind::Bool { label, toggle } => {
            ctx.remove_child(label);
            ctx.remove_child(toggle);
        }
        RowKind::Enum { label, selector } => {
            ctx.remove_child(label);
            ctx.remove_child(selector);
        }
    }
}

/// What makes two snapshots the same set of rows. Includes the kind, because a
/// field whose text is unchanged but whose kind changed needs a new widget.
fn signature_of(snap: &WorldSnapshot) -> Vec<String> {
    let mut signature = Vec::new();
    for component in &snap.inspector.components {
        signature.push(component.name.to_string());
        for field in &component.fields {
            signature.push(format!("{}={}#{:?}", field.name, field.value, field.kind));
        }
    }
    signature
}

/// Lays a label/editor pair out on one row: the label in a fixed left column,
/// the editor filling what is left. Generic over the editor's widget type,
/// which is the only thing that differs between the three editable kinds.
fn place_field<W: Widget + ?Sized>(
    ctx: &mut LayoutCtx<'_>,
    label: &mut WidgetPod<Label>,
    editor: &mut WidgetPod<W>,
    size: Size,
    y: f64,
    label_width: f64,
) {
    let x = PADDING * 2.0;
    ctx.run_layout(label, Size::new(label_width, ROW_HEIGHT));
    ctx.place_child(label, Point::new(x, y));

    let editor_x = x + label_width;
    let editor_width = (size.width - editor_x - PADDING).max(0.0);
    ctx.run_layout(editor, Size::new(editor_width, ROW_HEIGHT));
    ctx.place_child(editor, Point::new(editor_x, y));
}

impl Widget for Inspector {
    type Action = ();

    fn register_children(&mut self, ctx: &mut RegisterCtx<'_>) {
        for row in &mut self.rows {
            match &mut row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => ctx.register_child(pod),
                RowKind::Text { label, input, .. } => {
                    ctx.register_child(label);
                    ctx.register_child(input);
                }
                RowKind::Bool { label, toggle } => {
                    ctx.register_child(label);
                    ctx.register_child(toggle);
                }
                RowKind::Enum { label, selector } => {
                    ctx.register_child(label);
                    ctx.register_child(selector);
                }
            }
        }
    }

    fn on_action(
        &mut self,
        ctx: &mut ActionCtx<'_>,
        _props: &mut PropertiesMut<'_>,
        action: &ErasedAction,
        source: WidgetId,
    ) {
        match self.resolve_action(action, source) {
            Some(RowEvent::Commit(index, text)) => {
                self.commit(index, &text);
                ctx.set_handled();
            }
            Some(RowEvent::Pending(index, text)) => {
                if let Some((component, field, _)) =
                    self.rows.get(index).and_then(|row| row.target.clone())
                {
                    self.pending.insert((component, field), text);
                }
                ctx.set_handled();
            }
            None => {}
        }
    }

    /// Important #2 (final review): text rows commit "on Enter and on blur"
    /// per the design spec, but only the Enter half (`TextAction::Entered`,
    /// handled in `on_action`) shipped. `ChildFocusChanged(false)` is masonry's
    /// signal that no descendant of this widget holds focus any more -- see
    /// `commit_pending`'s doc comment for exactly what that does and doesn't
    /// cover.
    fn update(&mut self, _ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if let Update::ChildFocusChanged(false) = event {
            self.commit_pending();
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
        const LABEL_WIDTH: f64 = 90.0;
        for (index, row) in self.rows.iter_mut().enumerate() {
            let y = index as f64 * ROW_HEIGHT;
            match &mut row.kind {
                RowKind::Header(pod) => {
                    let row_size = Size::new((size.width - PADDING).max(0.0), ROW_HEIGHT);
                    ctx.run_layout(pod, row_size);
                    ctx.place_child(pod, Point::new(PADDING, y));
                }
                RowKind::ReadOnly(pod) => {
                    let row_size = Size::new((size.width - PADDING * 2.0).max(0.0), ROW_HEIGHT);
                    ctx.run_layout(pod, row_size);
                    ctx.place_child(pod, Point::new(PADDING * 2.0, y));
                }
                RowKind::Text { label, input, .. } => {
                    place_field(ctx, label, input, size, y, LABEL_WIDTH);
                }
                RowKind::Bool { label, toggle } => {
                    place_field(ctx, label, toggle, size, y, LABEL_WIDTH);
                }
                RowKind::Enum { label, selector } => {
                    place_field(ctx, label, selector, size, y, LABEL_WIDTH);
                }
            }
        }
        ctx.set_clip_path(size.to_rect());
    }

    /// Fills the background the same colour `SceneTree` uses for its header
    /// rows; the row text is each `Label` child's own job, painted after
    /// this.
    fn paint(
        &mut self,
        _ctx: &mut PaintCtx<'_>,
        _props: &PropertiesRef<'_>,
        painter: &mut Painter<'_>,
    ) {
        let rect = Rect::new(0.0, 0.0, NATURAL_WIDTH, self.content_height());
        painter.fill_rect(rect, Color::from_rgb8(44, 46, 54));
    }

    fn accessibility_role(&self) -> Role {
        Role::List
    }

    fn accessibility(
        &mut self,
        _ctx: &mut AccessCtx<'_>,
        _props: &PropertiesRef<'_>,
        _node: &mut Node,
    ) {
    }

    fn children_ids(&self) -> ChildrenIds {
        let mut ids = Vec::new();
        for row in &self.rows {
            match &row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => ids.push(pod.id()),
                RowKind::Text { label, input, .. } => ids.extend([label.id(), input.id()]),
                RowKind::Bool { label, toggle } => ids.extend([label.id(), toggle.id()]),
                RowKind::Enum { label, selector } => ids.extend([label.id(), selector.id()]),
            }
        }
        ids.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{FieldKind, InspectorComponent, InspectorField, InspectorView};
    use bevy_ecs::entity::Entity;
    use masonry::core::DefaultProperties;
    use masonry_testing::TestHarness;
    use sway_graph::{EditorCommand, FieldValue};

    /// `TestHarness::create` takes the default property set and a *prepared*
    /// widget (`NewWidget<W>`) -- see `canvas.rs`'s own `harness_with` for the
    /// same shape. `harness.root_widget()` is a `WidgetRef<W>`, which derefs
    /// to `W`, so the widget's own methods are called on it directly.
    fn harness_with(
        kind: FieldKind,
        value: &str,
    ) -> (
        TestHarness<Inspector>,
        crossbeam_channel::Receiver<EditorCommand>,
    ) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Inspector::new(tx).prepare());
        let snap = snapshot_with(kind, value);
        harness.edit_root_widget(|mut inspector| {
            Inspector::apply_snapshot(&mut inspector, &snap);
        });
        (harness, rx)
    }

    fn snapshot_with(kind: FieldKind, value: &str) -> WorldSnapshot {
        WorldSnapshot {
            inspector: InspectorView {
                entity: Some(Entity::from_raw_u32(3).expect("valid entity id")),
                components: vec![InspectorComponent {
                    name: "Knobs",
                    fields: vec![InspectorField {
                        name: "gain".to_string(),
                        value: value.to_string(),
                        kind,
                    }],
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_float_field_gets_an_editable_input_not_a_label() {
        let (harness, _rx) = harness_with(FieldKind::Float, "0.500");
        assert_eq!(harness.root_widget().editable_row_count(), 1);
    }

    #[test]
    fn a_bool_field_gets_a_checkbox_and_an_enum_gets_a_selector() {
        let (bools, _rx) = harness_with(FieldKind::Bool, "true");
        assert_eq!(bools.root_widget().editable_row_count(), 1);

        let (enums, _rx) = harness_with(
            FieldKind::Enum(vec!["Sine".to_string(), "Saw".to_string()]),
            "Saw",
        );
        assert_eq!(enums.root_widget().editable_row_count(), 1);
    }

    #[test]
    fn an_opaque_field_stays_read_only() {
        // Which remains the signal that a type wants editor TypeData.
        let (harness, _rx) = harness_with(FieldKind::Opaque, "?");
        assert_eq!(harness.root_widget().editable_row_count(), 0);
    }

    #[test]
    fn committing_a_float_sends_exactly_one_set_field() {
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 1, "0.75");
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::SetField { component, field, value: FieldValue::Float(v), .. }
                    if *component == "Knobs" && field == "gain" && (*v - 0.75).abs() < f32::EPSILON
            ),
            "got {:?}",
            commands[0],
        );
    }

    #[test]
    fn an_unparseable_float_sends_nothing() {
        // The field simply snaps back on the next snapshot.
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 1, "not a number");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn committing_on_a_header_row_sends_nothing() {
        // Row 0 is the "Knobs" header, which has no field to write.
        let (mut harness, rx) = harness_with(FieldKind::Float, "0.500");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 0, "0.75");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn a_value_change_followed_by_a_real_blur_commits_it_not_just_enter() {
        // Important #2 (final review): `commit` fired only on
        // `TextAction::Entered`, discarding `Changed` -- so a typed value
        // was lost if the user clicked away instead of pressing Enter,
        // contradicting the design spec's "committing on Enter and on
        // blur". This drives real focus, keystroke and blur events through
        // the harness (no `commit_for_test` bypass) to prove the blur half
        // now actually fires. Starts from an empty field so the typed text
        // is exactly what a fresh `TextArea`'s buffer ends up holding,
        // independent of where its initial cursor sits.
        let (mut harness, rx) = harness_with(FieldKind::Float, "");
        let input_id = harness
            .root_widget()
            .row_focus_id(1)
            .expect("row 1 is the gain text row");

        harness.focus_on(Some(input_id));
        harness.keyboard_type_chars("0.75");
        assert_eq!(
            rx.try_iter().count(),
            0,
            "a keystroke alone (Changed) must not commit -- only Enter or blur does",
        );

        harness.focus_on(None); // real blur: masonry dispatches ChildFocusChanged(false)

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1, "got {:?}", commands);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::SetField { component, field, value: FieldValue::Float(v), .. }
                    if *component == "Knobs" && field == "gain" && (*v - 0.75).abs() < f32::EPSILON
            ),
            "got {:?}",
            commands[0],
        );
    }

    /// Two components so a field on one (`FloatOut`'s tuple field `"0"`) can
    /// change while a field on the other (`Knobs.gain`) -- the one under
    /// test -- does not, mirroring an `Oscillator`'s continuously-authored
    /// `FloatOut` sitting alongside a knob the user is mid-edit on.
    fn two_component_snapshot(gain: &str, float_out: &str) -> WorldSnapshot {
        WorldSnapshot {
            inspector: InspectorView {
                entity: Some(Entity::from_raw_u32(3).expect("valid entity id")),
                components: vec![
                    InspectorComponent {
                        name: "Knobs",
                        fields: vec![InspectorField {
                            name: "gain".to_string(),
                            value: gain.to_string(),
                            kind: FieldKind::Float,
                        }],
                    },
                    InspectorComponent {
                        name: "FloatOut",
                        fields: vec![InspectorField {
                            name: "0".to_string(),
                            value: float_out.to_string(),
                            kind: FieldKind::Float,
                        }],
                    },
                ],
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_focused_row_survives_an_unrelated_value_change() {
        // Important #3 (final review): `apply_snapshot` tore down and
        // rebuilt every row -- new widgets, new `WidgetId`s, dropped focus
        // -- whenever ANY displayed value changed, even one the user isn't
        // touching. This is the minimum-exclusion fix (not a full
        // reconcile of every row; see the fix report for why the narrower
        // scope was chosen): the currently focused row's widget is carried
        // over untouched across a rebuild triggered by an unrelated field.
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut harness =
            TestHarness::create(DefaultProperties::default(), Inspector::new(tx).prepare());
        harness.edit_root_widget(|mut inspector| {
            Inspector::apply_snapshot(&mut inspector, &two_component_snapshot("0.500", "0.1"));
        });

        let gain_input_id = harness
            .root_widget()
            .row_focus_id(1)
            .expect("row 1 is the gain text row");
        harness.focus_on(Some(gain_input_id));
        assert_eq!(harness.focused_widget_id(), Some(gain_input_id));

        // Only FloatOut's field changes -- the row under test does not.
        harness.edit_root_widget(|mut inspector| {
            Inspector::apply_snapshot(&mut inspector, &two_component_snapshot("0.500", "0.2"));
        });

        assert_eq!(
            harness.root_widget().row_focus_id(1),
            Some(gain_input_id),
            "the focused row's widget must survive an unrelated field's value change",
        );
        assert_eq!(
            harness.focused_widget_id(),
            Some(gain_input_id),
            "focus itself must not be dropped by the rebuild",
        );
    }
}
