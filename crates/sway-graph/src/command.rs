//! The editor's write path. Spec M6-1.
//!
//! The editor produces plain data and sends it; this drains the channel and
//! mutates the world. `sway-editor` never sees a `World`, and nothing here
//! knows the document format exists.

use bevy_ecs::entity::Entity;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use crossbeam_channel::Receiver;

use crate::ctx::EditorPos;

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
        // Tasks 3-5 fill these in.
        EditorCommand::Create { .. }
        | EditorCommand::Delete { .. }
        | EditorCommand::SetField { .. }
        | EditorCommand::Connect { .. }
        | EditorCommand::Disconnect { .. } => {}
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
}
