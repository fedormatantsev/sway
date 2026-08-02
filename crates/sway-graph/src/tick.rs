//! The tick runner: one exclusive system in `FixedUpdate` that walks the
//! compiled plan. Spec §6.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::change_detection::{Mut, Tick};
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use bevy_time::{Fixed, Time};

use crate::compile::CompiledGraph;
use crate::edges::NodeRuntime;
use crate::ports::{Occurrence, PortArena};
use crate::registry::{
    CookFn, NodeTypeRegistry, PrefillFn, ProducedTickFn, SeedOutputsFn, TickFn, TickOfFn,
};
use crate::view::{PortView, SlotView, TickCtx};

/// Ticks since the graph started running, incremented once per `graph_tick`
/// call. Exposed as `TickCtx::tick_index`.
#[derive(Resource, Default)]
pub struct GraphTickCount(pub u64);

/// Clones a slot's value while preserving its concrete type.
///
/// `PartialReflect::to_dynamic` is the wrong tool here even though it reads
/// naturally: for `ReflectKind::Struct` (and List/Map/Enum/...) it returns a
/// `Dynamic*` proxy, not a value of the original concrete type — by design,
/// per its own doc ("generally returns a dynamic representation of `Self`").
/// A later `try_downcast_ref::<T>()` against that proxy fails, because a
/// `DynamicStruct` does not implement `Reflect` (only `PartialReflect`) and
/// so cannot downcast to `T` at all. `reflect_clone` is the method whose
/// documented job is producing "a clone of `Self` directly" — i.e. a real
/// `T` — which `derive(Reflect)` implements for every field by default. A
/// plain `f32` continuous port round-trips through `to_dynamic` only because
/// `ReflectKind::Opaque` happens to fall back to `reflect_clone` internally;
/// an `Event<T>` occurrence with a struct payload does not get that
/// fallback, which is what `merged_event_streams_arrive_in_offset_order`
/// caught.
fn clone_slot(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .unwrap_or_else(|e| {
            panic!(
                "graph_tick: could not clone a `{}` port value while gathering an edge ({e:?}) \
                 — the compiler should only ever produce edges between `#[derive(Reflect)]` \
                 types, which clone by default",
                value.reflect_type_path()
            )
        })
        .into_partial_reflect()
}

/// The graph tick: one exclusive system in `FixedUpdate` (spec §2.6/§6).
///
/// No-ops if `CompiledGraph` is not present (nothing has been compiled yet,
/// or compilation failed). Everything this calls is infallible: a compiled
/// graph has already been validated by `compile`, so any failure to look up
/// a registry entry or downcast a slot below means the compiler failed to
/// catch something it should have — not a condition to handle gracefully.
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
    let ctx = TickCtx {
        dt,
        tick_start,
        tick_index,
    };

    // The registry borrow: `world` is later borrowed mutably for `tick_fn`,
    // so the six fn pointers per plan are copied out into locals here,
    // before the loop, rather than holding a `&NodeTypeEntry` across it. Fn
    // pointers are `Copy`, so this is a cheap, allocation-light snapshot.
    let entries: Vec<(
        TickFn,
        PrefillFn,
        SeedOutputsFn,
        TickOfFn,
        Option<CookFn>,
        ProducedTickFn,
    )> = {
        let registry = world.resource::<NodeTypeRegistry>();
        compiled
            .plans
            .iter()
            .map(|plan| {
                let entry = registry.get(plan.node_type).unwrap_or_else(|| {
                    panic!(
                        "graph_tick: node {:?}'s node type {:?} is not in the registry — the \
                         compiler should have caught this",
                        plan.entity, plan.node_type
                    )
                });
                (
                    entry.tick,
                    entry.prefill,
                    entry.seed_outputs,
                    entry.params_changed_tick,
                    entry.cook,
                    entry.produced_change_tick,
                )
            })
            .collect()
    };

    world.resource_scope(|world: &mut World, mut arena: Mut<PortArena>| {
        if !compiled.outputs_seeded {
            for (plan, &(_, _, seed_outputs_fn, _, _, _)) in compiled.plans.iter().zip(&entries) {
                seed_outputs_fn(&mut arena, plan);
            }
            compiled.outputs_seeded = true;
        }

        arena.clear_events();

        for (plan, &(tick_fn, prefill_fn, _, params_changed_tick_fn, _, _)) in
            compiled.plans.iter().zip(&entries)
        {
            // `dirty` accumulates this tick's reasons to cook; it is OR-ed
            // into the sticky flag below rather than assigned, so a reason
            // raised on an earlier tick is not lost (design §6).
            let mut dirty = false;

            // Gather: copy each incoming edge's source slot into the input
            // slot. Continuous overwrites; events append (already merged in
            // source-rank order by the compiler).
            for &(src, dst) in &plan.continuous_copies {
                let incoming = clone_slot(&*arena.continuous[src]);
                // `reflect_partial_eq` returns None for values that cannot be
                // compared — including the `()` a freshly-resized arena slot
                // holds — and None must mean "changed", never "unchanged".
                let changed = arena.continuous[dst]
                    .reflect_partial_eq(&*incoming)
                    .map(|equal| !equal)
                    .unwrap_or(true);
                arena.continuous[dst] = incoming;
                dirty |= changed;
            }
            for &(src, dst) in &plan.event_merges {
                // `arena.events[src]` and `arena.events[dst]` alias the same
                // `Vec<Vec<Occurrence>>`, so the copy needs a temporary — the
                // one per-tick allocation this design does not avoid.
                let copied: Vec<Occurrence> = arena.events[src]
                    .iter()
                    .map(|o| Occurrence {
                        offset: o.offset,
                        value: clone_slot(&*o.value),
                    })
                    .collect();
                arena.events[dst].extend(copied);
            }
            for event_input in &mut arena.events
                [plan.event_base..plan.event_base + plan.schema.inputs.events.len()]
            {
                // Stable sorting supplies the primary offset order while
                // preserving the compiler's source-rank order for ties.
                event_input.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            }

            // Prefill, gated on the Params change tick (spec §4). A plain
            // inequality: we only need "did it move", not
            // `Tick::is_newer_than`'s wraparound-aware ordering, so no
            // `this_run` tick is needed either.
            let current = params_changed_tick_fn(world, plan.entity);
            let last = world
                .get::<NodeRuntime>(plan.entity)
                .and_then(|r| r.last_params_tick);
            if last != current {
                prefill_fn(world, plan.entity, &mut arena, plan);
                dirty = true;
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_params_tick = current;
                }
            }

            // Only touch NodeRuntime when there is something to record —
            // an unconditional `get_mut` would churn its change tick every
            // tick for every node.
            if dirty && let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = true;
            }

            // Dispatch.
            let mut view = PortView::new(
                &mut arena,
                plan.continuous_base,
                plan.event_base,
                plan.schema.continuous_len(),
                plan.schema.events_len(),
                &plan.connected_continuous,
            );
            tick_fn(world, plan.entity, &mut view, &ctx);
        }

        // --- Pass 2: cooks, in Feeds order (design §7) --------------------
        //
        // Ticks precede cooks globally, so a cook always sees its own node's
        // effective params already applied — parent §2.11's step B before its
        // step C. Inside the resource_scope, so the arena is provably out of
        // the world here too: a cook has no business touching ports.
        for &plan_idx in &compiled.cook_order {
            let Some(cook_fn) = entries[plan_idx].4 else {
                continue;
            };
            let plan = &compiled.plans[plan_idx];

            // Stored ticks, kept for the geometry side only — a product is
            // large and not usefully value-compared (design §6). A source
            // whose `produced_change_tick` is None never dirties its
            // consumers, which is exactly right for a material handle.
            let current: Vec<Option<Tick>> = plan
                .slots
                .iter()
                .map(|slot| {
                    slot.and_then(|source| (entries[source.plan_index].5)(world, source.entity))
                })
                .collect();

            let dirty = match world.get::<NodeRuntime>(plan.entity) {
                Some(rt) => rt.cook_dirty || rt.last_slot_ticks != current,
                None => false,
            };
            if !dirty {
                continue;
            }

            let view = SlotView::new(&plan.slots);
            cook_fn(world, plan.entity, &view);

            if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                rt.cook_dirty = false;
                rt.last_slot_ticks = current;
            }
        }
    });

    world.insert_resource(compiled);
}

/// Inserts the graph engine's resources and wires `graph_tick` into
/// `FixedUpdate`. Does not register any node types — callers add those with
/// `register_node_type`.
pub struct GraphPlugin;

impl Plugin for GraphPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PortArena::new(0, 0))
            .init_resource::<NodeTypeRegistry>()
            .init_resource::<GraphTickCount>()
            .register_type::<crate::edges::EditorPos>()
            .add_systems(FixedUpdate, graph_tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_nodes::{
        Emitter, Gain, GainParams, Sink, connect, connect_event, emitter_app, event_count,
        gain_app, port_value, recompile, sink_offsets, spawn_emitter, spawn_emitter_at, spawn_gain,
        spawn_sink,
    };

    #[test]
    fn an_edge_carries_a_value_within_one_tick() {
        // Spec §6: writes are immediate, so a node later in topological order
        // sees an earlier node's output in the SAME tick — not one tick late.
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 2.0, 3.0);
        let b = spawn_gain(app.world_mut(), 0.0, 5.0);
        connect(app.world_mut(), a, Gain::OUT_VALUE, b, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(
            port_value(&app, b, Gain::OUT_VALUE),
            30.0,
            "6.0 * 5.0 in one tick"
        );
    }

    #[test]
    fn an_unconnected_input_reads_its_authored_value() {
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 4.0, 0.5);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 2.0);
    }

    #[test]
    fn a_connected_input_shadows_the_authored_value_without_overwriting_it() {
        // Spec §4: Params holds what the author wrote; the arena holds what
        // the edge is sending. Saving a project must not bake in the latter.
        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);

        app.update();

        assert_eq!(
            port_value(&app, dst, Gain::OUT_VALUE),
            7.0,
            "driven, not authored"
        );
        assert_eq!(
            app.world().get::<GainParams>(dst).unwrap().gain,
            4.0,
            "Params must be untouched by the graph"
        );
    }

    #[test]
    fn disconnecting_and_recompiling_returns_the_port_to_its_authored_value() {
        let mut app = gain_app();
        let src = spawn_gain(app.world_mut(), 7.0, 1.0);
        let dst = spawn_gain(app.world_mut(), 4.0, 1.0);
        let e = connect(app.world_mut(), src, Gain::OUT_VALUE, dst, Gain::GAIN);
        recompile(&mut app);
        app.update();
        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 7.0);

        app.world_mut().despawn(e);
        recompile(&mut app);
        app.update();

        // Spec §4: not frozen where the edge left it.
        assert_eq!(port_value(&app, dst, Gain::OUT_VALUE), 4.0);
    }

    #[test]
    fn a_params_change_is_seen_however_many_ticks_later_it_is_read() {
        // THE `Changed<T>` FAILURE MODE (spec §4, §9). A filter would be true
        // for exactly one tick; this must hold across many.
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        for _ in 0..10 {
            app.update();
        }

        app.world_mut().get_mut::<GainParams>(a).unwrap().gain = 9.0;
        for _ in 0..10 {
            app.update();
        }

        assert_eq!(port_value(&app, a, Gain::OUT_VALUE), 9.0);
    }

    #[test]
    fn an_unchanged_node_does_not_reprefill() {
        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 1.0);
        recompile(&mut app);
        app.update();
        let first = app.world().get::<NodeRuntime>(a).unwrap().last_params_tick;
        app.update();
        let second = app.world().get::<NodeRuntime>(a).unwrap().last_params_tick;
        assert_eq!(first, second, "gate must not re-fire on an unchanged node");
    }

    #[test]
    fn event_slots_are_empty_at_the_start_of_every_tick() {
        let mut app = emitter_app(); // emits one occurrence per tick
        let e = spawn_emitter(app.world_mut());
        recompile(&mut app);
        app.update();
        assert_eq!(event_count(&app, e, Emitter::OUT_PULSE), 1);
        app.update();
        assert_eq!(
            event_count(&app, e, Emitter::OUT_PULSE),
            1,
            "not 2 — cleared each tick"
        );
    }

    #[test]
    fn merged_event_streams_arrive_in_offset_order() {
        // Spec §5: sorted by (offset, source's compiled index).
        let mut app = emitter_app();
        let late = spawn_emitter_at(app.world_mut(), 0.006);
        let early = spawn_emitter_at(app.world_mut(), 0.001);
        let sink = spawn_sink(app.world_mut());
        connect_event(
            app.world_mut(),
            late,
            Emitter::OUT_PULSE,
            sink,
            Sink::IN_PULSE,
        );
        connect_event(
            app.world_mut(),
            early,
            Emitter::OUT_PULSE,
            sink,
            Sink::IN_PULSE,
        );
        recompile(&mut app);

        app.update();

        assert_eq!(sink_offsets(&app, sink), vec![0.001, 0.006]);
    }

    #[test]
    fn a_changed_driven_input_dirties_the_node() {
        // The case that fails if the gate reads Params change ticks: a
        // connected port shadows the authored value, so Params never moves
        // while the effective parameter changes every tick (design §6).
        use crate::test_nodes::{Gain, spawn_gain};

        let mut app = gain_app();
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

        app.world_mut().get_mut::<GainParams>(src).unwrap().gain = 5.0;
        app.update();

        assert!(
            app.world().get::<NodeRuntime>(dst).unwrap().cook_dirty,
            "a driven input that changed must dirty its node"
        );
    }

    #[test]
    fn a_steady_driven_input_does_not_dirty_the_node() {
        use crate::test_nodes::{Gain, spawn_gain};

        let mut app = gain_app();
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
    fn an_authored_param_edit_dirties_the_node() {
        use crate::test_nodes::{spawn_gain};

        let mut app = gain_app();
        let a = spawn_gain(app.world_mut(), 1.0, 0.0);
        recompile(&mut app);
        app.update();
        app.world_mut().get_mut::<NodeRuntime>(a).unwrap().cook_dirty = false;

        app.world_mut().get_mut::<GainParams>(a).unwrap().gain = 3.0;
        app.update();

        assert!(app.world().get::<NodeRuntime>(a).unwrap().cook_dirty);
    }

    mod cooking {
        use super::*;
        use bevy_ecs::entity::Entity;
        use crate::edges::{EdgeFrom, EdgeTo, FeedsEdge};
        use crate::test_nodes::{
            BlobData, CookCounter, SinkGeoParams, SourceParams, spawn_sinkgeo, spawn_source,
            structure_app,
        };

        fn cooks(app: &App) -> u32 {
            app.world().resource::<CookCounter>().0
        }

        fn chain(app: &mut App) -> (Entity, Entity) {
            let src = spawn_source(app.world_mut());
            let sink = spawn_sinkgeo(app.world_mut());
            app.world_mut()
                .spawn((FeedsEdge { slot: 0 }, EdgeFrom(src), EdgeTo(sink)));
            recompile(app);
            (src, sink)
        }

        #[test]
        fn every_node_cooks_exactly_once_after_compilation() {
            let mut app = structure_app();
            let (_src, sink) = chain(&mut app);

            app.update();

            assert_eq!(cooks(&app), 2, "one cook each");
            assert!(app.world().get::<BlobData>(sink).is_some());
        }

        #[test]
        fn a_steady_graph_cooks_nothing_after_the_first_tick() {
            // The negative assertion §10 asks for, on a counter rather than
            // on an output that merely happens to be unchanged.
            let mut app = structure_app();
            let _ = chain(&mut app);
            app.update();
            let after_first = cooks(&app);

            for _ in 0..10 {
                app.update();
            }

            assert_eq!(cooks(&app), after_first, "an idle graph must not cook");
        }

        #[test]
        fn an_upstream_cook_propagates_to_its_feeds_consumer_in_the_same_tick() {
            let mut app = structure_app();
            let (src, sink) = chain(&mut app);
            app.update();
            let baseline = cooks(&app);

            app.world_mut().get_mut::<SourceParams>(src).unwrap().seed = 7.0;
            app.update();

            assert_eq!(cooks(&app), baseline + 2, "both ends re-cook");
            assert_eq!(app.world().get::<BlobData>(sink), Some(&BlobData(7)));
        }

        #[test]
        fn a_param_change_on_one_node_does_not_cook_its_upstream() {
            // Dirt flows with Feeds direction only. A downstream param edit
            // must not re-cook the operator above it.
            let mut app = structure_app();
            let (_src, sink) = chain(&mut app);
            app.update();
            let baseline = cooks(&app);

            app.world_mut().get_mut::<SinkGeoParams>(sink).unwrap().scale = 2.0;
            app.update();

            assert_eq!(cooks(&app), baseline + 1, "only the edited node cooks");
        }

        #[test]
        fn a_node_added_after_an_upstream_cook_still_cooks_against_it() {
            // §2.11's robustness case: the gate must survive a node joining
            // mid-session, which a `Changed<T>` filter would not.
            let mut app = structure_app();
            let src = spawn_source(app.world_mut());
            recompile(&mut app);
            app.update();
            for _ in 0..5 {
                app.update();
            }
            let baseline = cooks(&app);

            let sink = spawn_sinkgeo(app.world_mut());
            app.world_mut()
                .spawn((FeedsEdge { slot: 0 }, EdgeFrom(src), EdgeTo(sink)));
            recompile(&mut app);
            app.update();

            assert!(cooks(&app) > baseline, "the new node must cook");
            assert!(app.world().get::<BlobData>(sink).is_some());
        }
    }
}
