//! The tick runner: one exclusive system in `FixedUpdate` that walks the
//! compiled plan. Spec §6.

use bevy_app::{App, FixedUpdate, Plugin};
use bevy_ecs::change_detection::Mut;
use bevy_ecs::resource::Resource;
use bevy_ecs::world::World;
use bevy_reflect::PartialReflect;
use bevy_time::{Fixed, Time};

use crate::compile::CompiledGraph;
use crate::edges::NodeRuntime;
use crate::ports::{Occurrence, PortArena};
use crate::registry::{NodeTypeRegistry, PrefillFn, SeedOutputsFn, TickFn, TickOfFn};
use crate::view::{PortView, TickCtx};

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
    // so the four fn pointers per plan are copied out into locals here,
    // before the loop, rather than holding a `&NodeTypeEntry` across it. Fn
    // pointers are `Copy`, so this is a cheap, allocation-light snapshot.
    let entries: Vec<(TickFn, PrefillFn, SeedOutputsFn, TickOfFn)> = {
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
                (entry.tick, entry.prefill, entry.seed_outputs, entry.params_changed_tick)
            })
            .collect()
    };

    world.resource_scope(|world: &mut World, mut arena: Mut<PortArena>| {
        if !compiled.outputs_seeded {
            for (plan, &(_, _, seed_outputs_fn, _)) in compiled.plans.iter().zip(&entries) {
                seed_outputs_fn(&mut arena, plan);
            }
            compiled.outputs_seeded = true;
        }

        arena.clear_events();

        for (plan, &(tick_fn, prefill_fn, _, params_changed_tick_fn)) in
            compiled.plans.iter().zip(&entries)
        {
            // Gather: copy each incoming edge's source slot into the input
            // slot. Continuous overwrites; events append (already merged in
            // source-rank order by the compiler).
            for &(src, dst) in &plan.continuous_copies {
                arena.continuous[dst] = clone_slot(&*arena.continuous[src]);
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
                if let Some(mut rt) = world.get_mut::<NodeRuntime>(plan.entity) {
                    rt.last_params_tick = current;
                }
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
}
