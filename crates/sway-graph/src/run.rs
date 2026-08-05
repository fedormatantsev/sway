//! The tick: a flat walk of the step list. Spec §3.1.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::world::World;
use bevy_time::{Fixed, Time};

use crate::ctx::TickCtx;
use crate::order::{rebuild_order, GraphOrder, Step, TopologyDirty};
use crate::registry_wires::{BehaviourRegistry, WireRegistry};

/// Ticks since the graph started running. Exposed as `TickCtx::tick_index`.
#[derive(Resource, Default)]
pub struct WireTickCount(pub u64);

pub fn graph_tick(world: &mut World) {
    let (dt, tick_start) = {
        let time = world.resource::<Time<Fixed>>();
        let dt = time.delta_secs();
        (dt, time.elapsed_secs_f64() - dt as f64)
    };
    let tick_index = {
        let mut count = world.resource_mut::<WireTickCount>();
        let index = count.0;
        count.0 += 1;
        index
    };
    let ctx = TickCtx { dt, tick_start, tick_index };

    // Taken out so the steps can borrow `world` mutably, put back after.
    let order = world.remove_resource::<GraphOrder>().unwrap_or_default();
    for step in &order.steps {
        match *step {
            Step::Propagate { run, src, dst } => run(world, src, dst),
            Step::Run { run, entity } => run(world, entity, &ctx),
        }
    }
    world.insert_resource(order);
}

/// Inserts the wire engine's resources and schedules the rebuild and the tick.
pub struct WiresPlugin;

impl Plugin for WiresPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WireRegistry>()
            .init_resource::<BehaviourRegistry>()
            .init_resource::<GraphOrder>()
            .init_resource::<TopologyDirty>()
            .init_resource::<WireTickCount>()
            .init_resource::<Time<crate::transport::Transport>>()
            .register_type::<crate::transport::Transport>()
            .register_type::<crate::transport::TransportState>()
            .add_systems(FixedUpdate, (rebuild_order, graph_tick).chain());

        app.configure_sets(
            bevy_app::PreUpdate,
            crate::watch::WatchSet.run_if(bevy_ecs::schedule::common_conditions::resource_exists::<
                crate::watch::Authoring,
            >),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_wires::{register_behaviour, register_wire};
    use crate::test_wires::{spawn_float, spawn_gain, FloatOut, Gain, GainFrom};
    use bevy_ecs::entity::Entity;

    const TICK_HZ: f64 = 120.0;

    /// A `Gain` behaviour: value = factor * 2, published on `FloatOut` so a
    /// second hop can consume it.
    fn double(world: &mut World, entity: Entity, _ctx: &TickCtx) {
        let Some(gain) = world.get::<Gain>(entity) else { return };
        let doubled = gain.factor * 2.0;
        if let Some(mut gain) = world.get_mut::<Gain>(entity)
            && gain.value != doubled
        {
            gain.value = doubled;
        }
        if let Some(mut out) = world.get_mut::<FloatOut>(entity)
            && out.0 != doubled
        {
            out.0 = doubled;
        }
    }

    fn engine_app() -> App {
        let mut app = App::new();
        // `bevy_time::TimePlugin` alone leaves `FixedUpdate` driven by
        // wall-clock time, so a fast test's `app.update()` may accumulate less
        // than one timestep and never run `graph_tick`. Pinning the timestep
        // and stepping it manually makes each `update()` run it exactly once.
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(WiresPlugin);
        register_wire::<GainFrom>(&mut app);
        register_behaviour::<Gain>(&mut app, double);
        app.update(); // burn frame 0's empty fixed-time accumulator -- mirrors
                      // test_nodes.rs::engine_app's identical gotcha
        app
    }

    #[test]
    fn a_two_hop_chain_resolves_in_a_single_tick() {
        // THE claim. `a` -> `b`.factor -> b doubles -> `c`.factor.
        // If the order were wrong, `c` would lag a tick behind.
        let mut app = engine_app();
        let a = spawn_float(app.world_mut(), 3.0);
        let b = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(b).insert(FloatOut(0.0));
        let c = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(b).insert(GainFrom(a));
        app.world_mut().entity_mut(c).insert(GainFrom(b));

        app.update();

        assert_eq!(app.world().get::<Gain>(b).map(|g| g.factor), Some(3.0));
        assert_eq!(app.world().get::<Gain>(b).map(|g| g.value), Some(6.0));
        assert_eq!(
            app.world().get::<Gain>(c).map(|g| g.factor),
            Some(6.0),
            "the second hop must land in the SAME tick"
        );
    }

    #[test]
    fn an_unwired_consumer_keeps_its_authored_value() {
        // Spec §3.5: no prefill, no shadow copy — the field is simply not
        // written.
        let mut app = engine_app();
        let solo = spawn_gain(app.world_mut(), 7.5);

        app.update();

        assert_eq!(app.world().get::<Gain>(solo).map(|g| g.factor), Some(7.5));
    }

    use bevy_ecs::query::Changed;
    use bevy_ecs::system::{Query, ResMut};

    #[derive(Resource, Default)]
    struct ChangedCount(usize);

    fn count_changed(query: Query<(), Changed<Gain>>, mut count: ResMut<ChangedCount>) {
        count.0 = query.iter().count();
    }

    #[test]
    fn a_wire_carrying_an_unchanged_value_leaves_the_target_unchanged() {
        // Spec §3.4. `get_mut` marks Changed unconditionally, so a wire that
        // writes every tick destroys change detection for everything
        // downstream -- which is the whole dirty story now that the cook gate
        // is gone.
        let mut app = engine_app();
        app.init_resource::<ChangedCount>();
        app.add_systems(bevy_app::Last, count_changed);

        let src = spawn_float(app.world_mut(), 3.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        app.update();
        assert_eq!(
            app.world().resource::<ChangedCount>().0,
            1,
            "the first tick really does change it"
        );

        app.update();
        assert_eq!(
            app.world().resource::<ChangedCount>().0,
            0,
            "a second tick carrying the SAME value must not re-mark it"
        );
    }

    #[test]
    fn a_wire_carrying_a_new_value_does_mark_the_target_changed() {
        let mut app = engine_app();
        app.init_resource::<ChangedCount>();
        app.add_systems(bevy_app::Last, count_changed);

        let src = spawn_float(app.world_mut(), 3.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));
        app.update();
        app.update();

        app.world_mut().entity_mut(src).insert(FloatOut(4.0));
        app.update();

        assert_eq!(app.world().resource::<ChangedCount>().0, 1);
    }
}
