//! The editor's deferred-edit vocabulary, and the plugin that applies it.
//!
//! A masonry widget cannot borrow the world during event dispatch, so an edit
//! made in a widget has to be recorded and applied after dispatch returns.
//! That is an editor problem, so the recorded form belongs to the editor:
//! [`EditorEdit`] is a description of a gesture this crate made, not a second
//! way to mutate a graph. Everything that *can* reach `&mut Graph` at the
//! moment of the gesture — document load, the viewport's gizmo and picker,
//! `EditorUi::apply_graph` writing canvas placement — calls the graph's
//! methods directly and never builds one of these.
//!
//! The applier is a `match` mapping each variant onto the `Graph` method it
//! names. It runs in `PreUpdate`, so this frame's edits are in the graph
//! before `FixedUpdate` ticks it.

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::change_detection::Mut;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::common_conditions::resource_exists;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use crossbeam_channel::Receiver;
use sway_graph::graph::{EdgeId, Graph, NodeId, Port};
use sway_selection::{Selection, clear_selection_of_removed_nodes};

/// One gesture the editor made, waiting to reach the graph.
///
/// Field paths are relative to the part, exactly as an edge's are:
/// `"frequency"`, not `"inlets.frequency"`.
///
/// There is no "move" and no "select" here: canvas placement is written onto
/// the node's annotations by [`EditorUi::apply_graph`](crate::EditorUi::apply_graph),
/// and the selection is [`sway_selection::Selection`], which the editor owns
/// rather than the graph. Neither is a change to the scene, so neither is an
/// edit.
#[derive(Debug)]
pub enum EditorEdit {
    /// Adds a node of a registered kind, named by its reflected type path,
    /// and annotates it with where the editor dropped it.
    Create {
        /// The node kind's `TypePath::type_path()`.
        kind: String,
        /// Where the editor drops it on its canvas.
        pos: bevy_math::Vec2,
    },
    /// Removes a node and every edge naming it.
    Delete { node: NodeId },
    /// Writes one authored inlet field.
    ///
    /// The value arrives already of the field's declared type: converting
    /// whatever a control produced is this crate's job, done at the widget
    /// (see [`crate::reflect_ui::coerce_field`]), because the control is the
    /// only thing that knows what it produced.
    SetField {
        node: NodeId,
        /// A path relative to the node's `inlets`.
        path: String,
        value: Box<dyn PartialReflect>,
    },
    /// Connects an outlet to an inlet.
    Connect {
        src: Port,
        dst: Port,
        /// The ordering key. `0` for a non-variadic inlet.
        slot: i32,
    },
    /// Removes one edge.
    Disconnect { edge: EdgeId },
    /// Changes one edge's ordering key, reordering a variadic inlet.
    SetSlot { edge: EdgeId, slot: i32 },
}

/// Compares two edits, a reflected field value included.
///
/// Hand-written because `Box<dyn PartialReflect>` cannot derive it. A value
/// that cannot answer `reflect_partial_eq` compares unequal, which is the safe
/// side: this is used to assert what a gesture produced, never to decide
/// whether to write.
impl PartialEq for EditorEdit {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Create { kind, pos },
                Self::Create {
                    kind: other_kind,
                    pos: other_pos,
                },
            ) => kind == other_kind && pos == other_pos,
            (Self::Delete { node }, Self::Delete { node: other_node }) => node == other_node,
            (
                Self::SetField { node, path, value },
                Self::SetField {
                    node: other_node,
                    path: other_path,
                    value: other_value,
                },
            ) => {
                node == other_node
                    && path == other_path
                    && value.reflect_partial_eq(other_value.as_ref()) == Some(true)
            }
            (
                Self::Connect { src, dst, slot },
                Self::Connect {
                    src: other_src,
                    dst: other_dst,
                    slot: other_slot,
                },
            ) => src == other_src && dst == other_dst && slot == other_slot,
            (Self::Disconnect { edge }, Self::Disconnect { edge: other_edge }) => {
                edge == other_edge
            }
            (
                Self::SetSlot { edge, slot },
                Self::SetSlot {
                    edge: other_edge,
                    slot: other_slot,
                },
            ) => edge == other_edge && slot == other_slot,
            _ => false,
        }
    }
}

/// The receiving half, held by the world. Present only in an editor build.
///
/// The channel is not a thread boundary — the winit `window_event` handler
/// drives both masonry and `app.update()` — but it is already wired, already
/// `Send`, and replacing it is not what this crate is for.
#[derive(Resource)]
pub struct EditorEditRx(pub Receiver<EditorEdit>);

/// Drains every queued edit onto the graph.
///
/// Exclusive, because building a node needs the type registry and the graph at
/// once.
pub fn apply_editor_edits(world: &mut World) {
    let Some(rx) = world.get_resource::<EditorEditRx>() else {
        return;
    };
    let edits: Vec<EditorEdit> = rx.0.try_iter().collect();
    if edits.is_empty() {
        return;
    }
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let _ = world.try_resource_scope(|_world, mut graph: Mut<Graph>| {
        for edit in &edits {
            apply_editor_edit(&mut graph, &registry, edit);
        }
    });
}

/// Applies one edit. Split out from [`apply_editor_edits`] so the mapping is
/// testable with no `World` and no channel.
pub fn apply_editor_edit(
    graph: &mut Graph,
    registry: &bevy_reflect::TypeRegistry,
    edit: &EditorEdit,
) {
    match edit {
        EditorEdit::Create { kind, pos } => {
            if let Some(id) = graph.create(registry, kind)
                && let Some(node) = graph.get_mut(id)
            {
                node.metadata_mut()
                    .insert(crate::canvas::CANVAS_POS_KEY.to_string(), Box::new(*pos));
            }
        }
        EditorEdit::Delete { node } => {
            graph.remove(*node);
        }
        EditorEdit::SetField { node, path, value } => {
            graph.set_field(*node, path, value.as_ref());
        }
        EditorEdit::Connect { src, dst, slot } => {
            let _ = graph.connect(src.clone(), dst.clone(), *slot);
        }
        EditorEdit::Disconnect { edge } => {
            graph.disconnect(*edge);
        }
        EditorEdit::SetSlot { edge, slot } => {
            graph.set_slot(*edge, *slot);
        }
    }
}

/// Everything the editor needs on the world side: the edit channel's receiving
/// half, the applier, and the selection.
///
/// Added by the host in editor builds only. A show build never has one, which
/// is why the graph itself holds none of this.
pub struct GraphEditPlugin {
    rx: Receiver<EditorEdit>,
}

impl GraphEditPlugin {
    /// Takes the receiving half of the channel the widget tree sends on.
    pub fn new(rx: Receiver<EditorEdit>) -> Self {
        Self { rx }
    }
}

impl Plugin for GraphEditPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EditorEditRx(self.rx.clone()))
            .init_resource::<Selection>()
            // The editor annotates a node's canvas placement with a `Vec2`,
            // and an annotation recovers its type from the registry on load.
            // Registering it here is what keeps placement from being reported
            // and dropped the next time the project is opened.
            .register_type::<bevy_math::Vec2>()
            .add_systems(
                PreUpdate,
                (
                    apply_editor_edits.run_if(resource_exists::<EditorEditRx>),
                    clear_selection_of_removed_nodes,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_kinds::{Source, registry, source_and_gate};

    #[test]
    fn each_variant_maps_onto_the_graph_method_it_names() {
        let registry = registry();
        let (mut graph, source, gate) = source_and_gate();
        graph.drain_dirty();

        apply_editor_edit(
            &mut graph,
            &registry,
            &EditorEdit::SetField {
                node: source,
                path: "level".into(),
                value: Box::new(0.75f32),
            },
        );
        assert!(graph.is_dirty(source));

        let edge = graph.edges()[0].id;
        apply_editor_edit(
            &mut graph,
            &registry,
            &EditorEdit::SetSlot { edge, slot: 9 },
        );
        assert_eq!(graph.edge(edge).unwrap().slot, 9);

        apply_editor_edit(&mut graph, &registry, &EditorEdit::Disconnect { edge });
        assert!(graph.edges().is_empty());

        apply_editor_edit(
            &mut graph,
            &registry,
            &EditorEdit::Connect {
                src: Port::new(source, "out"),
                dst: Port::new(gate, "gate"),
                slot: 0,
            },
        );
        assert_eq!(graph.edges().len(), 1);

        apply_editor_edit(&mut graph, &registry, &EditorEdit::Delete { node: gate });
        assert!(graph.get(gate).is_none());
    }

    #[test]
    fn create_annotates_the_new_node_with_where_it_was_dropped() {
        let registry = registry();
        let mut graph = Graph::default();

        apply_editor_edit(
            &mut graph,
            &registry,
            &EditorEdit::Create {
                kind: <Source as bevy_reflect::TypePath>::type_path().to_string(),
                pos: bevy_math::Vec2::new(120.0, 40.0),
            },
        );

        let (_, node) = graph.iter().next().expect("one node");
        assert_eq!(
            node.metadata()[crate::canvas::CANVAS_POS_KEY].try_downcast_ref::<bevy_math::Vec2>(),
            Some(&bevy_math::Vec2::new(120.0, 40.0)),
        );
    }

    #[test]
    fn creating_an_unregistered_kind_changes_nothing() {
        let registry = registry();
        let mut graph = Graph::default();
        apply_editor_edit(
            &mut graph,
            &registry,
            &EditorEdit::Create {
                kind: "nothing::registered::Here".into(),
                pos: bevy_math::Vec2::ZERO,
            },
        );
        assert!(graph.is_empty());
    }
}
