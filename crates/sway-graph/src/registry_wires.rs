//! The wire and behaviour registries. Spec §2.4, §3.1.
//!
//! The tick path never reads these — a `Step` carries its own fn pointer.
//! They exist for the rebuild and for the editor.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::query::With;
use bevy_ecs::relationship::Relationship;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::world::World;

use crate::ctx::TickCtx;
use crate::order::Link;
use crate::wire::{Wire, propagate_of};

/// Ordered logic attached to a component type. Spec §2.4: only for components
/// whose output depends on a wired inlet within the same tick.
pub type BehaviourFn = fn(&mut World, Entity, &TickCtx);

pub struct WireEntry {
    pub name: &'static str,
    /// Every instance of this wire type, as links.
    pub collect: fn(&mut World, &mut Vec<Link>),
    /// Whether an entity could be this wire's producer — the editor's
    /// legality rule.
    pub has_source: fn(&World, Entity) -> bool,
    /// Whether an entity could be this wire's consumer.
    pub has_target: fn(&World, Entity) -> bool,
    /// Wire `dst`'s inlet to `src`, replacing whatever was there. The project
    /// format's write path (project spec §3).
    pub insert: fn(&mut World, Entity, Entity),
    /// Disconnect `dst`'s inlet. A no-op if it is not connected.
    pub remove: fn(&mut World, Entity),
    /// The producer `dst`'s inlet currently names.
    pub read: fn(&World, Entity) -> Option<Entity>,
}

pub struct BehaviourEntry {
    pub name: &'static str,
    pub run: BehaviourFn,
    /// Entities carrying this behaviour's component.
    pub collect: fn(&mut World, &mut Vec<Entity>),
}

#[derive(Resource, Default)]
pub struct WireRegistry {
    pub entries: Vec<WireEntry>,
}

#[derive(Resource, Default)]
pub struct BehaviourRegistry {
    pub entries: Vec<BehaviourEntry>,
}

fn collect_wire_of<W: Wire>(world: &mut World, out: &mut Vec<Link>) {
    let mut query = world.query::<(Entity, &W)>();
    for (dst, wire) in query.iter(world) {
        out.push(Link {
            src: wire.get(),
            dst,
            run: propagate_of::<W>,
            wire: W::NAME,
        });
    }
}

fn insert_wire_of<W: Wire>(world: &mut World, dst: Entity, src: Entity) {
    if let Ok(mut entity) = world.get_entity_mut(dst) {
        entity.insert(W::from(src));
    }
}

fn remove_wire_of<W: Wire>(world: &mut World, dst: Entity) {
    if let Ok(mut entity) = world.get_entity_mut(dst) {
        entity.remove::<W>();
    }
}

fn read_wire_of<W: Wire>(world: &World, dst: Entity) -> Option<Entity> {
    world.get::<W>(dst).map(Relationship::get)
}

fn collect_behaviour_of<C: Component>(world: &mut World, out: &mut Vec<Entity>) {
    let mut query = world.query_filtered::<Entity, With<C>>();
    out.extend(query.iter(world));
}

pub fn register_wire<W: Wire>(app: &mut App) {
    app.add_systems(
        bevy_app::PreUpdate,
        crate::watch::watch::<W>.in_set(crate::watch::WatchSet),
    );
    app.init_resource::<WireRegistry>();
    app.world_mut()
        .resource_mut::<WireRegistry>()
        .entries
        .push(WireEntry {
            name: W::NAME,
            collect: collect_wire_of::<W>,
            has_source: |world, entity| {
                world
                    .get_entity(entity)
                    .is_ok_and(|e| e.contains::<W::Source>())
            },
            has_target: |world, entity| {
                world
                    .get_entity(entity)
                    .is_ok_and(|e| e.contains::<W::Target>())
            },
            insert: insert_wire_of::<W>,
            remove: remove_wire_of::<W>,
            read: read_wire_of::<W>,
        });
}

pub fn register_behaviour<C: Component>(app: &mut App, run: BehaviourFn) {
    app.init_resource::<BehaviourRegistry>();
    app.world_mut()
        .resource_mut::<BehaviourRegistry>()
        .entries
        .push(BehaviourEntry {
            name: core::any::type_name::<C>(),
            run,
            collect: collect_behaviour_of::<C>,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::TickCtx;
    use crate::test_wires::{Gain, GainFrom, spawn_float, spawn_gain};
    use bevy_app::App;

    fn noop_behaviour(_: &mut World, _: Entity, _: &TickCtx) {}

    #[test]
    fn registering_a_wire_records_its_name() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);

        let registry = app.world().resource::<WireRegistry>();
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].name, "factor");
    }

    #[test]
    fn collect_finds_every_instance_as_a_link() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let a = spawn_gain(app.world_mut(), 0.0);
        let b = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(a).insert(GainFrom(src));
        app.world_mut().entity_mut(b).insert(GainFrom(src));

        let collect = app.world().resource::<WireRegistry>().entries[0].collect;
        let mut links = Vec::new();
        collect(app.world_mut(), &mut links);

        let mut pairs: Vec<(Entity, Entity)> = links.iter().map(|l| (l.src, l.dst)).collect();
        pairs.sort();
        let mut want = vec![(src, a), (src, b)];
        want.sort();
        assert_eq!(pairs, want);
    }

    #[test]
    fn legality_predicates_answer_for_the_editor() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let producer = spawn_float(app.world_mut(), 1.0);
        let consumer = spawn_gain(app.world_mut(), 0.0);

        let entry = &app.world().resource::<WireRegistry>().entries[0];
        assert!((entry.has_source)(app.world(), producer));
        assert!(!(entry.has_source)(app.world(), consumer));
        assert!((entry.has_target)(app.world(), consumer));
        assert!(!(entry.has_target)(app.world(), producer));
    }

    #[test]
    fn collect_finds_every_entity_carrying_a_behaviour_component() {
        let mut app = App::new();
        register_behaviour::<Gain>(&mut app, noop_behaviour);
        let a = spawn_gain(app.world_mut(), 0.0);
        let b = spawn_gain(app.world_mut(), 0.0);
        spawn_float(app.world_mut(), 1.0);

        let collect = app.world().resource::<BehaviourRegistry>().entries[0].collect;
        let mut found = Vec::new();
        collect(app.world_mut(), &mut found);

        found.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn a_registered_wire_can_be_inserted_read_and_removed() {
        // The project format's whole write path for wires. Going through the
        // registry rather than the concrete type is what lets the applier be
        // generic over every wire type at once.
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let entry_index = 0;
        let (insert, read, remove) = {
            let entry = &app.world().resource::<WireRegistry>().entries[entry_index];
            (entry.insert, entry.read, entry.remove)
        };

        assert_eq!(read(app.world(), dst), None, "nothing wired yet");

        insert(app.world_mut(), dst, src);
        assert_eq!(read(app.world(), dst), Some(src));

        remove(app.world_mut(), dst);
        assert_eq!(read(app.world(), dst), None);
    }

    #[test]
    fn inserting_over_an_existing_wire_replaces_its_source() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let first = spawn_float(app.world_mut(), 1.0);
        let second = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let (insert, read) = {
            let entry = &app.world().resource::<WireRegistry>().entries[0];
            (entry.insert, entry.read)
        };

        insert(app.world_mut(), dst, first);
        insert(app.world_mut(), dst, second);

        assert_eq!(read(app.world(), dst), Some(second));
    }

    #[test]
    fn removing_a_wire_that_is_not_there_is_a_no_op() {
        let mut app = App::new();
        register_wire::<GainFrom>(&mut app);
        let dst = spawn_gain(app.world_mut(), 0.0);

        let remove = app.world().resource::<WireRegistry>().entries[0].remove;
        remove(app.world_mut(), dst);

        assert!(app.world().get::<GainFrom>(dst).is_none());
    }
}
