//! Shared test harness for exercising new-model node kinds through the real
//! graph tick, mirroring `sway-graph`'s own `graph::tick` test harness
//! (`crates/sway-graph/src/graph/tick.rs`): a bare `World` — no `Graph`,
//! which is exactly what a node's `evaluate` sees — with a fixed timestep
//! already advanced by one step, so `delta_secs()` reports a constant,
//! reproducible `dt` from the first tick onward.

use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;
use bevy_reflect::{PartialReflect, TypeRegistry, TypeRegistryArc};
use bevy_time::{Fixed, Time};
use sway_graph::graph::{Graph, NodeId, Part, path};

pub const TICK_HZ: f64 = 120.0;

/// A bare `World` with `registry` installed and a fixed timestep at
/// [`TICK_HZ`], advanced by one step so the first tick already reports the
/// real per-tick `dt`.
pub fn trace_world(registry: TypeRegistry) -> World {
    let mut world = World::new();
    world.insert_resource(AppTypeRegistry(TypeRegistryArc::default()));
    {
        let installed = world.resource::<AppTypeRegistry>().clone();
        *installed.write() = registry;
    }
    let mut time = Time::<Fixed>::from_hz(TICK_HZ);
    let step = time.timestep();
    time.advance_by(step);
    world.insert_resource(time);
    world
}

/// Runs one tick with the graph genuinely outside the world, as
/// `World::resource_scope` guarantees in the real tick.
pub fn tick(graph: &mut Graph, world: &World) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    graph.rebuild_order_if_dirty();
    sway_graph::graph::tick::run(graph, world, &registry.read());
}

pub fn read_f32(graph: &Graph, node: NodeId, part: Part, field: &str) -> f32 {
    path::resolve(graph.get(node).expect("node"), part, field)
        .and_then(|value| value.try_downcast_ref::<f32>().copied())
        .expect("an f32 field")
}

pub fn set_field(graph: &mut Graph, node: NodeId, field: &str, value: &dyn PartialReflect) {
    let target = graph.get_mut(node).expect("node");
    path::resolve_mut(target, Part::Inlets, field)
        .expect("field")
        .try_apply(value)
        .expect("apply");
}
