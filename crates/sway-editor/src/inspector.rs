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
    Button, ButtonPress, Checkbox, CheckboxToggled, Label, SelectionChanged, Selector, TextAction,
    TextInput,
};
use masonry_core::kurbo::{Axis, Point, Rect, Size};
use peniko::Color;
use sway_graph::graph::{EdgeId, Graph, GraphCommand, NodeId as GraphNodeId, Part, path};
use sway_graph::{EditorCommand, FieldValue};

use crate::canvas::reorder_commands;
use crate::reflect_ui::{
    enum_variants, format_value, has_control, is_bool, is_variadic, parse_field, part_fields,
    short_type_name,
};
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
/// Width of one of an edge-order row's two move buttons.
const MOVE_BUTTON_WIDTH: f64 = 30.0;
/// What the two move buttons say.
const MOVE_UP: &str = "\u{2191}";
const MOVE_DOWN: &str = "\u{2193}";

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
    /// One edge landing on a variadic inlet, with the two buttons that move it
    /// within that inlet's ordering (task 7.7).
    EdgeOrder {
        label: WidgetPod<Label>,
        up: WidgetPod<Button>,
        down: WidgetPod<Button>,
    },
}

/// Which inlet field of which graph node a row edits.
///
/// The field's *reflected type* is what the row carries, not a parallel
/// classification of it (design D11): the control was chosen from this
/// `TypeInfo` and the committed text is parsed back against the same one, so
/// the two can never disagree.
#[derive(Clone, Debug)]
struct GraphFieldTarget {
    node: GraphNodeId,
    /// Inlets-relative field path, exactly what `SetField` takes.
    path: String,
    info: Option<&'static bevy_reflect::TypeInfo>,
}

impl PartialEq for GraphFieldTarget {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
            && self.path == other.path
            && self.info.map(|info| info.type_id()) == other.info.map(|info| info.type_id())
    }
}

/// One edge landing on one variadic inlet, as the inspector presents it.
///
/// `index` is its position in that inlet's ordering-key order -- the position
/// the graph fills the inlet's `Vec` from -- and is what the move buttons
/// shift. `slot` is the key itself, carried so the reorder can emit only the
/// `SetSlot`s that actually change something.
#[derive(Clone, Debug, PartialEq)]
struct EdgeOrderRow {
    node: GraphNodeId,
    /// The variadic inlet's own field path.
    path: String,
    edge: EdgeId,
    slot: i32,
    index: usize,
    label: String,
}

struct Row {
    kind: RowKind,
    /// Which component and field this row edits. `None` for headers.
    target: Option<(&'static str, String, FieldKind)>,
    /// Which graph inlet field this row edits, when the inspector is driven by
    /// the graph model. `None` for headers and for snapshot-driven rows.
    graph_target: Option<GraphFieldTarget>,
    /// Which edge of a variadic inlet this row reorders. `None` for every
    /// other row.
    edge_order: Option<EdgeOrderRow>,
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

    // --- the graph model (design D11).
    /// Where inlet edits go once this inspector is driven by the graph.
    graph_commands: Option<Sender<GraphCommand>>,
    /// The node whose inlets are currently listed.
    graph_node: Option<GraphNodeId>,
    graph_signature: Vec<String>,
    /// Uncommitted keystrokes, keyed by inlets-relative field path.
    graph_pending: HashMap<String, String>,
}

/// Parses exactly `N` comma-separated floats, or nothing.
///
/// Every component must parse and the count must match. The earlier
/// `Vec3`-only version discarded unparseable components instead, so
/// `"1, abc, 2, 3"` committed as `(1, 2, 3)` — a typo silently became a
/// different vector. Shared by both vector kinds so they cannot drift.
fn parse_components<const N: usize>(text: &str) -> Option<[f32; N]> {
    let mut parts = text.split(',');
    let mut out = [0.0; N];
    for slot in out.iter_mut() {
        *slot = parts.next()?.trim().parse::<f32>().ok()?;
    }
    parts.next().is_none().then_some(out)
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
            graph_commands: None,
            graph_node: None,
            graph_signature: Vec::new(),
            graph_pending: HashMap::new(),
        }
    }

    /// Points this inspector at the graph command set. Once set,
    /// [`populate_from_graph`](Self::populate_from_graph) is the read path and
    /// every edit commits as a [`GraphCommand::SetField`].
    pub fn set_graph_commands(this: &mut WidgetMut<'_, Self>, commands: Sender<GraphCommand>) {
        this.widget.graph_commands = Some(commands);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Rows that accept a *field* edit. Headers, values with no control, and
    /// the edge-order rows (which reorder connections rather than write a
    /// field) are not counted.
    pub fn editable_row_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| {
                !matches!(
                    row.kind,
                    RowKind::Header(_) | RowKind::ReadOnly(_) | RowKind::EdgeOrder { .. }
                )
            })
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
            FieldKind::Vec2 => match parse_components::<2>(text) {
                Some([x, y]) => FieldValue::Vec2(bevy_math::Vec2::new(x, y)),
                None => return,
            },
            FieldKind::Vec3 => match parse_components::<3>(text) {
                Some([x, y, z]) => FieldValue::Vec3(bevy_math::Vec3::new(x, y, z)),
                None => return,
            },
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

    /// Parses `text` against the row's *reflected field type* and sends one
    /// [`GraphCommand::SetField`].
    ///
    /// A connection into the field changes nothing here (task 7.8): an inlet
    /// with an edge is still editable, and the edit holds until the next tick
    /// propagates over it. Refusing the edit, or greying the row out, would
    /// misrepresent a graph that accepts the write.
    fn commit_graph(&mut self, row_index: usize, text: &str) {
        let Some(row) = self.rows.get(row_index) else {
            return;
        };
        let Some(target) = row.graph_target.clone() else {
            return; // a header row
        };
        // Acted on either way -- do not replay it on the next blur.
        self.graph_pending.remove(&target.path);
        let Some(info) = target.info else {
            return; // no static type info: no control, nothing to parse
        };
        let Some(value) = parse_field(info, text) else {
            return; // unparseable, or a type with no control
        };
        if let Some(commands) = &self.graph_commands {
            let _ = commands.send(GraphCommand::SetField {
                node: target.node,
                path: target.path,
                value,
            });
        }
    }

    /// Test seam for `commit`, which is otherwise only reachable through a
    /// real text-input action.
    pub fn commit_for_test(this: &mut WidgetMut<'_, Self>, row_index: usize, text: &str) {
        this.widget.commit(row_index, text);
    }

    /// Test seam for `commit_graph`.
    pub fn commit_graph_for_test(this: &mut WidgetMut<'_, Self>, row_index: usize, text: &str) {
        this.widget.commit_graph(row_index, text);
    }

    /// Moves the edge listed at `row_index` by `delta` places within its
    /// variadic inlet's ordering, emitting one `SetSlot` per edge whose key
    /// actually changes (task 7.7).
    ///
    /// Out of range in either direction is a no-op: the first row's "up" and
    /// the last row's "down" do nothing rather than wrapping.
    fn move_edge(&mut self, row_index: usize, delta: isize) {
        let Some(target) = self
            .rows
            .get(row_index)
            .and_then(|row| row.edge_order.clone())
        else {
            return;
        };
        let order: Vec<(EdgeId, i32)> = self
            .rows
            .iter()
            .filter_map(|row| row.edge_order.as_ref())
            .filter(|row| row.node == target.node && row.path == target.path)
            .map(|row| (row.edge, row.slot))
            .collect();
        let Ok(to) = usize::try_from(target.index as isize + delta) else {
            return;
        };
        if to >= order.len() {
            return;
        }
        for command in reorder_commands(&order, target.index, to) {
            if let Some(commands) = &self.graph_commands {
                let _ = commands.send(command);
            }
        }
    }

    /// The edges the inspector currently lists for a variadic inlet, in the
    /// order it lists them -- which is ordering-key order.
    pub fn graph_edge_rows(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| row.edge_order.as_ref().map(|edge| edge.label.clone()))
            .collect()
    }

    /// The move-up / move-down button ids of each listed edge, in the same
    /// order as [`graph_edge_rows`](Self::graph_edge_rows), so a test can
    /// click the real buttons.
    pub fn graph_edge_row_buttons(&self) -> Vec<(WidgetId, WidgetId)> {
        self.rows
            .iter()
            .filter_map(|row| match (&row.kind, &row.edge_order) {
                (RowKind::EdgeOrder { up, down, .. }, Some(_)) => Some((up.id(), down.id())),
                _ => None,
            })
            .collect()
    }

    /// The inlets-relative field path each row edits, headers included as
    /// `None`. Lets a caller (and a test) address a row by field rather than
    /// by the order it happens to be laid out in.
    pub fn graph_row_paths(&self) -> Vec<Option<String>> {
        self.rows
            .iter()
            .map(|row| row.graph_target.as_ref().map(|t| t.path.clone()))
            .collect()
    }

    /// Whether the row editing `path` accepts input.
    pub fn graph_row_is_editable(&self, path: &str) -> bool {
        self.rows.iter().any(|row| {
            row.graph_target.as_ref().is_some_and(|t| t.path == path)
                && !matches!(row.kind, RowKind::Header(_) | RowKind::ReadOnly(_))
        })
    }

    /// Whether a row for `path` exists at all -- a field with no control is
    /// shown read-only rather than dropped, so this is `true` either way.
    pub fn graph_lists(&self, path: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.graph_target.as_ref().is_some_and(|t| t.path == path))
    }

    /// Rebuilds the inspector from the graph's current selection.
    ///
    /// Reads the selected node's `inlets` part and nothing else: state is
    /// never shown, and outlets are canvas sockets rather than authored
    /// fields. Which control a field gets is decided from that field's own
    /// reflected type; a type with no control is listed read-only rather than
    /// omitted, because a silently missing field misrepresents the node.
    pub fn populate_from_graph(
        this: &mut WidgetMut<'_, Self>,
        graph: &Graph,
        registry: &bevy_reflect::TypeRegistry,
    ) {
        let selection = graph.selection();
        let node = selection.and_then(|id| graph.get(id));
        let header = node.map(|node| short_type_name(node.kind()));
        let mut pending: Vec<PendingRow> = Vec::new();
        if let (Some(id), Some(node)) = (selection, node) {
            for field in part_fields(registry, node.kind(), Part::Inlets) {
                let value = path::resolve(node, Part::Inlets, &field.path)
                    .map(format_value)
                    .unwrap_or_default();
                let variadic = field.info.is_some_and(is_variadic);
                pending.push(PendingRow::Field(
                    GraphFieldTarget {
                        node: id,
                        path: field.path.clone(),
                        info: field.info,
                    },
                    value,
                ));
                if variadic {
                    pending.extend(edge_order_rows(graph, registry, id, &field.path));
                }
            }
        }

        let mut signature: Vec<String> = vec![header.clone().unwrap_or_default()];
        signature.extend(pending.iter().map(PendingRow::signature));
        if signature == this.widget.graph_signature {
            return;
        }

        // Same minimum-exclusion rule the snapshot path uses: the row that
        // currently holds focus survives a rebuild triggered by an unrelated
        // field, so a value ticking underneath does not eat a keystroke.
        let focused_id = this.ctx.focus_target_id();
        let focused_target = focused_id.and_then(|id| {
            this.widget
                .rows
                .iter()
                .find(|row| focus_id_of_row(&row.kind) == Some(id))
                .and_then(|row| row.graph_target.clone())
        });

        let mut preserved: Option<Row> = None;
        for row in std::mem::take(&mut this.widget.rows) {
            if preserved.is_none() && focused_target.is_some() && row.graph_target == focused_target
            {
                preserved = Some(row);
                continue;
            }
            remove_row(&mut this.ctx, row);
        }

        this.widget.graph_node = selection;
        this.widget.entity = None;

        match &header {
            Some(header) => this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new(header.clone()))),
                target: None,
                graph_target: None,
                edge_order: None,
            }),
            None => this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("nothing selected"))),
                target: None,
                graph_target: None,
                edge_order: None,
            }),
        }

        for row in pending {
            let (target, value) = match row {
                PendingRow::Field(target, value) => (target, value),
                PendingRow::Edge(edge) => {
                    let label = WidgetPod::new(Label::new(edge.label.clone()));
                    this.widget.rows.push(Row {
                        kind: RowKind::EdgeOrder {
                            label,
                            up: WidgetPod::new(Button::with_text(MOVE_UP)),
                            down: WidgetPod::new(Button::with_text(MOVE_DOWN)),
                        },
                        target: None,
                        graph_target: None,
                        edge_order: Some(edge),
                    });
                    continue;
                }
            };
            if preserved.as_ref().and_then(|row| row.graph_target.clone()) == Some(target.clone()) {
                this.widget
                    .rows
                    .push(preserved.take().expect("checked above"));
                continue;
            }
            let label = WidgetPod::new(Label::new(target.path.clone()));
            let kind = match target.info {
                Some(info) if is_bool(info) => RowKind::Bool {
                    label,
                    toggle: WidgetPod::new(Checkbox::new(value == "true", "")),
                },
                // `Selector::new` debug-panics on an empty list, so a
                // variant-less enum falls through to read-only rather than
                // being trusted.
                Some(info) if enum_variants(info).is_some_and(|variants| !variants.is_empty()) => {
                    let variants = enum_variants(info).expect("checked above");
                    RowKind::Enum {
                        label,
                        selector: WidgetPod::new(
                            Selector::new(variants.clone()).with_selected_option(
                                variants.iter().position(|v| *v == value).unwrap_or(0),
                            ),
                        ),
                    }
                }
                Some(info) if has_control(info) => {
                    let text_input = TextInput::new(&value);
                    let input_area = text_input.area_pod().id();
                    RowKind::Text {
                        label,
                        input: WidgetPod::new(text_input),
                        input_area,
                    }
                }
                // No control: shown, not omitted.
                _ => RowKind::ReadOnly(WidgetPod::new(Label::new(format!(
                    "{}  {}",
                    target.path, value
                )))),
            };
            this.widget.rows.push(Row {
                kind,
                target: None,
                graph_target: Some(target),
                edge_order: None,
            });
        }

        if let Some(row) = preserved.take() {
            remove_row(&mut this.ctx, row);
        }

        this.widget.graph_signature = signature;
        this.widget.generation += 1;
        this.ctx.children_changed();
        this.ctx.request_layout();
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
        if !self.graph_pending.is_empty() {
            for (path, text) in std::mem::take(&mut self.graph_pending) {
                if let Some(index) = self
                    .rows
                    .iter()
                    .position(|row| row.graph_target.as_ref().is_some_and(|t| t.path == path))
                {
                    self.commit_graph(index, &text);
                }
            }
        }
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
                graph_target: None,
                edge_order: None,
            });
        }
        for component in &snap.inspector.components {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new(component.name))),
                target: None,
                graph_target: None,
                edge_order: None,
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
                    // Float, Int, Str, Vec2 and Vec3 all commit as text;
                    // `commit` parses each against its own kind.
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
                    target,
                    graph_target: None,
                    edge_order: None,
                });
            }
        }
        if this.widget.rows.is_empty() {
            this.widget.rows.push(Row {
                kind: RowKind::Header(WidgetPod::new(Label::new("no authored components"))),
                target: None,
                graph_target: None,
                edge_order: None,
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

/// One row the next rebuild will produce, held only for the duration of
/// [`Inspector::populate_from_graph`] so the `&Graph` borrow ends before the
/// widget tree is touched.
enum PendingRow {
    Field(GraphFieldTarget, String),
    Edge(EdgeOrderRow),
}

impl PendingRow {
    /// What makes two reads the same set of rows. An edge row's key is its
    /// position and its ordering key, so a reorder rebuilds and a tick that
    /// changes nothing does not.
    fn signature(&self) -> String {
        match self {
            Self::Field(target, value) => format!(
                "{}={}#{:?}",
                target.path,
                value,
                target.info.map(|info| info.type_path())
            ),
            Self::Edge(edge) => format!(
                "edge {} {} @{} #{}",
                edge.path, edge.edge, edge.slot, edge.index
            ),
        }
    }
}

/// The edges landing on one variadic inlet, in ordering-key order, as rows.
///
/// Task 7.7's input path lives here rather than on the canvas: the inspector
/// already walks the node's declared inlets and already knows which are
/// list-shaped, so presenting their fan costs a row type rather than a new
/// interaction model. Only shown when there are at least two edges -- with one
/// there is no order to change.
fn edge_order_rows(
    graph: &Graph,
    registry: &bevy_reflect::TypeRegistry,
    node: GraphNodeId,
    path: &str,
) -> Vec<PendingRow> {
    let mut edges: Vec<&sway_graph::graph::Edge> = graph
        .edges_into(node)
        .filter(|edge| edge.dst.path == path)
        .collect();
    if edges.len() < 2 {
        return Vec::new();
    }
    edges.sort_by_key(|edge| edge.sort_key());
    edges
        .into_iter()
        .enumerate()
        .map(|(index, edge)| {
            PendingRow::Edge(EdgeOrderRow {
                node,
                path: path.to_string(),
                edge: edge.id,
                slot: edge.slot,
                index,
                label: edge_label(graph, registry, edge),
            })
        })
        .collect()
}

/// How one edge of a variadic inlet is named: the source node's kind and id,
/// plus the outlet path when that kind has more than one outlet to tell apart.
fn edge_label(
    graph: &Graph,
    registry: &bevy_reflect::TypeRegistry,
    edge: &sway_graph::graph::Edge,
) -> String {
    let Some(source) = graph.get(edge.src.node) else {
        return format!("{} {}", edge.src.node, edge.src.path);
    };
    let kind = short_type_name(source.kind());
    if part_fields(registry, source.kind(), Part::Outlets).len() > 1 {
        format!("{kind} {} \u{00b7} {}", edge.src.node, edge.src.path)
    } else {
        format!("{kind} {}", edge.src.node)
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
        // The move buttons are reachable by Tab like any button, but a row
        // that holds no typed text has nothing to preserve across a rebuild --
        // which is all `focus_id_of_row` is consulted for.
        RowKind::Header(_) | RowKind::ReadOnly(_) | RowKind::EdgeOrder { .. } => None,
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
        RowKind::EdgeOrder { label, up, down } => {
            ctx.remove_child(label);
            ctx.remove_child(up);
            ctx.remove_child(down);
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
                RowKind::EdgeOrder { label, up, down } => {
                    ctx.register_child(label);
                    ctx.register_child(up);
                    ctx.register_child(down);
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
        if action.downcast_ref::<ButtonPress>().is_some()
            && let Some((index, delta)) =
                self.rows
                    .iter()
                    .enumerate()
                    .find_map(|(index, row)| match &row.kind {
                        RowKind::EdgeOrder { up, down, .. } if up.id() == source => {
                            Some((index, -1))
                        }
                        RowKind::EdgeOrder { down, .. } if down.id() == source => Some((index, 1)),
                        _ => None,
                    })
        {
            self.move_edge(index, delta);
            ctx.set_handled();
            return;
        }

        let on_graph = self.graph_commands.is_some();
        match self.resolve_action(action, source) {
            Some(RowEvent::Commit(index, text)) => {
                if on_graph {
                    self.commit_graph(index, &text);
                } else {
                    self.commit(index, &text);
                }
                ctx.set_handled();
            }
            Some(RowEvent::Pending(index, text)) => {
                if on_graph {
                    if let Some(target) = self
                        .rows
                        .get(index)
                        .and_then(|row| row.graph_target.clone())
                    {
                        self.graph_pending.insert(target.path, text);
                    }
                } else if let Some((component, field, _)) =
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
                // Indented under the inlet whose ordering it belongs to, with
                // the two move buttons pinned to the right.
                RowKind::EdgeOrder { label, up, down } => {
                    let x = PADDING * 4.0;
                    let buttons = MOVE_BUTTON_WIDTH * 2.0;
                    let label_width = (size.width - x - PADDING - buttons).max(0.0);
                    ctx.run_layout(label, Size::new(label_width, ROW_HEIGHT));
                    ctx.place_child(label, Point::new(x, y));
                    ctx.run_layout(up, Size::new(MOVE_BUTTON_WIDTH, ROW_HEIGHT));
                    ctx.place_child(up, Point::new(x + label_width, y));
                    ctx.run_layout(down, Size::new(MOVE_BUTTON_WIDTH, ROW_HEIGHT));
                    ctx.place_child(down, Point::new(x + label_width + MOVE_BUTTON_WIDTH, y));
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
                RowKind::EdgeOrder { label, up, down } => {
                    ids.extend([label.id(), up.id(), down.id()])
                }
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
    fn committing_a_vec2_sends_a_two_component_value() {
        // A `Vec2` row previously rendered read-only, so `PlaneMesh`'s `size`
        // could be seen but not edited. It must commit as `Vec2`, not as a
        // `Vec3` with a spare zero — reflection matches on the concrete type
        // and the write would be dropped.
        let (mut harness, rx) = harness_with(FieldKind::Vec2, "1.00, 2.00");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_for_test(&mut inspector, 1, "1.5, -2.5");
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(
                &commands[0],
                EditorCommand::SetField { value: FieldValue::Vec2(v), .. }
                    if *v == bevy_math::Vec2::new(1.5, -2.5)
            ),
            "got {:?}",
            commands[0],
        );
    }

    #[test]
    fn a_vector_with_the_wrong_component_count_sends_nothing() {
        // Both directions: too few and too many. The old `Vec3` parse
        // filtered unparseable components out before counting, so a typo
        // could still reach the right count and commit a different vector.
        for text in ["1.5", "1.5, 2.5, 3.5", "1.5, oops"] {
            let (mut harness, rx) = harness_with(FieldKind::Vec2, "1.00, 2.00");
            harness.edit_root_widget(|mut inspector| {
                Inspector::commit_for_test(&mut inspector, 1, text);
            });
            assert_eq!(rx.try_iter().count(), 0, "{text:?} must not commit");
        }
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

/// The graph model (design D11): the inspector reads the selected node's
/// `inlets` part through reflection and commits `GraphCommand::SetField`.
#[cfg(test)]
mod graph_model_tests {
    use super::Inspector;
    use crate::test_kinds::{
        Gate, Memory, Mixer, Source, chained_sources, registry, source_and_gate, variadic_graph,
    };
    use crossbeam_channel::Receiver;
    use masonry::core::{DefaultProperties, PointerButton, Widget};
    use masonry_testing::TestHarness;
    use sway_graph::graph::{FieldValue, Graph, GraphCommand, Node, NodeId as GraphNodeId, Port};

    fn harness(graph: &Graph) -> (TestHarness<Inspector>, Receiver<GraphCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (legacy_tx, _legacy_rx) = crossbeam_channel::unbounded();
        let mut harness = TestHarness::create(
            DefaultProperties::default(),
            Inspector::new(legacy_tx).prepare(),
        );
        harness.edit_root_widget(|mut inspector| {
            Inspector::set_graph_commands(&mut inspector, tx);
            Inspector::populate_from_graph(&mut inspector, graph, &registry());
        });
        (harness, rx)
    }

    fn selected(graph: &mut Graph, node: GraphNodeId) {
        graph.set_selection(Some(node));
    }

    fn paths(harness: &TestHarness<Inspector>) -> Vec<String> {
        harness
            .root_widget()
            .graph_row_paths()
            .into_iter()
            .flatten()
            .collect()
    }

    fn row_of(harness: &TestHarness<Inspector>, path: &str) -> usize {
        harness
            .root_widget()
            .graph_row_paths()
            .iter()
            .position(|row| row.as_deref() == Some(path))
            .unwrap_or_else(|| panic!("no row for {path}"))
    }

    #[test]
    fn the_inspector_lists_the_selected_nodes_inlet_fields() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (harness, _rx) = harness(&graph);

        assert_eq!(paths(&harness), vec!["level", "label", "enabled", "shape"]);
    }

    #[test]
    fn an_outlet_is_a_socket_not_an_editable_field() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (harness, _rx) = harness(&graph);

        assert!(
            !harness.root_widget().graph_lists("out"),
            "`out` is a canvas socket, not an inspector row",
        );
        assert!(!harness.root_widget().graph_lists("pair"));
    }

    #[test]
    fn state_is_hidden() {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(bevy_math::Vec2::ZERO, Memory::default()));
        selected(&mut graph, node);
        let (harness, _rx) = harness(&graph);

        assert_eq!(paths(&harness), vec!["rate"], "`phase` is state");
    }

    #[test]
    fn a_field_with_no_control_is_shown_read_only_rather_than_omitted() {
        let (mut graph, _sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (harness, _rx) = harness(&graph);

        assert!(
            harness.root_widget().graph_lists("terms"),
            "a `Vec<f32>` inlet is still listed",
        );
        assert!(
            !harness.root_widget().graph_row_is_editable("terms"),
            "and it is not editable",
        );
    }

    #[test]
    fn each_control_comes_from_its_fields_reflected_type() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (harness, _rx) = harness(&graph);

        for path in ["level", "label", "enabled", "shape"] {
            assert!(
                harness.root_widget().graph_row_is_editable(path),
                "{path} has a control",
            );
        }
        // Four editable rows and the kind header, nothing else.
        assert_eq!(harness.root_widget().row_count(), 5);
        assert_eq!(harness.root_widget().editable_row_count(), 4);
    }

    #[test]
    fn committing_an_edit_sends_one_set_field_naming_the_inlet_path() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (mut harness, rx) = harness(&graph);
        let row = row_of(&harness, "level");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_graph_for_test(&mut inspector, row, "0.75");
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![GraphCommand::SetField {
                node: source,
                path: "level".to_string(),
                value: FieldValue::Float(0.75),
            }],
        );
    }

    #[test]
    fn an_edit_that_does_not_parse_sends_nothing() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (mut harness, rx) = harness(&graph);
        let row = row_of(&harness, "level");

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_graph_for_test(&mut inspector, row, "not a number");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn committing_on_the_header_row_sends_nothing() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (mut harness, rx) = harness(&graph);

        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_graph_for_test(&mut inspector, 0, "0.75");
        });

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn a_connected_field_is_still_editable() {
        // Task 7.8: an inlet with an edge into it accepts an edit; the edit
        // holds until the next tick propagates over it.
        let (mut graph, _driver, driven) = chained_sources();
        selected(&mut graph, driven);
        let (mut harness, rx) = harness(&graph);

        assert!(
            graph
                .edges_into(driven)
                .any(|edge| edge.dst.path == "level"),
            "sanity: `level` really is connected",
        );
        assert!(harness.root_widget().graph_row_is_editable("level"));

        let row = row_of(&harness, "level");
        harness.edit_root_widget(|mut inspector| {
            Inspector::commit_graph_for_test(&mut inspector, row, "0.25");
        });

        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![GraphCommand::SetField {
                node: driven,
                path: "level".to_string(),
                value: FieldValue::Float(0.25),
            }],
        );
    }

    #[test]
    fn nothing_selected_lists_nothing() {
        let (graph, _source, _gate) = source_and_gate();
        let (harness, _rx) = harness(&graph);

        assert!(paths(&harness).is_empty());
        assert_eq!(harness.root_widget().row_count(), 1, "one header row");
    }

    #[test]
    fn an_unchanged_selection_does_not_rebuild_the_rows() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (mut harness, _rx) = harness(&graph);
        let first = harness.root_widget().generation();

        harness.edit_root_widget(|mut inspector| {
            Inspector::populate_from_graph(&mut inspector, &graph, &registry());
        });

        assert_eq!(harness.root_widget().generation(), first);
    }

    #[test]
    fn a_focused_row_survives_an_unrelated_value_change() {
        let (mut graph, source, _gate) = source_and_gate();
        selected(&mut graph, source);
        let (mut harness, _rx) = harness(&graph);
        let row = row_of(&harness, "level");
        let input_id = harness
            .root_widget()
            .row_focus_id(row)
            .expect("`level` is a text row");
        harness.focus_on(Some(input_id));

        // A different inlet changes underneath.
        if let Some(node) = graph.get_mut(source) {
            let value = sway_graph::graph::path::resolve_mut(
                node,
                sway_graph::graph::Part::Inlets,
                "label",
            )
            .expect("`label` resolves");
            value.try_apply(&"changed".to_string()).expect("same type");
        }
        harness.edit_root_widget(|mut inspector| {
            Inspector::populate_from_graph(&mut inspector, &graph, &registry());
        });

        assert_eq!(
            harness
                .root_widget()
                .row_focus_id(row_of(&harness, "level")),
            Some(input_id),
            "the focused row's widget must survive an unrelated change",
        );
        assert_eq!(harness.focused_widget_id(), Some(input_id));
    }

    // --- MARK: reordering a variadic inlet (task 7.7)

    #[test]
    fn a_variadic_inlets_edges_are_listed_in_ordering_key_order() {
        let (mut graph, sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (harness, _rx) = harness(&graph);

        // `variadic_graph` connects sources 0, 1, 2 at slots 30, 10, 20, so
        // ordering-key order is 1, 2, 0 -- not the order they were connected
        // in, and not `NodeId` order.
        assert_eq!(
            harness.root_widget().graph_edge_rows(),
            vec![
                format!("Source {} \u{00b7} out", sources[1]),
                format!("Source {} \u{00b7} out", sources[2]),
                format!("Source {} \u{00b7} out", sources[0]),
            ],
        );
        assert!(
            harness.root_widget().graph_lists("terms"),
            "the inlet itself is still listed above its edges",
        );
    }

    #[test]
    fn an_edge_label_names_the_outlet_when_the_source_has_more_than_one() {
        // `Source` declares two outlets (`out` and `pair`), so naming the node
        // alone would not say which of them this edge leaves from.
        let (mut graph, sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (harness, _rx) = harness(&graph);

        assert!(
            harness.root_widget().graph_edge_rows()[0].ends_with("\u{00b7} out"),
            "got {:?}",
            harness.root_widget().graph_edge_rows(),
        );
        assert!(harness.root_widget().graph_edge_rows()[0].contains(&sources[1].to_string()),);
    }

    #[test]
    fn an_edge_label_is_just_the_source_when_its_kind_has_one_outlet() {
        // `Gate` declares a single outlet, so naming the node says everything.
        let mut graph = Graph::default();
        let mixer = graph.insert(Node::of(bevy_math::Vec2::new(400.0, 0.0), Mixer::default()));
        let first = graph.insert(Node::of(bevy_math::Vec2::ZERO, Gate::default()));
        let second = graph.insert(Node::of(bevy_math::Vec2::new(0.0, 100.0), Gate::default()));
        for (node, slot) in [(first, 10), (second, 20)] {
            graph
                .connect(Port::new(node, "out"), Port::new(mixer, "terms"), slot)
                .expect("f32 -> Vec<f32>");
        }
        selected(&mut graph, mixer);
        let (harness, _rx) = harness(&graph);

        assert_eq!(
            harness.root_widget().graph_edge_rows(),
            vec![format!("Gate {first}"), format!("Gate {second}")],
        );
    }

    #[test]
    fn an_inlet_with_one_edge_lists_no_ordering_rows() {
        // There is no order to change, so the fan is not drawn.
        let mut graph = Graph::default();
        let mixer = graph.insert(Node::of(bevy_math::Vec2::new(400.0, 0.0), Mixer::default()));
        let source = graph.insert(Node::of(bevy_math::Vec2::ZERO, Source::default()));
        graph
            .connect(Port::new(source, "out"), Port::new(mixer, "terms"), 0)
            .expect("f32 -> Vec<f32>");
        selected(&mut graph, mixer);
        let (harness, _rx) = harness(&graph);

        assert!(harness.root_widget().graph_edge_rows().is_empty());
        assert!(harness.root_widget().graph_lists("terms"));
    }

    #[test]
    fn a_non_variadic_inlet_lists_no_ordering_rows() {
        let (mut graph, _driver, driven) = chained_sources();
        selected(&mut graph, driven);
        let (harness, _rx) = harness(&graph);

        assert!(harness.root_widget().graph_edge_rows().is_empty());
    }

    #[test]
    fn moving_an_edge_down_changes_only_the_keys_that_move() {
        let (mut graph, sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (mut harness, rx) = harness(&graph);
        // Ordering-key order is [s1@10, s2@20, s0@30]; ids in that order.
        let ids: Vec<_> = {
            let mut edges: Vec<_> = graph.edges().to_vec();
            edges.sort_by_key(|edge| edge.sort_key());
            edges.into_iter().map(|edge| edge.id).collect()
        };
        let (_, down) = harness.root_widget().graph_edge_row_buttons()[0];

        harness.mouse_click_on(down, Some(PointerButton::Primary));

        // The wanted order is [second, first, third] renumbered at 0, 10, 20.
        // The first edge moves from key 10 to key 10 -- it lands where it
        // already was, so no `SetSlot` is sent for it.
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![
                GraphCommand::SetSlot {
                    edge: ids[1],
                    slot: 0
                },
                GraphCommand::SetSlot {
                    edge: ids[2],
                    slot: 20
                },
            ],
        );
        assert_eq!(sources.len(), 3);
    }

    #[test]
    fn moving_an_edge_up_moves_it_the_other_way() {
        let (mut graph, _sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (mut harness, rx) = harness(&graph);
        let ids: Vec<_> = {
            let mut edges: Vec<_> = graph.edges().to_vec();
            edges.sort_by_key(|edge| edge.sort_key());
            edges.into_iter().map(|edge| edge.id).collect()
        };
        let (up, _) = harness.root_widget().graph_edge_row_buttons()[2];

        harness.mouse_click_on(up, Some(PointerButton::Primary));

        // [first, second, third] -> [first, third, second] at 0, 10, 20; the
        // second edge keeps key 20 and so is not written.
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![
                GraphCommand::SetSlot {
                    edge: ids[0],
                    slot: 0
                },
                GraphCommand::SetSlot {
                    edge: ids[2],
                    slot: 10
                },
            ],
        );
    }

    #[test]
    fn moving_an_edge_past_either_end_sends_nothing() {
        let (mut graph, _sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (mut harness, rx) = harness(&graph);
        let buttons = harness.root_widget().graph_edge_row_buttons();

        // The first row's "up" and the last row's "down" have nowhere to go.
        harness.mouse_click_on(buttons[0].0, Some(PointerButton::Primary));
        harness.mouse_click_on(buttons[2].1, Some(PointerButton::Primary));

        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn a_reorder_that_actually_happened_is_read_back_in_the_new_order() {
        // The graph is the truth: the inspector shows the new order only once
        // the graph has been told, exactly as the canvas does for selection.
        let (mut graph, sources, mixer) = variadic_graph();
        selected(&mut graph, mixer);
        let (mut harness, rx) = harness(&graph);
        let (_, down) = harness.root_widget().graph_edge_row_buttons()[0];

        harness.mouse_click_on(down, Some(PointerButton::Primary));
        for command in rx.try_iter() {
            let GraphCommand::SetSlot { edge, slot } = command else {
                panic!("expected SetSlot");
            };
            assert!(graph.set_slot(edge, slot), "the key really changed");
        }
        harness.edit_root_widget(|mut inspector| {
            Inspector::populate_from_graph(&mut inspector, &graph, &registry());
        });

        assert_eq!(
            harness.root_widget().graph_edge_rows(),
            vec![
                format!("Source {} \u{00b7} out", sources[2]),
                format!("Source {} \u{00b7} out", sources[1]),
                format!("Source {} \u{00b7} out", sources[0]),
            ],
        );
    }

    #[test]
    fn a_kind_the_editor_has_never_heard_of_is_inspectable() {
        // Design D11's claim: no editor-side description is written for a node
        // kind. `Source` is declared in the test fixtures and nothing in the
        // widget layer names it.
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(bevy_math::Vec2::ZERO, Source::default()));
        selected(&mut graph, node);
        let (harness, _rx) = harness(&graph);

        assert_eq!(paths(&harness), vec!["level", "label", "enabled", "shape"]);
    }
}
