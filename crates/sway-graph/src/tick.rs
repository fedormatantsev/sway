//! The tick runner: one exclusive system in `FixedUpdate` that walks the
//! compiled plan.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::change_detection::{Mut, Tick};
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use bevy_time::{Fixed, Time};

use crate::compile::CompiledGraph;
use crate::edges::NodeRuntime;
use crate::ports::PortArena;
use crate::registry::{
    CookFn, NodeTypeRegistry, PrefillFn, ProducedTickFn, SeedOutletsFn, TickFn, TickOfFn,
};
use crate::view::{PortView, TickCtx};

/// Ticks since the graph started running. Exposed as `TickCtx::tick_index`.
#[derive(Resource, Default)]
pub struct GraphTickCount(pub u64);

/// Clones a slot's value while preserving its concrete type.
///
/// `to_dynamic` is the wrong tool: for a struct it returns a `Dynamic*`
/// proxy that can no longer downcast to the concrete type. `reflect_clone`
/// produces a real `T`.
fn clone_slot(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|e| {
            panic!(
                "graph_tick: could not clone a `{}` slot value while gathering an edge ({e:?})",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// The graph tick: one exclusive system in `FixedUpdate`.
///
/// One order, one pass: each node gathers, ticks, and cooks if dirty when its
/// turn comes. A product edge is an ordinary value edge, so the order that
/// puts a producer before its consumer is the same order that puts an LFO
/// before the node it drives.
pub fn graph_tick(world: &mut World) {
    let Some(mut compiled) = world.remove_resource::<CompiledGraph>() else {
        return;
    };

    let (dt, tick_start) = {
        let time = world.resource::<Time<Fixed>>();
        let dt = time.delta_secs();
        (dt, time.elapsed_secs_f64() - dt as f64)
    };
    let tick_index = {
        let mut count = world.resource_mut::<GraphTickCount>();
        let idx = count.0;
        count.0 += 1;
        idx
    };
    let ctx = TickCtx { dt, tick_start, tick_index };

    // Fn pointers copied out before the loop: `world` is borrowed mutably
    // inside it, so a `&NodeTypeEntry` cannot be held across it.
    struct Dispatch {
        tick: TickFn,
        prefill: PrefillFn,
        seed_outlets: SeedOutletsFn,
        inlets_changed_tick: TickOfFn,
        cook: Option<CookFn>,
        produced_change_tick: ProducedTickFn,
    }

    let dispatch: Vec<Dispatch> = {
        let registry = world.resource::<NodeTypeRegistry>();
        compiled
            .plans
            .iter()
            .map(|plan| {
                let entry = registry.get(plan.node_type).unwrap_or_else(|| {
                    panic!(
                        "graph_tick: node {:?}'s node type {:?} is not in the registry",
                        plan.entity, plan.node_type
                    )
                });
                Dispatch {
                    tick: entry.tick,
                    prefill: entry.prefill,
                    seed_outlets: entry.seed_outlets,
                    inlets_changed_tick: entry.inlets_changed_tick,
                    cook: entry.cook,
                    produced_change_tick: entry.produced_change_tick,
                }
            })
            .collect()
    };

    world.resource_scope(|world: &mut World, mut arena: Mut<PortArena>| {
        if !compiled.outlets_seeded {
            for (plan, d) in compiled.plans.iter().zip(&dispatch) {
                (d.seed_outlets)(&mut arena, plan);
            }
            compiled.outlets_seeded = true;
        }

        // Empty every event list in place, keeping its allocation. This is
        // what stops a node that stopped writing its event outlet from
        // firing the same occurrence forever.
        for &(slot, clear) in &compiled.clears {
            clear(&mut *arena.values[slot]);
        }

        for (plan, d) in compiled.plans.iter().zip(&dispatch) {
            // `dirty` accumulates this tick's reasons to cook; it is OR-ed
            // into the sticky flag rather than assigned, so a reason raised
            // on an earlier tick is not lost.
            let mut dirty = false;

            for &(src, dst) in &plan.copies {
                let incoming = clone_slot(&*arena.values[src]);
                // `reflect_partial_eq` returns None for values that cannot be
                // compared — including the `()` a freshly-resized slot holds
                // — and None must mean "changed", never "unchanged".
                let changed = arena.values[dst]
                    .reflect_partial_eq(&*incoming)
                    .map(|equal| !equal)
                    .unwrap_or(true);
                arena.values[dst] = incoming;
                dirty |= changed;
            }

            let current = (d.inlets_changed_tick)(world, plan.entity);
            let last = world
                .get::<NodeRuntime>(plan.entity)
                .and_then(|r| r.last_inlets_tick);
            if last != current {
                (d.prefill)(world, plan.entity, &mut arena, plan);
                dirty = true;
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_inlets_tick = current;
                }
            }

            // Only touch NodeRuntime when there is something to record — an
            // unconditional get_mut churns its change tick every tick.
            if dirty && let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = true;
            }

            let mut view = PortView::new(
                &mut arena,
                plan.base,
                &plan.fields,
                &plan.field_offsets,
                &plan.field_lens,
                &plan.connected,
            );
            (d.tick)(world, plan.entity, &mut view, &ctx);

            // The cook, immediately after this node's tick. Its own params
            // are already applied, and every upstream product it reads has
            // already cooked, because the same order guarantees both.
            let Some(cook_fn) = d.cook else {
                continue;
            };

            let sources: Vec<Option<Tick>> = plan
                .product_inlets
                .iter()
                .map(|&(slot, access)| {
                    let source = (access.get)(&*arena.values[slot])?;
                    let source_plan = *compiled.plan_index_of.get(&source)?;
                    (dispatch[source_plan].produced_change_tick)(world, source)
                })
                .collect();

            let cook_dirty = match world.get::<NodeRuntime>(plan.entity) {
                Some(rt) => rt.cook_dirty || rt.last_product_ticks != sources,
                None => false,
            };
            if !cook_dirty {
                continue;
            }

            let view = PortView::new(
                &mut arena,
                plan.base,
                &plan.fields,
                &plan.field_offsets,
                &plan.field_lens,
                &plan.connected,
            );
            cook_fn(world, plan.entity, &view);

            if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = false;
                rt.last_product_ticks = sources;
            }
        }
    });

    world.insert_resource(compiled);
}

/// Inserts the graph engine's resources and wires `graph_tick` into
/// `FixedUpdate`.
pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PortArena::new(0))
            .init_resource::<NodeTypeRegistry>()
            .init_resource::<GraphTickCount>()
            .init_resource::<Time<crate::transport::Transport>>()
            .register_type::<crate::edges::EditorPos>()
            .register_type::<crate::transport::Transport>()
            .register_type::<crate::transport::TransportState>()
            .add_systems(FixedUpdate, graph_tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_nodes::{
        connect, connect_at, engine_app, event_offsets, port_value, recompile, spawn_consumer,
        spawn_emitter, spawn_gain, spawn_producer, spawn_sink, spawn_sum, Consumer, Emitter, Gain,
        GainInlets, Producer, Sink, Sum,
    };

    #[test]
    fn an_edge_carries_a_value_within_one_tick() {
        // Writes are immediate, so a node later in topological order sees an
        // earlier node's output in the SAME tick — not one tick late.
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 0.0, 5.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(port_value(&app, b, Gain::OUT_VALUE), 30.0, "6.0 * 5.0 in one tick");
    }

    #[test]
    fn an_unconnected_input_reads_its_authored_value() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 4.0, 0.5);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 2.0);
    }

    #[test]
    fn a_connected_input_shadows_the_authored_value_without_overwriting_it() {
        // `Inlets` holds what the author wrote; the arena holds what the
        // edge is sending. Saving a project must not bake in the latter.
        let mut app = engine_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 7.0, "driven, not authored");
        assert_eq!(
            app.world().get::<GainInlets>(dst).unwrap().gain,
            4.0,
            "Inlets must be untouched by the graph"
        );
    }

    #[test]
    fn disconnecting_and_recompiling_returns_the_port_to_its_authored_value() {
        let mut app = engine_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        let e = connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 7.0);

        app.world_mut().despawn(e);
        recompile(&mut app);
        app.update();

        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 4.0);
    }

    #[test]
    fn an_inlets_change_is_seen_however_many_ticks_later_it_is_read() {
        // THE `Changed<T>` FAILURE MODE. A filter would be true for exactly
        // one tick; this must hold across many.
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        for _ in 0..10 {
            app.update();
        }

        app.world_mut().get_mut::<GainInlets>(a).unwrap().gain = 9.0;
        for _ in 0..10 {
            app.update();
        }

        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 9.0);
    }

    #[test]
    fn an_unchanged_node_does_not_reprefill() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        app.update();
        let first = app.world().get::<NodeRuntime>(a).unwrap().last_inlets_tick;
        app.update();
        let second = app.world().get::<NodeRuntime>(a).unwrap().last_inlets_tick;
        assert_eq!(first, second, "gate must not re-fire on an unchanged node");
    }

    #[test]
    fn event_slots_are_empty_at_the_start_of_every_tick() {
        let mut app = engine_app();
        let e = spawn_emitter(app.world_mut(), 0.001); // emits one occurrence per tick
        let sink = spawn_sink(app.world_mut());
        connect(app.world_mut(), e, Emitter::OUT_PULSE, sink, Sink::PULSE);
        recompile(&mut app);
        app.update();
        assert_eq!(event_offsets(&app, sink, Sink::PULSE).len(), 1);
        app.update();
        assert_eq!(
            event_offsets(&app, sink, Sink::PULSE).len(),
            1,
            "not 2 — cleared each tick"
        );
    }

    #[test]
    fn merged_event_streams_arrive_in_offset_order() {
        // The offset a node reads back is whatever the source wrote, not
        // reordered by the copy.
        let mut app = engine_app();
        let early = spawn_emitter(app.world_mut(), 0.001);
        let sink = spawn_sink(app.world_mut());
        connect(app.world_mut(), early, Emitter::OUT_PULSE, sink, Sink::PULSE);
        recompile(&mut app);

        app.update();

        assert_eq!(event_offsets(&app, sink, Sink::PULSE), vec![0.001]);
    }

    #[test]
    fn a_changed_driven_input_dirties_the_node() {
        // The case that fails if the gate reads Inlets change ticks alone: a
        // connected port shadows the authored value, so Inlets never moves
        // while the effective value changes every tick.
        let mut app = engine_app();
        // bias must be nonzero: Gain::tick writes `gain * bias`, so with
        // bias == 0.0 the output would be pinned at 0.0 for every value of
        // gain and this test could never distinguish "changed" from "not".
        let src = spawn_gain(app.world_mut(), 2.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 1.0, 0.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();
        // Clear the compile-time dirty so the next assertion is about gather.
        app.world_mut().get_mut::<NodeRuntime>(dst).unwrap().cook_dirty = false;

        app.world_mut().get_mut::<GainInlets>(src).unwrap().gain = 5.0;
        app.update();

        assert!(
            app.world().get::<NodeRuntime>(dst).unwrap().cook_dirty,
            "a driven input that changed must dirty its node"
        );
    }

    #[test]
    fn a_steady_driven_input_does_not_dirty_the_node() {
        let mut app = engine_app();
        let src = spawn_gain(app.world_mut(), 2.0, 0.0);
        let dst = spawn_gain(app.world_mut(), 1.0, 0.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();
        app.world_mut().get_mut::<NodeRuntime>(dst).unwrap().cook_dirty = false;

        for _ in 0..5 {
            app.update();
        }

        assert!(
            !app.world().get::<NodeRuntime>(dst).unwrap().cook_dirty,
            "an unchanged value must not dirty its node every tick"
        );
    }

    #[test]
    fn an_authored_inlet_edit_dirties_the_node() {
        let mut app = engine_app();
        let a = spawn_gain(app.world_mut(), 1.0, 0.0);
        recompile(&mut app);
        app.update();
        app.world_mut().get_mut::<NodeRuntime>(a).unwrap().cook_dirty = false;

        app.world_mut().get_mut::<GainInlets>(a).unwrap().gain = 3.0;
        app.update();

        assert!(app.world().get::<NodeRuntime>(a).unwrap().cook_dirty);
    }

    #[test]
    fn a_variadic_inlet_sums_every_connected_element() {
        let mut app = engine_app();
        let sum = spawn_sum(app.world_mut(), vec![0.0, 0.0]);
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 4.0, 5.0);
        connect_at(app.world_mut(), a, Gain::OUT_VALUE, sum, Sum::TERMS, 0);
        connect_at(app.world_mut(), b, Gain::OUT_VALUE, sum, Sum::TERMS, 1);
        recompile(&mut app);

        app.update();
        app.update();

        assert_eq!(port_value(&app, sum, Sum::OUT_TOTAL), 26.0, "6 + 20");
    }

    mod cooking {
        use super::*;
        use crate::registry::{register_node_type, NodeType};
        use crate::schema::register_product;
        use crate::test_nodes::{Blob, ProducerInlets, ProducerOutlets, ProducerState};
        use bevy_app::App;
        use bevy_ecs::entity::Entity;
        use bevy_ecs::world::World;

        fn chain(app: &mut App) -> (Entity, Entity) {
            let src = spawn_producer(app.world_mut());
            let sink = spawn_consumer(app.world_mut());
            connect(app.world_mut(), src, Producer::OUT_BLOB, sink, Consumer::INPUT);
            recompile(app);
            (src, sink)
        }

        #[test]
        fn every_node_cooks_exactly_once_after_compilation() {
            let mut app = engine_app();
            let (src, sink) = chain(&mut app);

            app.update();

            assert_eq!(app.world().get::<ProducerState>(src).unwrap().cooks, 1);
            assert_eq!(
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks,
                1
            );
        }

        #[test]
        fn a_steady_graph_cooks_nothing_after_the_first_tick() {
            let mut app = engine_app();
            let (src, sink) = chain(&mut app);
            app.update();
            let src_after_first = app.world().get::<ProducerState>(src).unwrap().cooks;
            let sink_after_first =
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks;

            for _ in 0..10 {
                app.update();
            }

            assert_eq!(
                app.world().get::<ProducerState>(src).unwrap().cooks,
                src_after_first,
                "an idle graph must not cook"
            );
            assert_eq!(
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks,
                sink_after_first
            );
        }

        #[test]
        fn a_value_change_on_one_node_does_not_cook_a_node_it_does_not_feed() {
            // Dirt flows with the edge direction only. Editing the
            // consumer's own authored value must not re-cook the producer
            // above it.
            let mut app = engine_app();
            let (src, sink) = chain(&mut app);
            app.update();
            let baseline = app.world().get::<ProducerState>(src).unwrap().cooks;

            app.world_mut()
                .get_mut::<crate::test_nodes::ConsumerInlets>(sink)
                .unwrap()
                .scale = 2.0;
            app.update();

            assert_eq!(
                app.world().get::<ProducerState>(src).unwrap().cooks,
                baseline,
                "the producer must not re-cook for a change downstream of it"
            );
        }

        #[test]
        fn a_node_added_after_an_upstream_cook_still_cooks_against_it() {
            // Recompiling always marks every node's cook gate dirty (a fresh
            // `NodeRuntime` is inserted for every plan) — which is what lets
            // a node joining mid-session cook against an already-settled
            // upstream, something a `Changed<T>` filter could not do.
            let mut app = engine_app();
            let src = spawn_producer(app.world_mut());
            recompile(&mut app);
            for _ in 0..5 {
                app.update();
            }
            let baseline = app.world().get::<ProducerState>(src).unwrap().cooks;

            let sink = spawn_consumer(app.world_mut());
            connect(app.world_mut(), src, Producer::OUT_BLOB, sink, Consumer::INPUT);
            recompile(&mut app);
            app.update();

            assert!(
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks > 0,
                "the new node must cook"
            );
            assert!(app.world().get::<ProducerState>(src).unwrap().cooks > baseline);
        }

        /// Like `Producer`, but its `produced_change_tick` tracks its own
        /// `ProducerState` — which changes every time `cook` runs. `Producer`
        /// itself defaults to `None` ("editing a material's params does not
        /// change its handle" is the right default for a material node), so
        /// demonstrating same-tick propagation from an upstream cook to a
        /// downstream one needs a node that opts in, exactly as a real
        /// geometry-producing node will from Task 5 onward.
        struct TrackedProducer;

        impl NodeType for TrackedProducer {
            type Inlets = ProducerInlets;
            type Outlets = ProducerOutlets;
            type State = ProducerState;

            const ORDINALS: &'static [(&'static str, u16)] =
                &[("scale", 0), ("blob", 1)];
            const COOKS: bool = true;

            fn register(app: &mut App) {
                register_product::<Blob>(app);
            }

            fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

            fn cook(world: &mut World, node: Entity, _ports: &PortView) {
                world.get_mut::<ProducerState>(node).expect("state").cooks += 1;
            }

            fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
                world
                    .get_entity(node)
                    .ok()?
                    .get_change_ticks::<ProducerState>()
                    .map(|t| t.changed)
            }
        }

        fn tracked_chain(app: &mut App) -> (Entity, Entity) {
            let src_id = register_node_type::<TrackedProducer>(app);
            let src = {
                let mut query = app.world_mut().query::<&crate::edges::GraphNode>();
                let id = crate::edges::NodeId(query.iter(app.world()).count() as u32);
                app.world_mut()
                    .spawn((
                        crate::edges::GraphNode { id, node_type: src_id },
                        ProducerInlets::default(),
                        ProducerState::default(),
                    ))
                    .id()
            };
            let sink = spawn_consumer(app.world_mut());
            connect(app.world_mut(), src, Producer::OUT_BLOB, sink, Consumer::INPUT);
            recompile(app);
            (src, sink)
        }

        #[test]
        fn an_upstream_cook_propagates_to_its_feeds_consumer_in_the_same_tick() {
            let mut app = engine_app();
            let (src, sink) = tracked_chain(&mut app);
            app.update();
            let src_baseline = app.world().get::<ProducerState>(src).unwrap().cooks;
            let sink_baseline =
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks;

            app.world_mut().get_mut::<ProducerInlets>(src).unwrap().scale = 7.0;
            app.update();

            assert!(app.world().get::<ProducerState>(src).unwrap().cooks > src_baseline);
            assert!(
                app.world().get::<crate::test_nodes::ConsumerState>(sink).unwrap().cooks
                    > sink_baseline,
                "the consumer must re-cook in the same tick its upstream product's tick moved"
            );
        }
    }
}
