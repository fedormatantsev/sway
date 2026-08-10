//! What Bevy does to a relationship when the *producer* despawns.
//!
//! `relationship_semantics.rs` pins the consumer side. M6's `Delete` command
//! needs the other direction: if a wire component survives on a consumer whose
//! producer is gone, `Delete` must clear it by hand.

use bevy_ecs::world::World;
use sway_graph::test_wires::{GainFrom, spawn_float, spawn_gain};

#[test]
fn despawning_a_producer_removes_the_wire_from_its_consumers() {
    let mut world = World::new();
    let src = spawn_float(&mut world, 1.0);
    let dst = spawn_gain(&mut world, 0.0);
    world.entity_mut(dst).insert(GainFrom(src));

    world.despawn(src);

    assert!(
        world.get::<GainFrom>(dst).is_none(),
        "if this fails, EditorCommand::Delete must walk the producer's \
         RelationshipTarget and remove each consumer's wire component itself"
    );
    assert!(world.get_entity(dst).is_ok(), "the consumer itself must survive");
}
