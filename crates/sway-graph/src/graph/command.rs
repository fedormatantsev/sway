//! The graph command set — the only way anything outside the graph writes it.
//!
//! Replaces `EditorCommand`. Commands are plain data: the editor produces them
//! and sends them down a channel, and the applier does the reflect work on the
//! world side where the type registry is in hand. Authoring is
//! single-direction (design D7) — the gizmo emits the same `SetField` the
//! inspector does, and nothing writes the graph except this.

use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_math::{Quat, Vec2, Vec3};
use bevy_reflect::enums::{DynamicEnum, DynamicVariant};
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{PartialReflect, TypeRegistry};
use crossbeam_channel::Receiver;

use crate::graph::edge::Port;
use crate::graph::id::{EdgeId, NodeId};
use crate::graph::model::{ConnectError, Graph, reflect_equal};
use crate::graph::node::{Node, Part};
use crate::graph::path;
use crate::graph::registry::ReflectNodeKind;

/// One edited field's new value.
///
/// Deliberately not `Box<dyn Reflect>`: the channel payload stays `Send`,
/// `Clone` and plainly comparable. A whole-part write (document load) does not
/// go through commands — it applies a deserialized value directly.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    /// Any float field; narrowed to `f32` or `f64` on arrival.
    Float(f64),
    /// Any integer field; narrowed, saturating, on arrival.
    Int(i64),
    /// A `bool` field.
    Bool(bool),
    /// A unit enum variant, by name.
    Enum(String),
    /// A `String` field.
    Str(String),
    /// A `Vec2` field.
    Vec2(Vec2),
    /// A `Vec3` field.
    Vec3(Vec3),
    /// A `Quat` field — a scene node's `rotation`.
    Quat(Quat),
}

/// One edit, from the editor to the graph.
///
/// Field paths are relative to the part, exactly as an edge's are:
/// `"frequency"`, not `"inlets.frequency"`.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphCommand {
    /// Adds a node of a registered kind, named by its reflected type path.
    Create {
        /// The node kind's `TypePath::type_path()`.
        kind: String,
        /// Where the editor drops it.
        pos: Vec2,
    },
    /// Removes a node and every edge naming it.
    Delete {
        /// The node to remove.
        node: NodeId,
    },
    /// Writes one authored inlet field.
    SetField {
        /// The node to edit.
        node: NodeId,
        /// A path relative to the node's `inlets`.
        path: String,
        /// The new value.
        value: FieldValue,
    },
    /// Moves a node on the canvas.
    Move {
        /// The node to move.
        node: NodeId,
        /// Its new canvas position.
        pos: Vec2,
    },
    /// Connects an outlet to an inlet.
    Connect {
        /// The producing node and a path within its `outlets`.
        src: Port,
        /// The consuming node and a path within its `inlets`.
        dst: Port,
        /// The ordering key. `0` for a non-variadic inlet.
        slot: i32,
    },
    /// Removes one edge.
    Disconnect {
        /// The edge to remove.
        edge: EdgeId,
    },
    /// Changes one edge's ordering key, reordering a variadic inlet.
    SetSlot {
        /// The edge to reorder.
        edge: EdgeId,
        /// Its new ordering key.
        slot: i32,
    },
    /// Points the editor at a node, or at nothing.
    Select {
        /// The newly selected node.
        node: Option<NodeId>,
    },
}

/// What applying a command did. `Refused` carries the reason a connection was
/// rejected so the editor can surface it (task 7.6).
#[derive(Clone, Debug, PartialEq)]
pub enum CommandOutcome {
    /// The command changed the graph.
    Applied,
    /// The command was valid but changed nothing — an equal write, or a
    /// selection that was already current.
    NoChange,
    /// The command named something that does not exist, or a kind that is not
    /// registered.
    Unknown,
    /// A connection was refused by the invariants or the legality rule.
    Refused(ConnectError),
}

/// The receiving half, held by the world. Present only in an editor build.
#[derive(Resource)]
pub struct GraphRx(pub Receiver<GraphCommand>);

/// Drains every queued command.
///
/// Exclusive, because building a node needs the type registry and the graph at
/// once. Scheduled in `PreUpdate`, so this frame's edits are in the graph
/// before `FixedUpdate` ticks it.
pub fn apply_graph_commands(world: &mut World) {
    let Some(rx) = world.get_resource::<GraphRx>() else {
        return;
    };
    let commands: Vec<GraphCommand> = rx.0.try_iter().collect();
    if commands.is_empty() {
        return;
    }
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let registry = type_registry.read();
    let _ = world.try_resource_scope(
        |_world, mut graph: bevy_ecs::change_detection::Mut<Graph>| {
            for command in &commands {
                apply_graph_command(&mut graph, &registry, command);
            }
        },
    );
}

/// Applies one command. Split out from [`apply_graph_commands`] so the graph is
/// testable with no `World` and no channel.
pub fn apply_graph_command(
    graph: &mut Graph,
    registry: &TypeRegistry,
    command: &GraphCommand,
) -> CommandOutcome {
    match command {
        GraphCommand::Create { kind, pos } => {
            let Some(registration) = registry.get_with_type_path(kind) else {
                return CommandOutcome::Unknown;
            };
            // Only a registered *node kind* can be created: a bare reflected
            // type has no `evaluate` and no declared parts.
            if registration.data::<ReflectNodeKind>().is_none() {
                return CommandOutcome::Unknown;
            }
            let Some(reflect_default) = registration.data::<ReflectDefault>() else {
                return CommandOutcome::Unknown;
            };
            let type_path = registration.type_info().type_path();
            graph.insert(Node::new(type_path, *pos, reflect_default.default()));
            CommandOutcome::Applied
        }
        GraphCommand::Delete { node } => match graph.remove(*node) {
            Some(_) => CommandOutcome::Applied,
            None => CommandOutcome::Unknown,
        },
        GraphCommand::Move { node, pos } => {
            let Some(target) = graph.get_mut(*node) else {
                return CommandOutcome::Unknown;
            };
            if !target.set_pos(*pos) {
                return CommandOutcome::NoChange;
            }
            graph.mark_dirty(*node);
            CommandOutcome::Applied
        }
        GraphCommand::SetField {
            node,
            path: field,
            value,
        } => {
            let Some(target) = graph.get_mut(*node) else {
                return CommandOutcome::Unknown;
            };
            let Some(field) = path::resolve_mut(target, Part::Inlets, field) else {
                return CommandOutcome::Unknown;
            };
            match write_field(field, value) {
                WriteOutcome::Changed => {
                    graph.mark_dirty(*node);
                    CommandOutcome::Applied
                }
                WriteOutcome::Equal => CommandOutcome::NoChange,
                WriteOutcome::Rejected => CommandOutcome::Unknown,
            }
        }
        GraphCommand::Connect { src, dst, slot } => {
            match graph.connect(src.clone(), dst.clone(), *slot) {
                Ok(_) => CommandOutcome::Applied,
                Err(error) => CommandOutcome::Refused(error),
            }
        }
        GraphCommand::Disconnect { edge } => {
            if graph.disconnect(*edge) {
                CommandOutcome::Applied
            } else {
                CommandOutcome::Unknown
            }
        }
        GraphCommand::SetSlot { edge, slot } => {
            if graph.edge(*edge).is_none() {
                return CommandOutcome::Unknown;
            }
            if graph.set_slot(*edge, *slot) {
                CommandOutcome::Applied
            } else {
                CommandOutcome::NoChange
            }
        }
        GraphCommand::Select { node } => {
            if graph.selection() == *node {
                return CommandOutcome::NoChange;
            }
            graph.set_selection(*node);
            CommandOutcome::Applied
        }
    }
}

enum WriteOutcome {
    Changed,
    Equal,
    Rejected,
}

/// Boxes an inspector integer as the concrete integer type of the field it is
/// about to be applied to.
///
/// Out-of-range values **saturate** rather than being dropped. A dropped write
/// looks identical to a UI that ignored the keystroke, because the inspector
/// re-reads the unchanged field and snaps back. Note the cast must be explicit
/// — `-1i64 as u32` wraps to `u32::MAX`, which would be a far worse answer
/// than `0`.
fn int_as(existing: &dyn PartialReflect, value: i64) -> Option<Box<dyn PartialReflect>> {
    macro_rules! narrow {
        ($($t:ty),+ $(,)?) => {$(
            if existing.try_downcast_ref::<$t>().is_some() {
                let saturated = <$t>::try_from(value)
                    .unwrap_or(if value < 0 { <$t>::MIN } else { <$t>::MAX });
                return Some(Box::new(saturated));
            }
        )+};
    }
    narrow!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);
    None
}

fn boxed_for(existing: &dyn PartialReflect, value: &FieldValue) -> Option<Box<dyn PartialReflect>> {
    match value {
        FieldValue::Float(number) => {
            if existing.try_downcast_ref::<f32>().is_some() {
                Some(Box::new(*number as f32))
            } else if existing.try_downcast_ref::<f64>().is_some() {
                Some(Box::new(*number))
            } else {
                None
            }
        }
        FieldValue::Int(number) => int_as(existing, *number),
        FieldValue::Bool(flag) => existing
            .try_downcast_ref::<bool>()
            .map(|_| Box::new(*flag) as Box<dyn PartialReflect>),
        FieldValue::Str(text) => existing
            .try_downcast_ref::<String>()
            .map(|_| Box::new(text.clone()) as Box<dyn PartialReflect>),
        FieldValue::Vec2(v) => existing
            .try_downcast_ref::<Vec2>()
            .map(|_| Box::new(*v) as Box<dyn PartialReflect>),
        FieldValue::Vec3(v) => existing
            .try_downcast_ref::<Vec3>()
            .map(|_| Box::new(*v) as Box<dyn PartialReflect>),
        FieldValue::Quat(q) => existing
            .try_downcast_ref::<Quat>()
            .map(|_| Box::new(*q) as Box<dyn PartialReflect>),
        FieldValue::Enum(variant) => {
            // A unit variant, by name. `try_apply` on an enum switches variant.
            Some(Box::new(DynamicEnum::new(
                variant.clone(),
                DynamicVariant::Unit,
            )))
        }
    }
}

fn write_field(field: &mut dyn PartialReflect, value: &FieldValue) -> WriteOutcome {
    let Some(boxed) = boxed_for(field, value) else {
        return WriteOutcome::Rejected;
    };
    // Never write an equal value (architecture §7): an equal write must not
    // dirty the node.
    if reflect_equal(field, boxed.as_ref()) {
        return WriteOutcome::Equal;
    }
    match field.try_apply(boxed.as_ref()) {
        Ok(()) => WriteOutcome::Changed,
        Err(_) => WriteOutcome::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Source, test_registry};

    fn graph_with_two() -> (Graph, TypeRegistry, NodeId, NodeId) {
        let registry = test_registry();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        graph.drain_dirty();
        (graph, registry, a, b)
    }

    #[test]
    fn create_adds_a_node_of_the_named_kind() {
        let registry = test_registry();
        let mut graph = Graph::default();
        let outcome = apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::Create {
                kind: "sway_graph::graph::testing::Counter".into(),
                pos: Vec2::new(5.0, 6.0),
            },
        );

        assert_eq!(outcome, CommandOutcome::Applied);
        let (id, node) = graph.iter().next().expect("one node");
        assert_eq!(node.kind(), "sway_graph::graph::testing::Counter");
        assert_eq!(node.pos(), Vec2::new(5.0, 6.0));
        assert_eq!(graph.drain_dirty(), vec![id]);
    }

    #[test]
    fn create_refuses_a_type_that_is_not_a_node_kind() {
        let registry = test_registry();
        let mut graph = Graph::default();
        // `Step` is registered, but only as a part type.
        let outcome = apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::Create {
                kind: "sway_graph::graph::testing::Step".into(),
                pos: Vec2::ZERO,
            },
        );
        assert_eq!(outcome, CommandOutcome::Unknown);
        assert!(graph.is_empty());
    }

    #[test]
    fn set_field_writes_one_inlet_and_dirties_only_that_node() {
        let (mut graph, registry, a, b) = graph_with_two();
        let outcome = apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::SetField {
                node: b,
                path: "step".into(),
                value: FieldValue::Float(2.5),
            },
        );

        assert_eq!(outcome, CommandOutcome::Applied);
        assert_eq!(graph.drain_dirty(), vec![b], "{a} is untouched");
    }

    #[test]
    fn an_equal_set_field_reports_nothing() {
        let (mut graph, registry, _, b) = graph_with_two();
        let command = GraphCommand::SetField {
            node: b,
            path: "step".into(),
            value: FieldValue::Float(0.0),
        };
        assert_eq!(
            apply_graph_command(&mut graph, &registry, &command),
            CommandOutcome::NoChange
        );
        assert!(graph.drain_dirty().is_empty());
    }

    #[test]
    fn set_field_addresses_a_nested_path() {
        use crate::graph::testing::Nested;
        let registry = test_registry();
        let mut graph = Graph::default();
        let id = graph.insert(Node::of(Vec2::ZERO, Nested::default()));
        graph.drain_dirty();

        apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::SetField {
                node: id,
                path: "point.y".into(),
                value: FieldValue::Float(4.0),
            },
        );

        let point = path::resolve(graph.get(id).unwrap(), Part::Inlets, "point").unwrap();
        assert_eq!(point.reflect_partial_eq(&Vec2::new(0.0, 4.0)), Some(true));
    }

    #[test]
    fn an_out_of_range_integer_saturates_rather_than_being_dropped() {
        #[derive(bevy_reflect::Reflect, Default, Debug)]
        struct Narrow {
            small: u8,
        }
        let mut value = Narrow::default();
        let field: &mut dyn PartialReflect = &mut value.small;
        assert!(matches!(
            write_field(field, &FieldValue::Int(-5)),
            WriteOutcome::Equal
        ));
        let field: &mut dyn PartialReflect = &mut value.small;
        assert!(matches!(
            write_field(field, &FieldValue::Int(9999)),
            WriteOutcome::Changed
        ));
        assert_eq!(value.small, u8::MAX);
    }

    #[test]
    fn move_dirties_only_on_a_real_move() {
        let (mut graph, registry, _, b) = graph_with_two();
        let command = GraphCommand::Move {
            node: b,
            pos: Vec2::ZERO,
        };
        assert_eq!(
            apply_graph_command(&mut graph, &registry, &command),
            CommandOutcome::NoChange
        );
        let command = GraphCommand::Move {
            node: b,
            pos: Vec2::new(1.0, 0.0),
        };
        assert_eq!(
            apply_graph_command(&mut graph, &registry, &command),
            CommandOutcome::Applied
        );
        assert_eq!(graph.drain_dirty(), vec![b]);
    }

    #[test]
    fn connect_surfaces_the_reason_it_was_refused() {
        let (mut graph, registry, a, b) = graph_with_two();
        let outcome = apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::Connect {
                src: Port::new(a, "out"),
                dst: Port::new(b, "nope"),
                slot: 0,
            },
        );
        assert_eq!(
            outcome,
            CommandOutcome::Refused(ConnectError::MissingDestinationPath)
        );
    }

    #[test]
    fn connect_then_disconnect_round_trips() {
        let (mut graph, registry, a, b) = graph_with_two();
        apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::Connect {
                src: Port::new(a, "out"),
                dst: Port::new(b, "step"),
                slot: 0,
            },
        );
        let edge = graph.edges()[0].id;
        assert_eq!(
            apply_graph_command(&mut graph, &registry, &GraphCommand::Disconnect { edge }),
            CommandOutcome::Applied
        );
        assert!(graph.edges().is_empty());
    }

    #[test]
    fn delete_removes_the_node_and_its_edges() {
        let (mut graph, registry, a, b) = graph_with_two();
        apply_graph_command(
            &mut graph,
            &registry,
            &GraphCommand::Connect {
                src: Port::new(a, "out"),
                dst: Port::new(b, "step"),
                slot: 0,
            },
        );
        assert_eq!(
            apply_graph_command(&mut graph, &registry, &GraphCommand::Delete { node: a }),
            CommandOutcome::Applied
        );
        assert!(graph.get(a).is_none());
        assert!(graph.edges().is_empty());
        assert_eq!(graph.drain_removed(), vec![a]);
    }

    #[test]
    fn select_points_at_a_node() {
        let (mut graph, registry, a, _) = graph_with_two();
        assert_eq!(
            apply_graph_command(
                &mut graph,
                &registry,
                &GraphCommand::Select { node: Some(a) }
            ),
            CommandOutcome::Applied
        );
        assert_eq!(graph.selection(), Some(a));
        assert_eq!(
            apply_graph_command(
                &mut graph,
                &registry,
                &GraphCommand::Select { node: Some(a) }
            ),
            CommandOutcome::NoChange
        );
        assert!(
            graph.drain_dirty().is_empty(),
            "selection is not node content"
        );
    }

    #[test]
    fn set_slot_reorders_one_edge() {
        use crate::graph::testing::Fan;
        let registry = test_registry();
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let edge = graph
            .connect(Port::new(a, "out"), Port::new(fan, "values"), 10)
            .expect("legal");

        assert_eq!(
            apply_graph_command(
                &mut graph,
                &registry,
                &GraphCommand::SetSlot { edge, slot: 20 }
            ),
            CommandOutcome::Applied
        );
        assert_eq!(graph.edge(edge).unwrap().slot, 20);
    }
}
