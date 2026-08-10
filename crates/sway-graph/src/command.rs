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

use crate::ctx::EditorPos;

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
    Vec3(Vec3),
}

/// One edit, from the editor to the world.
///
/// `component` and `wire` are the `&'static str` keys already carried by
/// `ComponentEntry::name` and `WireEntry::name`, so a command names a type
/// without carrying one.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    Create { component: &'static str, pos: Vec2 },
    Delete { entity: Entity },
    SetField { entity: Entity, component: &'static str, field: String, value: FieldValue },
    MoveNode { entity: Entity, pos: Vec2 },
    Connect { wire: &'static str, src: Entity, dst: Entity },
    Disconnect { wire: &'static str, dst: Entity },
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
            let Some(reflect_default) = reflect_data_for::<ReflectDefault>(world, component)
            else {
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
        }
        EditorCommand::SetField { entity, component, field, value } => {
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
            let ReflectRef::Struct(target) = reflected.reflect_ref() else {
                return;
            };
            let Some(existing) = target.field(field) else {
                return;
            };

            let replacement: Box<dyn PartialReflect> = match value {
                FieldValue::Float(v) => Box::new(*v),
                FieldValue::Int(v) => Box::new(*v),
                FieldValue::Bool(v) => Box::new(*v),
                FieldValue::Str(v) => Box::new(v.clone()),
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
            let ReflectMut::Struct(target) = reflected.reflect_mut() else {
                return;
            };
            let Some(existing) = target.field_mut(field) else {
                return;
            };
            // A failed apply here would mean the equal-value check above
            // passed a value `try_apply` then rejects; nothing else follows
            // in this arm, so the error is simply dropped.
            let _ = existing.try_apply(replacement.as_ref());
        }
        EditorCommand::Connect { wire, src, dst } => {
            let Some((insert, has_source, has_target)) = world
                .get_resource::<crate::WireRegistry>()
                .and_then(|r| r.entries.iter().find(|e| e.name == *wire))
                .map(|e| (e.insert, e.has_source, e.has_target))
            else {
                return;
            };
            // The editor filters illegal drops before sending, but a command
            // is data and may arrive stale — the world enforces it too.
            if !has_source(world, *src) || !has_target(world, *dst) {
                return;
            }
            insert(world, *dst, *src);
        }
        EditorCommand::Disconnect { wire, dst } => {
            let Some(remove) = world
                .get_resource::<crate::WireRegistry>()
                .and_then(|r| r.entries.iter().find(|e| e.name == *wire))
                .map(|e| e.remove)
            else {
                return;
            };
            remove(world, *dst);
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

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::new(40.0, 90.0) })
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

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::new(7.0, 7.0) }).unwrap();
        app.update();

        assert!(!app.world().entity(entity).get_ref::<EditorPos>().unwrap().is_changed());
    }

    #[test]
    fn a_command_naming_a_dead_entity_is_ignored_not_a_panic() {
        let (mut app, tx) = command_app();
        let entity = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        app.world_mut().despawn(entity);

        tx.send(EditorCommand::MoveNode { entity, pos: Vec2::ONE }).unwrap();
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
    struct Widget { size: f32 }

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
            &EditorCommand::Create { component: "Widget", pos: Vec2::new(12.0, 34.0) },
        );

        let entity = app
            .world_mut()
            .query_filtered::<Entity, bevy_ecs::query::With<Widget>>()
            .single(app.world())
            .expect("exactly one Widget was created");
        assert!(app.world().get::<Blip>(entity).is_some(), "#[require] supplied Blip");
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
            &EditorCommand::Create { component: "Widget", pos: Vec2::ZERO },
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
            &EditorCommand::Create { component: "Nonexistent", pos: Vec2::ZERO },
        );
        assert_eq!(app.world().entities().len(), before);
    }

    #[test]
    fn delete_reparents_children_to_the_grandparent_before_despawning() {
        // Bevy's despawn cascades through Children, so a child would be
        // destroyed with its parent unless it is moved first.
        let mut app = registry_app();
        let grandparent = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let parent = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(grandparent))).id();
        let child = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(parent))).id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(app.world().get_entity(parent).is_err(), "the target despawned");
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
        let child = app.world_mut().spawn((EditorPos(Vec2::ZERO), ChildOf(parent))).id();

        apply_editor_command(app.world_mut(), &EditorCommand::Delete { entity: parent });

        assert!(app.world().get_entity(child).is_ok());
        assert!(app.world().get::<ChildOf>(child).is_none());
    }

    #[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
    #[reflect(Component, Default, PartialEq)]
    struct Knobs { gain: f32, steps: i64, on: bool }

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

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "steps".to_string(), value: FieldValue::Int(9),
        });
        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "on".to_string(), value: FieldValue::Bool(true),
        });

        let knobs = app.world().get::<Knobs>(entity).copied().unwrap();
        assert_eq!(knobs.steps, 9);
        assert!(knobs.on);
    }

    #[test]
    fn writing_an_equal_value_does_not_mark_the_component_changed() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs { gain: 0.5, ..Default::default() }).id();
        app.update();

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "gain".to_string(), value: FieldValue::Float(0.5),
        });

        assert!(!app.world().entity(entity).get_ref::<Knobs>().unwrap().is_changed());
    }

    #[test]
    fn a_type_mismatch_leaves_the_field_alone() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs { gain: 0.25, ..Default::default() }).id();

        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity,
            component: "Knobs",
            field: "gain".to_string(),
            value: FieldValue::Bool(true),
        });

        assert_eq!(app.world().get::<Knobs>(entity).map(|k| k.gain), Some(0.25));
    }

    #[test]
    fn an_unknown_field_name_is_ignored() {
        let mut app = knobs_app();
        let entity = app.world_mut().spawn(Knobs::default()).id();
        apply_editor_command(app.world_mut(), &EditorCommand::SetField {
            entity, component: "Knobs", field: "nope".to_string(), value: FieldValue::Float(1.0),
        });
    }

    use crate::test_wires::{GainFrom, spawn_float, spawn_gain};

    fn wired_app() -> App {
        let mut app = registry_app();
        crate::register_wire::<GainFrom>(&mut app);
        app
    }

    #[test]
    fn connect_inserts_the_wire() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src, dst },
        );

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(src));
    }

    #[test]
    fn connect_replaces_an_existing_source_without_a_disconnect_first() {
        let mut app = wired_app();
        let first = spawn_float(app.world_mut(), 1.0);
        let second = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src: first, dst });
        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src: second, dst });

        assert_eq!(app.world().get::<GainFrom>(dst).map(|w| w.0), Some(second));
    }

    #[test]
    fn connect_refuses_a_source_without_the_source_component() {
        let mut app = wired_app();
        let not_a_source = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();
        let dst = spawn_gain(app.world_mut(), 0.0);

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src: not_a_source, dst },
        );

        assert!(app.world().get::<GainFrom>(dst).is_none(), "legality is enforced world-side too");
    }

    #[test]
    fn connect_refuses_a_target_without_the_target_component() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let not_a_target = app.world_mut().spawn(EditorPos(Vec2::ZERO)).id();

        apply_editor_command(
            app.world_mut(),
            &EditorCommand::Connect { wire: "factor", src, dst: not_a_target },
        );

        assert!(app.world().get::<GainFrom>(not_a_target).is_none());
    }

    #[test]
    fn disconnect_removes_the_wire_and_is_a_no_op_when_absent() {
        let mut app = wired_app();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        apply_editor_command(app.world_mut(), &EditorCommand::Connect { wire: "factor", src, dst });

        apply_editor_command(app.world_mut(), &EditorCommand::Disconnect { wire: "factor", dst });
        assert!(app.world().get::<GainFrom>(dst).is_none());

        apply_editor_command(app.world_mut(), &EditorCommand::Disconnect { wire: "factor", dst });
    }

    #[test]
    fn a_connect_marks_the_topology_dirty_for_the_next_rebuild() {
        // The ordering guarantee M6-1 rests on: apply_editor_commands runs
        // before WatchSet, so the watch sees this frame's insert.
        let (mut app, tx) = command_app();
        crate::register_wire::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.update();
        app.update();

        tx.send(EditorCommand::Connect { wire: "factor", src, dst }).unwrap();
        app.update();

        assert_eq!(
            app.world().resource::<crate::GraphOrder>().steps.len(),
            1,
            "the new edge reached the order in the same frame the command arrived",
        );
    }
}
