//! What Bevy 0.19's relationships actually do, pinned. The wire model in
//! `docs/superpowers/specs/2026-08-05-wires-design.md` §2.2 claims the ECS
//! enforces one-source-per-inlet, rewire eviction, and non-cascading
//! despawn. These tests are that claim, checked against the real engine.

use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::relationship::RelationshipTarget;
use bevy_ecs::world::World;

#[derive(Component)]
#[relationship(relationship_target = Consumers)]
struct DrivenBy(#[entities] Entity);

#[derive(Component)]
#[relationship_target(relationship = DrivenBy)]
struct Consumers(Vec<Entity>);

/// THE hazard. A wire's target collection must not behave like `Children`:
/// despawning a producer must leave its consumers alive.
#[test]
fn despawning_a_producer_does_not_despawn_its_consumers() {
    let mut world = World::new();
    let producer = world.spawn_empty().id();
    let consumer = world.spawn(DrivenBy(producer)).id();

    world.despawn(producer);

    assert!(
        world.get_entity(consumer).is_ok(),
        "a consumer must survive its producer -- if this fails, LINKED_SPAWN \
         is not gated as assumed and the design needs a hand-written \
         RelationshipTarget impl"
    );
}

/// One component per type per entity is what makes "an inlet has at most one
/// source" true by construction, with no validation pass.
#[test]
fn rewiring_evicts_the_previous_source() {
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();
    let consumer = world.spawn(DrivenBy(first)).id();

    world.entity_mut(consumer).insert(DrivenBy(second));

    let first_len = world.get::<Consumers>(first).map_or(0, |c| c.len());
    let second_len = world.get::<Consumers>(second).map_or(0, |c| c.len());
    assert_eq!(first_len, 0, "the old producer must lose its consumer");
    assert_eq!(second_len, 1, "the new producer must gain it");
    assert_eq!(world.get::<DrivenBy>(consumer).map(|d| d.0), Some(second));
}

/// Fan-out is the target collection, not a rule the engine enforces.
#[test]
fn one_producer_drives_many_consumers() {
    let mut world = World::new();
    let producer = world.spawn_empty().id();
    let a = world.spawn(DrivenBy(producer)).id();
    let b = world.spawn(DrivenBy(producer)).id();

    let consumers = world.get::<Consumers>(producer).expect("target collection");
    let mut seen: Vec<Entity> = consumers.iter().collect();
    seen.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(seen, want);
}

/// A self-wire is a cycle. Bevy rejects it for us, so the sort never sees one
/// and `propagate_of`'s two-entity fetch never aliases.
#[test]
fn a_self_referential_wire_is_removed() {
    let mut world = World::new();
    let entity = world.spawn_empty().id();
    world.entity_mut(entity).insert(DrivenBy(entity));
    world.flush();

    assert!(
        world.get::<DrivenBy>(entity).is_none(),
        "Bevy warns and removes a self-referential relationship"
    );
}

/// A despawned producer leaves the consumer's wire component in place, naming
/// a dead entity. Spec §2.2: this is the one case the ECS does not clean up,
/// and `propagate_of` skips it.
#[test]
fn a_despawned_producer_leaves_a_dangling_wire() {
    let mut world = World::new();
    let producer = world.spawn_empty().id();
    let consumer = world.spawn(DrivenBy(producer)).id();

    world.despawn(producer);

    let dangling = world.get::<DrivenBy>(consumer).map(|d| d.0);
    assert_eq!(dangling, Some(producer), "the wire still names the dead entity");
    assert!(world.get_entity(producer).is_err(), "which is in fact dead");
}
