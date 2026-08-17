//! The editor's write path. Spec M6-1.
//!
//! The editor produces plain data and sends it; this drains the channel and
//! mutates the world. `sway-editor` never sees a `World`, and nothing here
//! knows the document format exists.

use bevy_ecs::entity::Entity;
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::reflect::{AppTypeRegistry, ReflectComponent};
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{PartialReflect, ReflectMut, ReflectRef, TypeData};
use crossbeam_channel::Receiver;

use crate::ctx::{EditorPos, Selection};

/// Looks up one piece of `TypeData` (e.g. [`ReflectComponent`],
/// [`ReflectDefault`]) for a document-registered component, by its
/// `ComponentDocRegistry` name. `None` covers every way this can fail: the
/// name isn't registered, or the registered type doesn't carry `T`.
///
/// `T` is returned by value (not a reference into the registry) because
/// `AppTypeRegistry`'s read guard borrows the `TypeRegistryArc` clone made
/// inside this function; that clone — and the guard — cannot outlive the
/// call. `TypeData` impls are cheap to clone (function-pointer tables), so
/// this costs nothing callers would have avoided by borrowing instead.
fn reflect_data_for<T: TypeData + Clone>(world: &World, name: &str) -> Option<T> {
    let type_id = world
        .get_resource::<crate::ComponentDocRegistry>()?
        .by_name(name)?
        .type_id;
    let type_registry = world.get_resource::<AppTypeRegistry>()?.clone();
    let registry = type_registry.read();
    registry.get(type_id)?.data::<T>().cloned()
}

/// One edited field's new value. Deliberately not `Box<dyn Reflect>`: the
/// channel payload stays `Send` and plainly comparable, and the applier does
/// the reflect work on the world side where the type registry is in hand.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Float(f32),
    Int(i64),
    Bool(bool),
    /// A unit enum variant, by name.
    Enum(String),
    Str(String),
    Vec2(Vec2),
    Vec3(Vec3),
}

/// One edit, from the editor to the world.
///
/// `component` is a `ComponentDocRegistry` short name. `wire` is the
/// reflected type path of a wire type (`TypePath::type_path()`).
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    Create {
        component: &'static str,
        pos: Vec2,
    },
    Delete {
        entity: Entity,
    },
    SetField {
        entity: Entity,
        component: &'static str,
        field: String,
        value: FieldValue,
    },
    MoveNode {
        entity: Entity,
        pos: Vec2,
    },
    Connect {
        wire: &'static str,
        src: Entity,
        dst: Entity,
    },
    Disconnect {
        wire: &'static str,
        dst: Entity,
    },
    Select {
        entity: Option<Entity>,
    },
}

/// The receiving half, held by the world. Present only in an editor build.
#[derive(Resource)]
pub struct EditorRx(pub Receiver<EditorCommand>);

/// Drains every queued command. Exclusive, because applying spawns, despawns
/// and inserts relationship components.
///
/// Scheduled in `PreUpdate` **before** `WatchSet`, so this frame's rewires are
/// seen by the per-wire topology watches and mark `TopologyDirty`; the rebuild
/// then happens in the following `FixedUpdate` exactly as it does for a
/// document reload.
pub fn apply_editor_commands(world: &mut World) {
    let Some(rx) = world.get_resource::<EditorRx>() else {
        return;
    };
    let commands: Vec<EditorCommand> = rx.0.try_iter().collect();
    for command in &commands {
        apply_editor_command(world, command);
    }
}

/// Boxes an inspector integer as the concrete integer type of the field it is
/// about to be applied to.
///
/// The inspector has exactly one integer commit path — parse the typed text as
/// `i64`, send `FieldValue::Int` — but the field on the other end may be any
/// width the snapshot's `kind_of` classifies as an integer. Reflection matches
/// on the concrete type, so the value has to be narrowed here or the write is
/// rejected.
///
/// Out-of-range values **saturate** rather than being dropped. A dropped write
/// is the failure this function exists to fix: it looks identical to a UI that
/// ignored the keystroke, because the inspector re-reads the unchanged field
/// and snaps back. Saturating lands on the nearest representable value, which
/// is visible immediately. Note the cast must be explicit — `-1i64 as u32`
/// wraps to `u32::MAX`, which would be a far worse answer than `0`.
///
/// `None` for a field that is not an integer at all, which the caller turns
/// into a no-op.
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

/// One command. Split out from [`apply_editor_commands`] so tests can drive it
/// directly without a channel.
pub fn apply_editor_command(world: &mut World, command: &EditorCommand) {
    match command {
        EditorCommand::MoveNode { entity, pos } => {
            let Ok(mut entity_mut) = world.get_entity_mut(*entity) else {
                return;
            };
            let Some(mut editor_pos) = entity_mut.get_mut::<EditorPos>() else {
                return;
            };
            // Never write an equal value (architecture §7).
            if editor_pos.0 != *pos {
                editor_pos.0 = *pos;
            }
        }
        EditorCommand::Create { component, pos } => {
            // An unregistered name, or one missing either piece of reflect
            // data, is a no-op rather than a panic.
            let Some(reflect_component) = reflect_data_for::<ReflectComponent>(world, component)
            else {
                return;
            };
            let Some(reflect_default) = reflect_data_for::<ReflectDefault>(world, component) else {
                return;
            };
            let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
                return;
            };

            let entity = world.spawn(EditorPos(*pos)).id();
            {
                // `AppTypeRegistry` is cloned above (it is an Arc) so the read
                // guard does not borrow `world` while the world is mutated.
                let registry = type_registry.read();
                let value = reflect_default.default();
                let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
                    return;
                };
                reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &registry);
            }
            // `EditorPos` is inserted before the component so a component that
            // `#[require]`s it does not overwrite the click position with a
            // default. Re-assert it afterwards in case it did.
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                entity_mut.insert(EditorPos(*pos));
            }
        }
        EditorCommand::Delete { entity } => {
            let Ok(entity_ref) = world.get_entity(*entity) else {
                return;
            };
            let parent = entity_ref.get::<ChildOf>().map(|c| c.0);
            let children: Vec<Entity> = entity_ref
                .get::<Children>()
                .map(|c| c.iter().copied().collect())
                .unwrap_or_default();

            for child in children {
                let Ok(mut child_mut) = world.get_entity_mut(child) else {
                    continue;
                };
                match parent {
                    Some(grandparent) => {
                        child_mut.insert(ChildOf(grandparent));
                    }
                    None => {
                        child_mut.remove::<ChildOf>();
                    }
                }
            }
            world.despawn(*entity);
            if let Some(mut selection) = world.get_resource_mut::<Selection>()
                && selection.0 == Some(*entity)
            {
                selection.0 = None;
            }
        }
        EditorCommand::SetField {
            entity,
            component,
            field,
            value,
        } => {
            let Some(reflect_component) = reflect_data_for::<ReflectComponent>(world, component)
            else {
                return;
            };
            let Ok(entity_ref) = world.get_entity(*entity) else {
                return;
            };

            // Reach the field through an immutable reflect first: taking a
            // `Mut` via `reflect_mut` marks `Changed` on deref regardless of
            // whether a write follows, so the equal-value no-op has to be
            // decided before any mutable borrow is taken.
            let Some(reflected) = reflect_component.reflect(entity_ref) else {
                return;
            };
            let existing = match reflected.reflect_ref() {
                ReflectRef::Struct(target) => target.field(field),
                ReflectRef::TupleStruct(target) => field
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| target.field(index)),
                _ => return,
            };
            let Some(existing) = existing else {
                return;
            };

            let replacement: Box<dyn PartialReflect> = match value {
                FieldValue::Float(v) => Box::new(*v),
                // Boxed as the field's own integer type, not as `i64`.
                // Reflection matches on the concrete type, so a boxed `i64`
                // applied to a `u32` field is a mismatch that `try_apply`
                // rejects — and the rejection is discarded below, which
                // turned every non-`i64` integer edit into a silent no-op.
                FieldValue::Int(v) => match int_as(existing, *v) {
                    Some(boxed) => boxed,
                    None => return,
                },
                FieldValue::Bool(v) => Box::new(*v),
                FieldValue::Str(v) => Box::new(v.clone()),
                FieldValue::Vec2(v) => Box::new(*v),
                FieldValue::Vec3(v) => Box::new(*v),
                FieldValue::Enum(variant) => {
                    // A unit variant is addressed by name against the field's
                    // own static type info, so the caller never needs the
                    // type path, and the variant name is validated to exist
                    // before a `DynamicEnum` naming it is ever constructed.
                    let Some(bevy_reflect::TypeInfo::Enum(enum_info)) =
                        existing.get_represented_type_info()
                    else {
                        return;
                    };
                    if !enum_info.contains_variant(variant.as_str()) {
                        return;
                    }
                    // `DynamicEnum` names the variant directly; applying it to
                    // the concrete field converts it back.
                    Box::new(bevy_reflect::enums::DynamicEnum::new(
                        variant.clone(),
                        bevy_reflect::enums::DynamicVariant::Unit,
                    ))
                }
            };

            // Type mismatch and equal-value are both no-ops.
            if existing.reflect_partial_eq(replacement.as_ref()) == Some(true) {
                return;
            }

            let Ok(entity_mut) = world.get_entity_mut(*entity) else {
                return;
            };
            let Some(mut reflected) = reflect_component.reflect_mut(entity_mut) else {
                return;
            };
            let existing = match reflected.reflect_mut() {
                ReflectMut::Struct(target) => target.field_mut(field),
                ReflectMut::TupleStruct(target) => field
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| target.field_mut(index)),
                _ => return,
            };
            let Some(existing) = existing else {
                return;
            };
            // A failed apply here would mean the equal-value check above
            // passed a value `try_apply` then rejects; nothing else follows
            // in this arm, so the error is simply dropped.
            let _ = existing.try_apply(replacement.as_ref());
        }
        EditorCommand::Connect { wire, src, dst } => {
            if !crate::dispatch::connect_is_legal(world, wire, *src, *dst) {
                return;
            }
            crate::dispatch::insert_wire(world, wire, *dst, *src);
        }
        EditorCommand::Disconnect { wire, dst } => {
            crate::dispatch::remove_wire(world, wire, *dst);
        }
        EditorCommand::Select { entity } => {
            // A selection naming a despawned entity is a no-op rather than a
            // stale pointer.
            let entity = entity.filter(|e| world.get_entity(*e).is_ok());
            let Some(mut selection) = world.get_resource_mut::<Selection>() else {
                return;
            };
            if selection.0 != entity {
                selection.0 = entity;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::EditorPos;
    use crate::run::WiresPlugin;
    use crate::watch::Authoring;
    use bevy_app::App;
    use bevy_ecs::change_detection::DetectChanges;
    use bevy_math::Vec2;
    use bevy_time::{Fixed, Time};

    fn command_app() -> (App, crossbeam_channel::Sender<EditorCommand>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1))
            .insert_resource(Authoring)
            .insert_resource(EditorRx(rx))
            .add_plugins(WiresPlugin);
        // Two updates: frame 0 only primes the fixed-time accumulator.
        app.update();
        app.update();
        (app, tx)
    }

    #[test]
    fn a_move_node_command_writes_editor_pos() {
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        tx.send(EditorCommand::MoveNode {
            entity,
            pos: Vec2::new(40.0, 90.0),
        })
        .expect("the receiver is alive in the world");
        app.update();

        assert_eq!(
            app.world().get::<EditorPos>(entity).map(|p| p.0),
            Some(Vec2::new(40.0, 90.0)),
        );
    }

    #[test]
    fn an_unchanged_position_does_not_mark_the_component_changed() {
        // Global constraint: never write an equal value.
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::new(7.0, 7.0))).id();
        app.update();

        tx.send(EditorCommand::MoveNode {
            entity,
            pos: Vec2::new(7.0, 7.0),
        })
        .unwrap();
        app.update();

        assert!(
            !app.world()
                .entity(entity)
                .get_ref::<EditorPos>()
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn a_command_naming_a_dead_entity_is_ignored_not_a_panic() {
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        app.world_mut().despawn(entity);

        tx.send(EditorCommand::MoveNode {
            entity,
            pos: Vec2::ONE,
        })
        .unwrap();
        app.update();
    }

    use bevy_ecs::component::Component;
    use bevy_ecs::hierarchy::ChildOf;
    use bevy_reflect::Reflect;

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Blip(f32);

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    #[require(Blip, EditorPos)]
    struct Widget {
        size: f32,
    }

    fn registry_app() -> App {
        let (_, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(Authoring)
            .insert_resource(EditorRx(rx))
            .add_plugins(WiresPlugin);
        crate::register_authorable::<Widget>(&mut app, "Widget");
        crate::register_authorable::<Blip>(&mut app, "Blip");
        crate::register_authorable::<EditorPos>(&mut app, "EditorPos");
        app
    }

    #[test]
    fn create_spawns_the_component_its_requires_and_an_editor_pos() {
        let mut app = registry_app();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create {
                component: "Widget",
                pos: Vec2::new(12.0, 34.0),
            },
        );

        let entity = app
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<Widget>>()
            .single(app.world())
            .expect("exactly one Widget was created");
        assert!(
            app.world().get::<Blip>(entity).is_some(),
            "#[require] supplied Blip"
        );
        assert_eq!(
            app.world().get::<EditorPos>(entity).map(|p| p.0),
            Some(Vec2::new(12.0, 34.0)),
            "the palette's click position becomes the canvas position",
        );
    }

    #[test]
    fn create_uses_the_components_reflect_default() {
        let mut app = registry_app();
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create {
                component: "Widget",
                pos: Vec2::ZERO,
            },
        );
        let entity = app
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<Widget>>()
            .single(app.world())
            .unwrap();
        assert_eq!(app.world().get::<Widget>(entity), Some(&Widget::default()));
    }

    #[test]
    fn create_with_an_unregistered_name_does_nothing() {
        let mut app = registry_app();
        let before = app.world().entities().len();
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Create {
                component: "Nonexistent",
                pos: Vec2::ZERO,
            },
        );
        assert_eq!(app.world().entities().len(), before);
    }

    #[test]
    fn delete_reparents_children_to_the_grandparent_before_despawning() {
        // Bevy's despawn cascades through Children, so a child would be
        // destroyed with its parent unless it is moved first.
        let mut app = registry_app();
        let grandparent = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let parent = app
            .world_mut()
            .spawn((EditorPos(Vec2::ZERO), ChildOf(grandparent)))
            .id();
        let child = app
            .world_mut()
            .spawn((EditorPos(Vec2::ZERO), ChildOf(parent)))
            .id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(
            app.world().get_entity(parent).is_err(),
            "the target despawned"
        );
        assert!(app.world().get_entity(child).is_ok(), "the child survived");
        assert_eq!(
            app.world().get::<ChildOf>(child).map(|c| c.0),
            Some(grandparent),
            "the child was reparented to its grandparent",
        );
    }

    #[test]
    fn deleting_a_root_makes_its_children_roots() {
        let mut app = registry_app();
        let parent = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let child = app
            .world_mut()
            .spawn((EditorPos(Vec2::ZERO), ChildOf(parent)))
            .id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(app.world().get_entity(child).is_ok());
        assert!(app.world().get::<ChildOf>(child).is_none());
    }

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Knobs {
        gain: f32,
        steps: i64,
        /// A second integer width, deliberately not `i64`. The inspector
        /// classifies every integer field as `FieldKind::Int` and commits it
        /// as `i64`, so `i64` is the one width where the reflected write
        /// happens to land on a matching type — testing only that width
        /// hides the failure for every other one.
        subdivisions: u32,
        origin: Vec2,
        on: bool,
    }

    fn knobs_app() -> App {
        let mut app = registry_app();
        crate::register_authorable::<Knobs>(&mut app, "Knobs");
        app
    }

    #[test]
    fn set_field_writes_a_float_through_reflection() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "gain".to_string(),
                value: FieldValue::Float(0.75),
            },
        );

        assert_eq!(app.world().get::<Knobs>(entity).map(|k| k.gain), Some(0.75));
    }

    #[test]
    fn set_field_writes_ints_and_bools() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "steps".to_string(),
                value: FieldValue::Int(9),
            },
        );
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "on".to_string(),
                value: FieldValue::Bool(true),
            },
        );

        let knobs = app.world().get::<Knobs>(entity).copied().unwrap();
        assert_eq!(knobs.steps, 9);
        assert!(knobs.on);
    }

    #[test]
    fn set_field_writes_an_int_whose_field_is_not_i64() {
        // The inspector has one integer commit path: parse as `i64`, send
        // `FieldValue::Int`. The field it lands on can be any integer width
        // `kind_of` accepts (`i32`, `u32`, `usize`, ...). Applying a boxed
        // `i64` to a `u32` field is a reflected type mismatch, `try_apply`
        // returns `Err`, and the error is discarded — so the edit silently
        // does nothing and the inspector snaps back to the old value on the
        // next refresh. First reachable through `PlaneMesh`'s `horizontal` /
        // `vertical` subdivision counts, which are `u32`.
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "subdivisions".to_string(),
                value: FieldValue::Int(31),
            },
        );

        assert_eq!(
            app.world().get::<Knobs>(entity).map(|k| k.subdivisions),
            Some(31)
        );
    }

    #[test]
    fn set_field_writes_a_vec2() {
        // `Vec2` is a distinct reflected type from `Vec3`, so the two-
        // component case needs its own `FieldValue` arm — sending a `Vec3`
        // with a spare zero would be the same silent type mismatch that made
        // integer edits vanish. First reachable through `PlaneMesh`'s `size`.
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "origin".to_string(),
                value: FieldValue::Vec2(Vec2::new(1.5, -2.5)),
            },
        );

        assert_eq!(
            app.world().get::<Knobs>(entity).map(|k| k.origin),
            Some(Vec2::new(1.5, -2.5))
        );
    }

    #[test]
    fn an_out_of_range_int_saturates_rather_than_wrapping_or_vanishing() {
        // Nothing stops an author typing a negative subdivision count. The
        // two ways to get this wrong are both worse than clamping: `as`
        // would wrap -1 into u32::MAX (a 4-billion-segment mesh), and
        // dropping the write is the silent no-op this whole fix removes.
        let mut app = knobs_app();
        let entity = app
            .world_mut()
            .spawn(Knobs {
                subdivisions: 7,
                ..Default::default()
            })
            .id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "subdivisions".to_string(),
                value: FieldValue::Int(-1),
            },
        );

        assert_eq!(
            app.world().get::<Knobs>(entity).map(|k| k.subdivisions),
            Some(0),
            "a negative count clamps to zero, never wraps to u32::MAX"
        );
    }

    #[test]
    fn writing_an_equal_value_does_not_mark_the_component_changed() {
        let mut app = knobs_app();
        let entity = app
            .world_mut()
            .spawn(Knobs {
                gain: 0.5,
                ..Default::default()
            })
            .id();
        app.update();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "gain".to_string(),
                value: FieldValue::Float(0.5),
            },
        );

        assert!(
            !app.world()
                .entity(entity)
                .get_ref::<Knobs>()
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn a_type_mismatch_leaves_the_field_alone() {
        let mut app = knobs_app();
        let entity = app
            .world_mut()
            .spawn(Knobs {
                gain: 0.25,
                ..Default::default()
            })
            .id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "gain".to_string(),
                value: FieldValue::Bool(true),
            },
        );

        assert_eq!(app.world().get::<Knobs>(entity).map(|k| k.gain), Some(0.25));
    }

    #[test]
    fn an_unknown_field_name_is_ignored() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "Knobs",
                field: "nope".to_string(),
                value: FieldValue::Float(1.0),
            },
        );
    }

    // Mirrors `sway_nodes::FloatOut(pub f32)`: a tuple-struct authorable
    // component. `sway-graph` cannot depend on `sway-nodes` (crate
    // boundary), so this is a local stand-in with the same shape.
    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct FloatOut(pub f32);

    fn float_out_app() -> App {
        let mut app = registry_app();
        crate::register_authorable::<FloatOut>(&mut app, "FloatOut");
        app
    }

    #[test]
    fn set_field_writes_through_a_tuple_struct_field() {
        // Important #1: the inspector names tuple-struct fields by index
        // ("0", "1", ...); the applier must resolve those through
        // `ReflectRef::TupleStruct`/`ReflectMut::TupleStruct`, not only
        // `Struct`.
        let mut app = float_out_app();
        let entity = app.world_mut().spawn(FloatOut(0.0)).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "FloatOut",
                field: "0".to_string(),
                value: FieldValue::Float(0.75),
            },
        );

        assert_eq!(app.world().get::<FloatOut>(entity).map(|f| f.0), Some(0.75));
    }

    #[test]
    fn writing_an_equal_tuple_struct_value_does_not_mark_the_component_changed() {
        let mut app = float_out_app();
        let entity = app.world_mut().spawn(FloatOut(0.5)).id();
        app.update();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::SetField {
                entity,
                component: "FloatOut",
                field: "0".to_string(),
                value: FieldValue::Float(0.5),
            },
        );

        assert!(
            !app.world()
                .entity(entity)
                .get_ref::<FloatOut>()
                .unwrap()
                .is_changed()
        );
    }

    use crate::test_wires::{GainFrom, spawn_float, spawn_gain};
    use bevy_reflect::TypePath;

    fn wired_app() -> App {
        let mut app = registry_app();
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<crate::test_wires::Gain>();
        crate::register_wire_type::<GainFrom>(&mut app);
        app
    }

    fn factor_wire() -> &'static str {
        GainFrom::type_path()
    }

    #[test]
    fn connect_inserts_the_wire() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src,
                dst,
            },
        );

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn connect_replaces_an_existing_source_without_a_disconnect_first() {
        let mut app = wired_app();
        let first = spawn_float(app.world_mut(), 1.0);
        let second = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src: first,
                dst,
            },
        );
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src: second,
                dst,
            },
        );

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(second));
    }

    #[test]
    fn connect_refuses_a_source_without_the_source_component() {
        let mut app = wired_app();
        let not_a_source = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src: not_a_source,
                dst,
            },
        );

        assert!(
            app.world().get::<GainFrom>(dst).is_none(),
            "legality is enforced world-side too"
        );
    }

    #[test]
    fn connect_refuses_a_target_without_the_target_component() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let not_a_target = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src,
                dst: not_a_target,
            },
        );

        assert!(app.world().get::<GainFrom>(not_a_target).is_none());
    }

    #[test]
    fn disconnect_removes_the_wire_and_is_a_no_op_when_absent() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect {
                wire: factor_wire(),
                src,
                dst,
            },
        );

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Disconnect {
                wire: factor_wire(),
                dst,
            },
        );
        assert!(app.world().get::<GainFrom>(dst).is_none());

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Disconnect {
                wire: factor_wire(),
                dst,
            },
        );
    }

    #[test]
    fn a_connect_marks_the_topology_dirty_for_the_next_rebuild() {
        // The ordering guarantee M6-1 rests on: apply_editor_commands runs
        // before WatchSet, so the watch sees this frame's insert.
        let (mut app, tx) = command_app();
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<crate::test_wires::Gain>();
        crate::register_wire_type::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.update();
        app.update();

        tx.send(EditorCommand::Connect {
            wire: factor_wire(),
            src,
            dst,
        })
        .unwrap();
        app.update();

        assert_eq!(
            app.world().resource::<crate::GraphOrder>().steps.len(),
            1,
            "the new edge reached the order in the same frame the command arrived",
        );
    }

    #[test]
    fn select_sets_the_selection_resource() {
        let mut world = World::new();
        world.init_resource::<crate::Selection>();
        let entity = world.spawn_empty().id();

        apply_editor_command(
            &mut world,
            &EditorCommand::Select {
                entity: Some(entity),
            },
        );

        assert_eq!(world.resource::<crate::Selection>().0, Some(entity));
    }

    #[test]
    fn selecting_nothing_clears_it() {
        let mut world = World::new();
        let entity = world.spawn_empty().id();
        world.insert_resource(crate::Selection(Some(entity)));

        apply_editor_command(&mut world, &EditorCommand::Select { entity: None });

        assert_eq!(world.resource::<crate::Selection>().0, None);
    }

    #[test]
    fn deleting_the_selected_entity_clears_the_selection() {
        // Otherwise the inspector and the gizmo both keep pointing at a dead
        // entity, and the gizmo would draw at a stale transform.
        let mut world = World::new();
        world.init_resource::<crate::Selection>();
        let entity = world.spawn(EditorPos(Vec2::ZERO)).id();
        apply_editor_command(
            &mut world,
            &EditorCommand::Select {
                entity: Some(entity),
            },
        );

        apply_editor_command(&mut world, &EditorCommand::Delete { entity });

        assert_eq!(world.resource::<crate::Selection>().0, None);
    }
}
