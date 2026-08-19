//! The tick: a flat walk of the rebuilt step list.
//!
//! Design D4 — the tick is an **exclusive system** driven through
//! `World::resource_scope`, so `&mut Graph` and `&World` are held at once.
//! That is not a workaround, it is the point: inside the scope the `World`
//! genuinely does not contain the `Graph`, so a node cannot reach the graph
//! through the `&World` it is handed, cannot re-enter the tick, and cannot
//! read another node's outlet behind the edge list's back.

use bevy_app::{App, FixedUpdate, Plugin, PreUpdate};
use bevy_ecs::change_detection::Mut;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_ecs::schedule::common_conditions::resource_exists;
use bevy_ecs::world::World;
use bevy_reflect::enums::{DynamicEnum, DynamicVariant};
use bevy_reflect::tuple::DynamicTuple;
use bevy_reflect::{GetPath, ParsedPath, PartialReflect, ReflectMut, TypeRegistry};

use crate::graph::command::{GraphRx, apply_graph_commands};
use crate::graph::id::NodeId;
use crate::graph::model::{Graph, reflect_equal};
use crate::graph::node::Part;
use crate::graph::order::{GraphStep, PropagateStep, Target};
use crate::graph::registry::ReflectNodeKind;

/// Runs the rebuilt plan once.
///
/// `world` must not contain the `Graph` — [`tick_graph`] guarantees that by
/// running inside `resource_scope`.
pub fn run(graph: &mut Graph, world: &World, registry: &TypeRegistry) {
    // Taken out so each step can borrow the graph mutably, put back after.
    let order = graph.take_order();
    for step in &order.steps {
        match step {
            GraphStep::TruncateList { node, path, len } => truncate_list(graph, *node, path, *len),
            GraphStep::Propagate(step) => propagate(graph, step),
            GraphStep::Evaluate { node } => evaluate(graph, *node, world, registry),
        }
    }
    graph.put_order(order);
}

/// Shrinks a list-shaped inlet to the number of value edges landing on it.
/// Growth is [`propagate`]'s job: it pushes the value it is about to write.
fn truncate_list(graph: &mut Graph, node: NodeId, path: &ParsedPath, len: usize) {
    let changed = {
        let Some(target) = graph.get_mut(node) else {
            return;
        };
        let Ok(field) = target.value_mut().reflect_path_mut(path) else {
            return;
        };
        let ReflectMut::List(list) = field.reflect_mut() else {
            return;
        };
        let mut changed = false;
        while list.len() > len {
            list.pop();
            changed = true;
        }
        changed
    };
    if changed {
        graph.dirty_insert(node);
    }
}

/// Copies one outlet field into one inlet field.
///
/// `slice::get_disjoint_mut` behind [`Graph::pair_mut`] gives `&outlets` of one
/// node and `&mut inlets` of another with no allocation. It errors on
/// duplicate indices, which is why self-edges are refused at connect time.
fn propagate(graph: &mut Graph, step: &PropagateStep) {
    let changed = {
        let Some((src, dst)) = graph.pair_mut(step.src, step.dst) else {
            return;
        };
        let Ok(value) = src.value().reflect_path(&step.src_path) else {
            return;
        };
        let Ok(field) = dst.value_mut().reflect_path_mut(&step.dst_path) else {
            return;
        };
        write(field, value, step.target)
    };
    if changed {
        graph.dirty_insert(step.dst);
    }
}

/// Writes `value` into `field` per the destination's shape, guarded by a
/// reflect-equality check so an equal value does not dirty the node.
fn write(field: &mut dyn PartialReflect, value: &dyn PartialReflect, target: Target) -> bool {
    match target {
        Target::Direct => {
            if reflect_equal(field, value) {
                return false;
            }
            field.try_apply(value).is_ok()
        }
        Target::Optional => {
            let mut some = DynamicTuple::default();
            some.insert_boxed(value.to_dynamic());
            let some = DynamicEnum::new("Some", DynamicVariant::Tuple(some));
            if reflect_equal(field, &some) {
                return false;
            }
            field.try_apply(&some).is_ok()
        }
        Target::Index(index) => {
            let ReflectMut::List(list) = field.reflect_mut() else {
                return false;
            };
            match index.cmp(&list.len()) {
                core::cmp::Ordering::Less => {
                    let Some(element) = list.get_mut(index) else {
                        return false;
                    };
                    if reflect_equal(element, value) {
                        return false;
                    }
                    element.try_apply(value).is_ok()
                }
                // Rebuild emits the indices of one variadic inlet in ascending
                // order, so the list is always exactly one short here.
                core::cmp::Ordering::Equal => {
                    list.push(value.to_dynamic());
                    true
                }
                core::cmp::Ordering::Greater => false,
            }
        }
    }
}

/// Runs one node.
///
/// The dirty rule needs to know whether `evaluate` actually changed anything,
/// and `&mut self` cannot report that — so state and outlets are snapshotted
/// and compared. The snapshot is skipped when the node is already dirty, which
/// is the common case on the tick after an edit.
fn evaluate(graph: &mut Graph, node: NodeId, world: &World, registry: &TypeRegistry) {
    let already_dirty = graph.is_dirty(node);
    let Some(target) = graph.get(node) else {
        return;
    };
    let Some(reflect_kind) = registry.get_type_data::<ReflectNodeKind>(target.value().type_id())
    else {
        // Not a registered node kind: nothing to run. A graph loaded against a
        // build that does not know this kind still ticks everything else.
        return;
    };
    let before = (!already_dirty)
        .then(|| {
            Some((
                target.part(Part::State)?.to_dynamic(),
                target.part(Part::Outlets)?.to_dynamic(),
            ))
        })
        .flatten();

    {
        let Some(target) = graph.get_mut(node) else {
            return;
        };
        let Some(kind) = reflect_kind.get_mut(target.value_mut()) else {
            return;
        };
        kind.evaluate(world);
    }

    let Some((state, outlets)) = before else {
        return;
    };
    let Some(target) = graph.get(node) else {
        return;
    };
    let unchanged = target
        .part(Part::State)
        .is_some_and(|part| reflect_equal(part, state.as_ref()))
        && target
            .part(Part::Outlets)
            .is_some_and(|part| reflect_equal(part, outlets.as_ref()));
    if !unchanged {
        graph.dirty_insert(node);
    }
}

/// Rebuilds the order if the shape changed, then runs one tick.
///
/// Exclusive by design (D4). `resource_scope` is what makes the graph absent
/// from the `&World` a node sees.
pub fn tick_graph(world: &mut World) {
    let Some(type_registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        return;
    };
    let _ = world.try_resource_scope(|world: &mut World, mut graph: Mut<Graph>| {
        let registry = type_registry.read();
        graph.rebuild_order_if_dirty();
        run(&mut graph, world, &registry);
    });
}

/// Inserts the graph resource and schedules the command drain and the tick.
///
/// Deliberately does **not** register any node kinds: `sway-nodes` and
/// `sway-runtime` own that, which is what keeps `sway-graph` free of
/// `bevy_render`, MIDI types and the document format.
pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Graph>()
            .register_type::<NodeId>()
            .register_type::<crate::graph::id::EdgeId>()
            .add_systems(FixedUpdate, tick_graph)
            .add_systems(
                PreUpdate,
                apply_graph_commands.run_if(resource_exists::<GraphRx>),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::edge::Port;
    use crate::graph::node::Node;
    use crate::graph::path;
    use crate::graph::testing::{Clock, Counter, Fan, Nested, Sink, Source, test_registry};
    use bevy_math::Vec2;
    use bevy_time::{Fixed, Time};

    const TICK_HZ: f64 = 120.0;

    /// A `World` with the fixture kinds registered and a fixed timestep, but
    /// without the `Graph` resource — which is exactly the shape a node sees.
    fn trace_world() -> World {
        let mut world = World::new();
        world.insert_resource(AppTypeRegistry(bevy_reflect::TypeRegistryArc::default()));
        {
            let registry = world.resource::<AppTypeRegistry>().clone();
            *registry.write() = test_registry();
        }
        // A bare `Time<Fixed>` reports a zero delta until the fixed-update
        // loop advances it; advance it once by hand so the trace runs at the
        // fixed delta the golden values assume.
        let mut time = Time::<Fixed>::from_hz(TICK_HZ);
        let step = time.timestep();
        time.advance_by(step);
        world.insert_resource(time);
        world
    }

    /// One tick at a fixed delta, with the graph genuinely outside the world.
    fn tick(graph: &mut Graph, world: &World) {
        let registry = world.resource::<AppTypeRegistry>().clone();
        graph.rebuild_order_if_dirty();
        run(graph, world, &registry.read());
    }

    fn read(graph: &Graph, node: NodeId, part: Part, field: &str) -> f32 {
        path::resolve(graph.get(node).expect("node"), part, field)
            .and_then(|value| value.try_downcast_ref::<f32>().copied())
            .expect("an f32 field")
    }

    fn set(graph: &mut Graph, node: NodeId, field: &str, value: f32) {
        let target = graph.get_mut(node).expect("node");
        path::resolve_mut(target, Part::Inlets, field)
            .expect("field")
            .try_apply(&value)
            .expect("apply");
    }

    // --- 3.5 / 3.6 ------------------------------------------------------

    #[test]
    fn a_two_hop_chain_resolves_in_a_single_tick() {
        // THE claim: A -> B -> C lands end to end in one tick, which is only
        // true if the order puts every producer before its consumer.
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        let c = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        set(&mut graph, a, "base", 3.0);
        graph
            .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
            .expect("legal");
        graph
            .connect(Port::new(b, "total"), Port::new(c, "value"), 0)
            .expect("legal");

        tick(&mut graph, &world);

        assert_eq!(read(&graph, a, Part::Outlets, "out"), 3.0);
        assert_eq!(read(&graph, b, Part::Inlets, "step"), 3.0);
        assert_eq!(
            read(&graph, c, Part::Outlets, "seen"),
            3.0,
            "the second hop must land in the SAME tick"
        );
    }

    #[test]
    fn a_golden_trace_is_reproducible_at_a_fixed_delta() {
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        set(&mut graph, a, "base", 0.5);
        graph
            .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
            .expect("legal");

        let mut trace = Vec::new();
        for _ in 0..5 {
            tick(&mut graph, &world);
            trace.push(read(&graph, b, Part::Outlets, "total"));
        }

        assert_eq!(trace, vec![0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    #[test]
    fn a_cycle_is_reported_and_still_ticks() {
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
        graph
            .connect(Port::new(a, "total"), Port::new(b, "step"), 0)
            .expect("legal");
        graph
            .connect(Port::new(b, "total"), Port::new(a, "step"), 0)
            .expect("legal");
        // Both inlets are driven by the cycle, so seed `a`'s state instead:
        // a cycle member reads the previous tick's values, and there is no
        // previous tick.
        {
            let target = graph.get_mut(a).expect("node");
            path::resolve_mut(target, Part::State, "total")
                .expect("field")
                .try_apply(&1.0f32)
                .expect("apply");
        }

        tick(&mut graph, &world);

        let mut cycles = graph.order().cycles.clone();
        cycles.sort();
        assert_eq!(cycles, vec![a, b], "diagnostics name both nodes");
        assert_eq!(
            read(&graph, a, Part::Outlets, "total"),
            1.0,
            "the tick still evaluates the cycle's members"
        );
        assert_eq!(
            read(&graph, b, Part::Outlets, "total"),
            1.0,
            "and the acyclic part of the cycle still carries a value through"
        );
    }

    #[test]
    fn an_external_time_source_is_an_ordinary_node() {
        let world = trace_world();
        let mut graph = Graph::default();
        let clock = graph.insert(Node::of(Vec2::ZERO, Clock::default()));

        tick(&mut graph, &world);
        tick(&mut graph, &world);

        let dt = (1.0 / TICK_HZ) as f32;
        assert!((read(&graph, clock, Part::Outlets, "elapsed") - dt * 2.0).abs() < 1e-6);
    }

    // --- 1.5 / dirty ----------------------------------------------------

    #[test]
    fn a_tick_that_writes_only_equal_values_reports_nothing() {
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(b, "value"), 0)
            .expect("legal");

        tick(&mut graph, &world);
        graph.drain_dirty();
        tick(&mut graph, &world);

        assert!(
            graph.drain_dirty().is_empty(),
            "a second tick carrying the same values must not dirty anything"
        );
    }

    #[test]
    fn a_glam_typed_outlet_does_not_dirty_on_an_equal_write() {
        // Regression. glam's `reflect_partial_eq` answers by downcasting the
        // operand to its own concrete type, so a concrete `Vec3` compared
        // against the `to_dynamic()` snapshot `evaluate` takes answers
        // `Some(false)` even when the two are equal -- the reverse comparison
        // answers `Some(true)`. Asking one way only reported every node
        // carrying a `Vec3`/`Quat` in state or outlets as changed on every
        // tick, which would hand the projectors a dirty set containing most of
        // the scene every frame and make per-node change tracking pointless.
        // A scalar fixture cannot catch this; `Placer` exists to.
        let world = trace_world();
        let mut graph = Graph::default();
        let placer = graph.insert(Node::of(
            Vec2::ZERO,
            crate::graph::testing::Placer::default(),
        ));

        tick(&mut graph, &world);
        graph.drain_dirty();
        tick(&mut graph, &world);

        assert!(
            graph.drain_dirty().is_empty(),
            "a second tick writing an equal `Vec3` must not dirty {placer}"
        );
    }

    #[test]
    fn a_changed_value_dirties_only_the_nodes_it_reaches() {
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let b = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        let idle = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        graph
            .connect(Port::new(a, "out"), Port::new(b, "value"), 0)
            .expect("legal");

        tick(&mut graph, &world);
        graph.drain_dirty();
        set(&mut graph, a, "base", 9.0);
        graph.mark_dirty(a);
        graph.drain_dirty();

        tick(&mut graph, &world);

        assert_eq!(graph.drain_dirty(), vec![a, b], "{idle} is untouched");
    }

    // --- 3.2 / 3.3 ------------------------------------------------------

    #[test]
    fn many_connections_arrive_in_key_order() {
        let world = trace_world();
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        let mut sources = Vec::new();
        for value in [1.0, 2.0, 3.0] {
            let id = graph.insert(Node::of(Vec2::ZERO, Source::default()));
            set(&mut graph, id, "base", value);
            sources.push(id);
        }
        // Keys 30, 10, 20 -> values must arrive as 2.0, 3.0, 1.0.
        for (id, slot) in sources.iter().zip([30, 10, 20]) {
            graph
                .connect(Port::new(*id, "out"), Port::new(fan, "values"), slot)
                .expect("legal");
        }

        tick(&mut graph, &world);

        let seen = path::resolve(graph.get(fan).unwrap(), Part::Outlets, "seen")
            .and_then(|value| value.try_downcast_ref::<Vec<f32>>().cloned())
            .expect("a Vec<f32>");
        assert_eq!(seen, vec![2.0, 3.0, 1.0]);
        assert_eq!(read(&graph, fan, Part::Outlets, "sum"), 6.0);
    }

    #[test]
    fn disconnecting_a_variadic_edge_shrinks_the_list() {
        let world = trace_world();
        let mut graph = Graph::default();
        let fan = graph.insert(Node::of(Vec2::ZERO, Fan::default()));
        let mut edges = Vec::new();
        for value in [1.0, 2.0] {
            let id = graph.insert(Node::of(Vec2::ZERO, Source::default()));
            set(&mut graph, id, "base", value);
            edges.push(
                graph
                    .connect(Port::new(id, "out"), Port::new(fan, "values"), 0)
                    .expect("legal"),
            );
        }
        tick(&mut graph, &world);
        assert_eq!(read(&graph, fan, Part::Outlets, "sum"), 3.0);

        graph.disconnect(edges[0]);
        tick(&mut graph, &world);

        assert_eq!(
            read(&graph, fan, Part::Outlets, "sum"),
            2.0,
            "the Vec is derived from the edge set, so the stale element is gone"
        );
    }

    #[test]
    fn an_unconnected_optional_inlet_is_absent_not_defaulted() {
        let world = trace_world();
        let mut graph = Graph::default();
        let sink = graph.insert(Node::of(Vec2::ZERO, Sink::default()));

        tick(&mut graph, &world);

        let had = path::resolve(graph.get(sink).unwrap(), Part::Outlets, "had_maybe")
            .and_then(|value| value.try_downcast_ref::<bool>().copied())
            .expect("a bool");
        assert!(!had, "no substitute value was supplied on its behalf");
    }

    #[test]
    fn a_connected_optional_inlet_arrives_wrapped() {
        let world = trace_world();
        let mut graph = Graph::default();
        let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let sink = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        set(&mut graph, a, "base", 4.0);
        graph
            .connect(Port::new(a, "out"), Port::new(sink, "maybe"), 0)
            .expect("legal");

        tick(&mut graph, &world);

        let maybe = path::resolve(graph.get(sink).unwrap(), Part::Inlets, "maybe")
            .and_then(|value| value.try_downcast_ref::<Option<f32>>().copied())
            .expect("an Option<f32>");
        assert_eq!(maybe, Some(4.0));
    }

    #[test]
    fn a_valueless_edge_writes_nothing_but_still_orders() {
        let world = trace_world();
        let mut graph = Graph::default();
        // `Sink` is ordered after `Source` purely by the marker edge.
        let sink = graph.insert(Node::of(Vec2::ZERO, Sink::default()));
        let source = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        graph
            .connect(Port::new(source, "marker"), Port::new(sink, "marker"), 0)
            .expect("legal");

        tick(&mut graph, &world);

        assert_eq!(
            graph.order().order,
            vec![source, sink],
            "the marker edge is a sort constraint"
        );
        assert!(
            graph
                .order()
                .steps
                .iter()
                .all(|step| !matches!(step, GraphStep::Propagate(_))),
            "and produced no propagate step"
        );
    }

    #[test]
    fn a_nested_destination_writes_only_that_component() {
        let world = trace_world();
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(Vec2::ZERO, Source::default()));
        let nested = graph.insert(Node::of(Vec2::ZERO, Nested::default()));
        set(&mut graph, source, "base", 7.0);
        {
            let target = graph.get_mut(nested).expect("node");
            path::resolve_mut(target, Part::Inlets, "point")
                .expect("field")
                .try_apply(&Vec2::new(1.0, 2.0))
                .expect("apply");
        }
        graph
            .connect(Port::new(source, "out"), Port::new(nested, "point.y"), 0)
            .expect("legal");

        tick(&mut graph, &world);

        let point = path::resolve(graph.get(nested).unwrap(), Part::Inlets, "point")
            .and_then(|value| value.try_downcast_ref::<Vec2>().copied())
            .expect("a Vec2");
        assert_eq!(point, Vec2::new(1.0, 7.0), "x is unchanged");
    }

    // --- plugin ---------------------------------------------------------

    #[test]
    fn the_plugin_ticks_the_graph_through_an_app() {
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(GraphPlugin);
        {
            let registry = app
                .world_mut()
                .get_resource_or_init::<AppTypeRegistry>()
                .clone();
            let mut write = registry.write();
            crate::graph::registry::register_node_kind::<Counter>(&mut write);
            crate::graph::registry::register_node_kind::<Source>(&mut write);
        }
        // Burn frame 0's empty fixed-time accumulator.
        app.update();

        let (a, b) = {
            let mut graph = app.world_mut().resource_mut::<Graph>();
            let a = graph.insert(Node::of(Vec2::ZERO, Source::default()));
            let b = graph.insert(Node::of(Vec2::ZERO, Counter::default()));
            graph
                .connect(Port::new(a, "out"), Port::new(b, "step"), 0)
                .expect("legal");
            (a, b)
        };
        set(&mut app.world_mut().resource_mut::<Graph>(), a, "base", 2.0);

        app.update();

        let graph = app.world().resource::<Graph>();
        assert_eq!(read(graph, b, Part::Outlets, "total"), 2.0);
    }

    #[test]
    fn a_node_cannot_reach_the_graph_from_its_world() {
        // `Clock::evaluate` debug-asserts this; running it through the real
        // `tick_graph` is what proves `resource_scope` is doing the work.
        let mut app = App::new();
        app.add_plugins(bevy_time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(bevy_time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(GraphPlugin);
        {
            let registry = app
                .world_mut()
                .get_resource_or_init::<AppTypeRegistry>()
                .clone();
            crate::graph::registry::register_node_kind::<Clock>(&mut registry.write());
        }
        app.update();
        app.world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(Vec2::ZERO, Clock::default()));

        app.update();
    }
}
