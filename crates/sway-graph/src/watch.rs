//! Topology watching. Spec §3.2.
//!
//! The graph's shape changes only while authoring. Component hooks on
//! reflected wire and behaviour types notice that it did; in a show they
//! see no `Authoring` resource and leave the baked order alone.

use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::SystemSet;

/// Present iff this build can author the graph. Insert it before adding
/// `WiresPlugin` in an editor build; omit it in a show.
#[derive(Resource)]
pub struct Authoring;

#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WatchSet;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{GraphOrder, Step, TopologyDirty};
    use crate::register::{register_behaviour_type, register_wire_type};
    use crate::run::WiresPlugin;
    use crate::test_wires::{Gain, GainFrom, spawn_float, spawn_gain};
    use bevy_app::App;
    use bevy_time::{Fixed, Time};

    fn watched_app(authoring: bool) -> App {
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1));
        if authoring {
            app.insert_resource(Authoring);
        }
        app.add_plugins(WiresPlugin);
        app.register_type::<crate::test_wires::FloatOut>();
        app.register_type::<Gain>();
        register_wire_type::<GainFrom>(&mut app);
        // DEVIATION from the brief's literal test code: a single `update()`
        // does not actually run `FixedUpdate` here -- frame 0's fixed-time
        // accumulator starts empty, so the first call only primes it (the
        // same gotcha `run.rs::engine_app` and `test_nodes.rs` document and
        // work around). Settling the initial dirty flag needs two calls.
        app.update();
        app.update();
        app
    }

    #[test]
    fn inserting_a_wire_marks_the_topology_dirty() {
        let mut app = watched_app(true);
        assert!(!app.world().resource::<TopologyDirty>().0, "settled");

        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        // Hooks mark dirty on insert; FixedUpdate then rebuilds and clears it,
        // so observe the effect rather than the flag: the order gains steps.
        app.update();

        assert_eq!(app.world().resource::<GraphOrder>().steps.len(), 1);
    }

    #[test]
    fn removing_a_wire_marks_the_topology_dirty() {
        let mut app = watched_app(true);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));
        app.update();

        app.world_mut().entity_mut(dst).remove::<GainFrom>();
        app.update();

        assert!(app.world().resource::<GraphOrder>().steps.is_empty());
    }

    #[test]
    fn without_authoring_the_topology_is_never_re_scanned() {
        // A show build: the initial build happens, and nothing after it.
        let mut app = watched_app(false);
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        app.update();

        assert!(
            app.world().resource::<GraphOrder>().steps.is_empty(),
            "a show build does not notice authoring it cannot do"
        );
    }

    #[test]
    fn adding_a_behaviour_without_a_wire_still_rebuilds() {
        let mut app = watched_app(true);
        register_behaviour_type::<Gain>(&mut app);
        assert!(app.world().resource::<GraphOrder>().steps.is_empty());

        spawn_gain(app.world_mut(), 1.0);
        app.update();

        let steps = &app.world().resource::<GraphOrder>().steps;
        assert!(
            matches!(steps.as_slice(), [Step::Run { .. }]),
            "adding a behaviour carrier with no wire change must still rebuild"
        );
    }
}
