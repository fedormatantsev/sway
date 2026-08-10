//! The per-wire change-detection check architecture §9 requires.
//!
//! Both halves matter. The first proves the wire does not dirty its target when
//! nothing changed. The second proves the harness can see a write at all —
//! without it, a wire that never writes anything would pass the first half.

#![cfg(test)]

use bevy::prelude::*;
use sway_graph::{Wire, propagate_of};

fn changed_count<T: Component>(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), Changed<T>>();
    query.iter(world).count()
}

/// Propagates `source` twice and asserts the second write left `Changed` clear,
/// then propagates `different` and asserts that one did not.
pub(crate) fn assert_writes_only_on_change<W: Wire>(
    source: W::Source,
    different: W::Source,
    target: W::Target,
) {
    let mut world = World::new();
    let src = world.spawn(source).id();
    let dst = world.spawn(target).id();

    propagate_of::<W>(&mut world, src, dst);
    world.clear_trackers();
    propagate_of::<W>(&mut world, src, dst);
    assert_eq!(
        changed_count::<W::Target>(&mut world),
        0,
        "wire \"{}\" wrote an equal value; use map_unchanged(..).set_if_neq(..)",
        W::NAME
    );

    let other = world.spawn(different).id();
    world.clear_trackers();
    propagate_of::<W>(&mut world, other, dst);
    assert_eq!(
        changed_count::<W::Target>(&mut world),
        1,
        "wire \"{}\" did not write a genuinely different value — the check above \
         proves nothing",
        W::NAME
    );
}
