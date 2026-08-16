//! The per-wire change-detection check architecture §9 requires.
//!
//! Both halves matter. The first proves the wire does not dirty its target when
//! nothing changed. The second proves the harness can see a write at all —
//! without it, a wire that never writes anything would pass the first half.

#![cfg(test)]

use std::any::TypeId;

use bevy::prelude::*;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_reflect::{GetTypeRegistration, TypePath};
use sway_graph::{Wire, propagate_reflected, register_wire_type};

fn changed_count<T: Component>(world: &mut World) -> usize {
    let mut query = world.query_filtered::<(), Changed<T>>();
    query.iter(world).count()
}

/// Inserts `W` on `dst` naming `src` and runs the reflected propagate helper.
pub(crate) fn propagate_wire<W>(world: &mut World, src: Entity, dst: Entity)
where
    W: Wire + Component + From<Entity>,
{
    world.entity_mut(dst).insert(W::from(src));
    propagate_reflected(world, src, dst, TypeId::of::<W>()).expect("propagate");
}

/// Propagates `source` twice and asserts the second write left `Changed` clear,
/// then propagates `different` and asserts that one did not.
pub(crate) fn assert_writes_only_on_change<W, S, T>(source: S, different: S, target: T)
where
    W: Wire + Component + Reflect + GetTypeRegistration + TypePath + From<Entity>,
    S: Component + Reflect + GetTypeRegistration,
    T: Component + Reflect + GetTypeRegistration,
{
    let mut app = App::new();
    app.init_resource::<AppTypeRegistry>();
    app.register_type::<S>();
    app.register_type::<T>();
    register_wire_type::<W>(&mut app);

    let src = app.world_mut().spawn(source).id();
    let dst = app.world_mut().spawn(target).id();
    app.world_mut().entity_mut(dst).insert(W::from(src));

    propagate_reflected(app.world_mut(), src, dst, TypeId::of::<W>()).expect("propagate");
    app.world_mut().clear_trackers();
    propagate_reflected(app.world_mut(), src, dst, TypeId::of::<W>()).expect("propagate");
    assert_eq!(
        changed_count::<T>(app.world_mut()),
        0,
        "wire {} wrote an equal value; use map_unchanged + reflect_partial_eq",
        W::type_path()
    );

    let other = app.world_mut().spawn(different).id();
    app.world_mut().entity_mut(dst).insert(W::from(other));
    app.world_mut().clear_trackers();
    propagate_reflected(app.world_mut(), other, dst, TypeId::of::<W>()).expect("propagate");
    assert_eq!(
        changed_count::<T>(app.world_mut()),
        1,
        "wire {} did not write a genuinely different value — the check above \
         proves nothing",
        W::type_path()
    );
}
