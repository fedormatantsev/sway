//! `Inspector` -- the selected entity's authored values, editable.
//!
//! Rows are `Label` children for headers and read-only values, for the same
//! reason `SceneTree`'s are: `imaging::Painter` takes only pre-shaped
//! glyphs. An editable field gets the widget its `FieldKind` calls for, and
//! committing an edit sends exactly one `EditorCommand::SetField`.

use bevy_ecs::entity::Entity;
use crossbeam_channel::Sender;
use masonry::accesskit::{Node, Role};
use masonry::core::{
    AccessCtx, ActionCtx, ChildrenIds, ErasedAction, LayoutCtx, MeasureCtx, PaintCtx,
    PropertiesMut, PropertiesRef, RegisterCtx, Widget, WidgetId, WidgetMut, WidgetPod,
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
    Text { label: WidgetPod<Label>, input: WidgetPod<TextInput>, input_area: WidgetId },
    Bool { label: WidgetPod<Label>, toggle: WidgetPod<Checkbox> },
    Enum { label: WidgetPod<Label>, selector: WidgetPod<Selector> },
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
}

impl Inspector {
    pub fn new(commands: Sender<EditorCommand>) -> Self {
        Self {
            rows: Vec::new(),
            signature: Vec::new(),
            generation: 0,
            entity: None,
            commands,
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

    /// Parses `text` against the row's `FieldKind` and sends a `SetField`.
    /// A value that does not parse sends nothing -- the field simply snaps back
    /// on the next snapshot.
    fn commit(&mut self, row_index: usize, text: &str) {
        let Some(row) = self.rows.get(row_index) else { return };
        let (Some(entity), Some((component, field, kind))) = (self.entity, row.target.clone())
        else {
            return; // a header row, or nothing selected
        };
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

    pub fn apply_snapshot(this: &mut WidgetMut<'_, Self>, snap: &WorldSnapshot) {
        let signature = signature_of(snap);
        if signature == this.widget.signature {
            return;
        }

        for row in std::mem::take(&mut this.widget.rows) {
            match row.kind {
                RowKind::Header(pod) | RowKind::ReadOnly(pod) => this.ctx.remove_child(pod),
                RowKind::Text { label, input, .. } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(input);
                }
                RowKind::Bool { label, toggle } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(toggle);
                }
                RowKind::Enum { label, selector } => {
                    this.ctx.remove_child(label);
                    this.ctx.remove_child(selector);
                }
            }
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
                    FieldKind::Opaque | FieldKind::Enum(_) => {
                        RowKind::ReadOnly(WidgetPod::new(Label::new(format!(
                            "{}  {}",
                            field.name, field.value
                        ))))
                    }
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
                this.widget.rows.push(Row {
                    kind,
                    target: Some((component.name, field.name.clone(), field.kind.clone())),
                });
            }
        }
        if this.widget.rows.is_empty() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("no authored components"))),
                target: None,
            });
        }

        this.widget.signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
    }

    fn content_height(&self) -> f64 {
        self.rows.len() as f64 * ROW_HEIGHT
    }

    /// Which row an action came from, and the text it commits.
    ///
    /// `None` when the action is one this widget does not act on -- notably
    /// `TextAction::Changed`, which fires per keystroke.
    fn resolve_action(&self, action: &ErasedAction, source: WidgetId) -> Option<(usize, String)> {
        for (index, row) in self.rows.iter().enumerate() {
            match &row.kind {
                RowKind::Text { input, input_area, .. } => {
                    // The action comes from the TextArea inside the TextInput,
                    // not from the TextInput itself.
                    if input.id() != source && *input_area != source {
                        continue;
                    }
                    return match action.downcast_ref::<TextAction>()? {
                        TextAction::Entered(text) => Some((index, text.clone())),
                        TextAction::Changed(_) => None,
                    };
                }
                RowKind::Bool { toggle, .. } if toggle.id() == source => {
                    let CheckboxToggled(checked) = action.downcast_ref::<CheckboxToggled>()?;
                    return Some((index, checked.to_string()));
                }
                RowKind::Enum { selector, .. } if selector.id() == source => {
                    let changed = action.downcast_ref::<SelectionChanged>()?;
                    return Some((index, changed.selected_content.clone()));
                }
                _ => {}
            }
        }
        None
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
        let Some((index, text)) = self.resolve_action(action, source) else {
            return;
        };
        self.commit(index, &text);
        ctx.set_handled();
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
    fn paint(&mut self, _ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let rect = Rect::new(0.0, 0.0, NATURAL_WIDTH, self.content_height());
        painter.fill_rect(rect, Color::from_rgb8(44, 46, 54));
    }

    fn accessibility_role(&self) -> Role {
        Role::List
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx<'_>, _props: &PropertiesRef<'_>, _node: &mut Node) {}

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
    ) -> (TestHarness<Inspector>, crossbeam_channel::Receiver<EditorCommand>) {
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
}
