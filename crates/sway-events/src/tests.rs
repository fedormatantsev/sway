//! Behaviour tests over a real `Graph`, driven by `sway-graph`'s tick harness.
//!
//! Everything asserted here goes through the actual tick — the real order, the
//! real propagate step, the real dirty rule — rather than a model of it
//! (design D10).

use bevy_app::App;
use bevy_ecs::world::World;
use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
use sway_graph::graph::testing::{TICK_HZ, read_field, set_field, tick_once, trace_world};
use sway_graph::graph::{ConnectError, Graph, Node, NodeId, Part, Port};
use sway_graph::{GraphPlugin, GraphTickSet};

use crate::arena::EventArena;
use crate::fixtures::{
    Emitter, EmitterIn, Held, HeldIn, Maybe, Ping, Relay, Tally, register_fixtures, test_registry,
};
use crate::handle::EventHandle;
use crate::plugin::EventsPlugin;

/// The fixture world, with the arena in it as the plugin would insert it.
fn arena_world() -> World {
    let mut world = trace_world(test_registry());
    world.insert_non_send(EventArena::default());
    world
}

/// A world with the fixtures registered but **no arena at all**.
fn bare_world() -> World {
    trace_world(test_registry())
}

/// One tick in the order the schedule runs it: empty the arena, then tick.
fn tick(graph: &mut Graph, world: &mut World) {
    if let Some(mut arena) = world.get_non_send_mut::<EventArena>() {
        arena.clear();
    }
    tick_once(graph, world);
}

fn emitter(count: f32) -> Emitter {
    Emitter {
        inlets: EmitterIn { count },
        ..Default::default()
    }
}

fn handle_at(graph: &Graph, node: NodeId, part: Part, field: &str) -> EventHandle<Ping> {
    read_field::<EventHandle<Ping>>(graph, node, part, field)
}

/// Reads a node's outlet handle through the arena.
fn batch_of(world: &World, graph: &Graph, node: NodeId, field: &str) -> Vec<Ping> {
    let handle = handle_at(graph, node, Part::Outlets, field);
    world
        .get_non_send::<EventArena>()
        .and_then(|arena| arena.read(handle))
        .map(|batch| batch.into_iter().copied().collect())
        .unwrap_or_default()
}

// --- 5.2 connect legality ----------------------------------------------

#[test]
fn two_handles_of_one_payload_connect_like_any_other_edge() {
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(1.0)));
    let sink = graph.insert(Node::of(Tally::default()));

    let edge = graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("a handle is an ordinary value on a wire");

    assert_eq!(
        graph.edge(edge).expect("the edge").compat,
        sway_graph::graph::Compat::Direct,
        "no new kind of connection was required for it"
    );
}

#[test]
fn handles_of_different_payloads_are_refused_at_connect() {
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(1.0)));
    let sink = graph.insert(Node::of(Maybe::default()));

    let refused = graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pongs"), 0)
        .expect_err("Ping and Pong are different payloads");

    assert!(
        matches!(refused, ConnectError::IncompatibleTypes { .. }),
        "refused when the connection is made, with no evaluation: {refused:?}"
    );
}

#[test]
fn an_option_of_a_handle_is_optional_and_a_vec_of_one_is_variadic() {
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(1.0)));
    let sink = graph.insert(Node::of(Maybe::default()));

    let optional = graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("Option<EventHandle<Ping>> takes an EventHandle<Ping>");
    let variadic = graph
        .connect(Port::new(source, "pings"), Port::new(sink, "many"), 0)
        .expect("Vec<EventHandle<Ping>> takes an EventHandle<Ping>");

    assert_eq!(
        graph.edge(optional).expect("the edge").compat,
        sway_graph::graph::Compat::Optional
    );
    assert_eq!(
        graph.edge(variadic).expect("the edge").compat,
        sway_graph::graph::Compat::Variadic
    );
}

// --- 5.3 same-tick delivery --------------------------------------------

#[test]
fn a_two_hop_trigger_chain_resolves_in_one_tick() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let a = graph.insert(Node::of(emitter(2.0)));
    let b = graph.insert(Node::of(Relay::default()));
    let c = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(a, "pings"), Port::new(b, "pings"), 0)
        .expect("legal");
    graph
        .connect(Port::new(b, "pings"), Port::new(c, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);

    assert_eq!(
        read_field::<f32>(&graph, c, Part::Outlets, "count"),
        2.0,
        "the second hop must land in the SAME tick"
    );
    assert_eq!(
        read_field::<f32>(&graph, c, Part::Outlets, "sum"),
        201.0,
        "and what arrived is the relay's own batch (0+100, 1+100)"
    );
}

#[test]
fn forwarding_publishes_a_new_batch() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let a = graph.insert(Node::of(emitter(2.0)));
    let b = graph.insert(Node::of(Relay::default()));
    graph
        .connect(Port::new(a, "pings"), Port::new(b, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);

    let received = handle_at(&graph, b, Part::Inlets, "pings");
    let published = handle_at(&graph, b, Part::Outlets, "pings");
    assert_ne!(
        received, published,
        "the handle on its outlet is not the handle it received"
    );
    assert_eq!(
        batch_of(&world, &graph, b, "pings"),
        vec![Ping(100), Ping(101)]
    );
}

// --- 5.4 fan-out -------------------------------------------------------

#[test]
fn two_consumers_of_one_outlet_read_the_same_batch() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(2.0)));
    let first = graph.insert(Node::of(Tally::default()));
    let second = graph.insert(Node::of(Tally::default()));
    for sink in [first, second] {
        graph
            .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
            .expect("legal");
    }

    tick(&mut graph, &mut world);

    assert_eq!(
        handle_at(&graph, first, Part::Inlets, "pings"),
        handle_at(&graph, second, Part::Inlets, "pings"),
        "the batch is not duplicated per connection"
    );
    for sink in [first, second] {
        assert_eq!(read_field::<f32>(&graph, sink, Part::Outlets, "count"), 2.0);
        assert_eq!(read_field::<f32>(&graph, sink, Part::Outlets, "sum"), 1.0);
    }
}

#[test]
fn one_consumer_reading_does_not_change_what_the_other_reads() {
    let world = arena_world();
    let arena = world.get_non_send::<EventArena>().expect("an arena");
    let handle = arena.publish([Ping(1), Ping(2)]);

    let first = arena.read(handle).expect("a batch");
    let second = arena.read(handle).expect("still every occurrence");

    assert_eq!(first.len(), 2);
    assert_eq!(&*second, &[Ping(1), Ping(2)]);
}

// --- 5.5 producer discipline -------------------------------------------

#[test]
fn a_producer_with_nothing_to_publish_writes_the_empty_handle() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(0.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);

    assert_eq!(
        handle_at(&graph, source, Part::Outlets, "pings"),
        EventHandle::EMPTY
    );
    assert_eq!(read_field::<f32>(&graph, sink, Part::Outlets, "count"), 0.0);
}

#[test]
fn a_producer_keeps_neither_the_occurrences_nor_the_handle() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(3.0)));

    tick(&mut graph, &mut world);

    let state = graph
        .get(source)
        .expect("a live node")
        .part(Part::State)
        .expect("a state part");
    assert!(
        state.try_downcast_ref::<()>().is_some(),
        "an `Emitter`'s whole state is `()`"
    );
    assert_eq!(batch_of(&world, &graph, source, "pings").len(), 3);
}

#[test]
fn a_producer_that_stops_publishing_leaves_nothing_behind() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(2.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);
    assert_eq!(read_field::<f32>(&graph, sink, Part::Outlets, "count"), 2.0);

    set_field(&mut graph, source, "count", &0.0f32);
    tick(&mut graph, &mut world);

    assert_eq!(read_field::<f32>(&graph, sink, Part::Outlets, "count"), 0.0);
}

// --- 5.6 lifetime ------------------------------------------------------

#[test]
fn nothing_survives_to_the_next_tick() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    graph.insert(Node::of(emitter(2.0)));

    tick(&mut graph, &mut world);
    assert_eq!(world.get_non_send::<EventArena>().expect("arena").len(), 1);

    if let Some(mut arena) = world.get_non_send_mut::<EventArena>() {
        arena.clear();
    }

    assert_eq!(
        world.get_non_send::<EventArena>().expect("arena").len(),
        0,
        "the arena holds no batch from the previous tick"
    );
}

#[test]
fn a_stale_handle_reads_empty_and_not_another_producers_batch() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(1.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    let edge = graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");
    tick(&mut graph, &mut world);
    let stale = handle_at(&graph, sink, Part::Inlets, "pings");
    assert_ne!(stale, EventHandle::EMPTY, "a handle naming a live batch");

    // The producer is disconnected, so nothing overwrites the consumer's
    // inlet and it holds a handle from a tick that has ended. Meanwhile
    // another producer fills the very slot that handle names.
    graph.disconnect(edge);
    set_field(&mut graph, source, "count", &0.0f32);
    let other = graph.insert(Node::of(emitter(5.0)));
    tick(&mut graph, &mut world);
    assert_eq!(
        handle_at(&graph, other, Part::Outlets, "pings").slot(),
        stale.slot(),
        "the new producer really did take the slot the stale handle names"
    );

    assert_eq!(
        handle_at(&graph, sink, Part::Inlets, "pings"),
        stale,
        "the inlet still holds the earlier tick's handle"
    );
    assert_eq!(
        read_field::<f32>(&graph, sink, Part::Outlets, "count"),
        0.0,
        "which reads as no occurrences, and the evaluation succeeded"
    );
    assert_eq!(
        batch_of(&world, &graph, other, "pings").len(),
        5,
        "even though another producer's batch is live in that slot this tick"
    );
}

#[test]
fn publishing_every_tick_does_not_grow_the_arena() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    graph.insert(Node::of(emitter(4.0)));
    graph.insert(Node::of(emitter(4.0)));

    for _ in 0..20 {
        tick(&mut graph, &mut world);
        assert_eq!(
            world.get_non_send::<EventArena>().expect("arena").len(),
            2,
            "only the current tick's batches"
        );
    }
}

// --- 5.7 wrappers ------------------------------------------------------

#[test]
fn several_trigger_sources_merge_in_ordering_key_order() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let merge = graph.insert(Node::of(Maybe::default()));
    // Three emitters publishing distinguishable batches: counts 1, 2 and 3
    // publish `[0]`, `[0,1]` and `[0,1,2]`.
    let sources: Vec<NodeId> = [1.0, 2.0, 3.0]
        .into_iter()
        .map(|count| graph.insert(Node::of(emitter(count))))
        .collect();
    for (source, slot) in sources.iter().zip([30, 10, 20]) {
        graph
            .connect(Port::new(*source, "pings"), Port::new(merge, "many"), slot)
            .expect("legal");
    }

    tick(&mut graph, &mut world);

    let merged = read_field::<Vec<f32>>(&graph, merge, Part::Outlets, "merged");
    assert_eq!(
        merged,
        vec![0.0, 1.0, 0.0, 1.0, 2.0, 0.0],
        "keys 10, 20, 30 — the ordinary variadic rule, not a mechanism of its own"
    );
}

#[test]
fn an_unconnected_optional_handle_inlet_is_absent() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let node = graph.insert(Node::of(Maybe::default()));

    tick(&mut graph, &mut world);

    assert!(
        !read_field::<bool>(&graph, node, Part::Outlets, "had_pings"),
        "absent, exactly as for any other optional inlet"
    );
}

#[test]
fn an_unconnected_plain_handle_inlet_reads_as_no_occurrences() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let node = graph.insert(Node::of(Tally::default()));

    tick(&mut graph, &mut world);

    assert_eq!(
        handle_at(&graph, node, Part::Inlets, "pings"),
        EventHandle::EMPTY,
        "no authoring was needed for it"
    );
    assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "count"), 0.0);
}

// --- 5.8 dirty ---------------------------------------------------------

#[test]
fn a_silent_producer_and_its_consumers_report_no_change() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(0.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);
    graph.drain_dirty();
    tick(&mut graph, &mut world);

    assert!(
        graph.drain_dirty().is_empty(),
        "the empty handle replacing the empty handle is the one case that \
         reports nothing"
    );
}

#[test]
fn an_unconditional_producer_with_nothing_to_say_reports_no_change() {
    // `Emitter` publishes on every tick without checking first, so this is the
    // hardening design D7 puts in `publish` rather than in node authors: an
    // empty batch folds to `EMPTY`, tick after tick.
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(0.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");
    tick(&mut graph, &mut world);
    graph.drain_dirty();

    for tick_index in 0..5 {
        tick(&mut graph, &mut world);
        assert!(
            graph.drain_dirty().is_empty(),
            "tick {tick_index} reported a change while carrying nothing"
        );
    }
}

#[test]
fn a_publishing_producer_is_reported_changed_and_so_is_what_it_reaches() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(1.0)));
    let sink = graph.insert(Node::of(Tally::default()));
    let idle = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");
    tick(&mut graph, &mut world);
    graph.drain_dirty();

    tick(&mut graph, &mut world);

    let dirty = graph.drain_dirty();
    assert!(dirty.contains(&source), "a handle names one tick's batch");
    assert!(dirty.contains(&sink), "and so does each node it reaches");
    assert!(!dirty.contains(&idle), "and nothing else");
}

// --- 5.9 cycles --------------------------------------------------------

#[test]
fn a_trigger_connection_in_a_cycle_carries_nothing() {
    let mut world = arena_world();
    let mut graph = Graph::default();
    let a = graph.insert(Node::of(Relay::default()));
    let b = graph.insert(Node::of(Relay::default()));
    graph
        .connect(Port::new(a, "pings"), Port::new(b, "pings"), 0)
        .expect("legal");
    graph
        .connect(Port::new(b, "pings"), Port::new(a, "pings"), 0)
        .expect("legal");
    // Seed `a`'s inlet with a live handle, so there is something a cycle
    // member could wrongly read on the next tick.
    let seeded = world
        .get_non_send::<EventArena>()
        .expect("arena")
        .publish([Ping(7)]);
    set_field(&mut graph, a, "pings", &seeded);

    tick(&mut graph, &mut world);
    tick(&mut graph, &mut world);

    let mut cycles = graph.cycles().to_vec();
    cycles.sort();
    assert_eq!(cycles, vec![a, b], "the tick still evaluates both");
    for node in [a, b] {
        assert!(
            batch_of(&world, &graph, node, "pings").is_empty(),
            "neither reads the other's occurrences: the handle its partner \
             published last tick is stale"
        );
    }
}

// --- 5.10 no arena -----------------------------------------------------

#[test]
fn a_graph_with_no_arena_ticks_and_leaves_handle_outlets_empty() {
    let mut world = bare_world();
    let mut graph = Graph::default();
    let source = graph.insert(Node::of(emitter(3.0)));
    let relay = graph.insert(Node::of(Relay::default()));
    let sink = graph.insert(Node::of(Tally::default()));
    graph
        .connect(Port::new(source, "pings"), Port::new(relay, "pings"), 0)
        .expect("legal");
    graph
        .connect(Port::new(relay, "pings"), Port::new(sink, "pings"), 0)
        .expect("legal");

    tick(&mut graph, &mut world);

    assert!(world.get_non_send::<EventArena>().is_none(), "no arena");
    for node in [source, relay] {
        assert_eq!(
            handle_at(&graph, node, Part::Outlets, "pings"),
            EventHandle::EMPTY,
            "its handle outlets are empty"
        );
    }
    assert_eq!(
        read_field::<f32>(&graph, sink, Part::Outlets, "count"),
        0.0,
        "and the evaluation succeeded"
    );
}

// --- 4.4 the plugin ----------------------------------------------------

/// An `App` with the real schedule: `GraphPlugin` + `EventsPlugin`, fixtures
/// registered, and one update burned so the fixed-time accumulator is warm.
fn plugin_app() -> App {
    let mut app = App::new();
    app.add_plugins(TimePlugin)
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
        .add_plugins((GraphPlugin, EventsPlugin));
    register_fixtures(&mut app);
    app.update();
    app
}

#[test]
fn one_plugin_is_the_whole_mechanism() {
    let app = plugin_app();

    assert!(
        app.world().get_non_send::<EventArena>().is_some(),
        "the host registered no system, set or resource on the crate's behalf"
    );
}

#[test]
fn a_batch_published_on_one_update_is_not_readable_on_the_next() {
    let mut app = plugin_app();
    let source = app
        .world_mut()
        .resource_mut::<Graph>()
        .insert(Node::of(emitter(2.0)));

    app.update();
    let first = {
        let graph = app.world().resource::<Graph>();
        handle_at(graph, source, Part::Outlets, "pings")
    };
    assert_eq!(
        app.world()
            .get_non_send::<EventArena>()
            .expect("arena")
            .read(first)
            .expect("this tick's batch")
            .len(),
        2
    );

    app.update();

    assert!(
        app.world()
            .get_non_send::<EventArena>()
            .expect("arena")
            .read(first)
            .is_none(),
        "the clear really does run before the tick"
    );
}

#[test]
fn the_clear_is_ordered_before_the_graph_tick() {
    // Belt and braces on the ordering above: the set is declared `.before`,
    // and a batch published *by a node* is still readable in the tick that
    // published it, which only holds if the clear ran first.
    let mut app = plugin_app();
    let source = app
        .world_mut()
        .resource_mut::<Graph>()
        .insert(Node::of(emitter(1.0)));
    let sink = app
        .world_mut()
        .resource_mut::<Graph>()
        .insert(Node::of(Tally::default()));
    {
        let mut graph = app.world_mut().resource_mut::<Graph>();
        graph
            .connect(Port::new(source, "pings"), Port::new(sink, "pings"), 0)
            .expect("legal");
    }

    app.update();

    let graph = app.world().resource::<Graph>();
    assert_eq!(read_field::<f32>(graph, sink, Part::Outlets, "count"), 1.0);
    let _ = GraphTickSet;
}

// --- 6. the document round-trip ----------------------------------------

mod document {
    use super::*;
    use sway_document::v4::{StableIds, load, parse, to_document, to_ron};

    /// A graph holding one `Held` node with an authored `gain` and a handle
    /// inlet, plus the ids to save it under.
    fn held_graph(pings: EventHandle<Ping>) -> (Graph, NodeId, StableIds) {
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Held {
            inlets: HeldIn { pings, gain: 0.75 },
            ..Default::default()
        }));
        let mut ids = StableIds::new();
        ids.assign("held".to_string(), node);
        (graph, node, ids)
    }

    #[test]
    fn a_handle_inlet_does_not_stop_a_node_from_saving() {
        let registry = test_registry();
        let arena = EventArena::default();
        let (graph, _, mut ids) = held_graph(arena.publish([Ping(1), Ping(2)]));

        let doc = to_document(&graph, &registry, &mut ids).expect("no diagnostic");

        let entry = &doc.nodes["held"];
        assert_eq!(entry.kind, "Held", "written like every other node");
        let inlets = entry.inlets.get_ron();
        assert!(
            !inlets.contains("generation") && !inlets.contains("slot"),
            "the entry names no batch and no generation: {inlets}"
        );
        assert!(inlets.contains("0.75"), "the authored value is there");
    }

    #[test]
    fn a_handle_inlet_reloads_as_the_empty_handle() {
        let registry = test_registry();
        let arena = EventArena::default();
        let live = arena.publish([Ping(1)]);
        assert_ne!(live, EventHandle::EMPTY, "a handle naming a live batch");
        let (graph, _, mut ids) = held_graph(live);

        let doc = to_document(&graph, &registry, &mut ids).expect("saves");
        let text = to_ron(&doc).expect("emits");
        let reparsed = parse(&text).expect("reparses");
        let (loaded, loaded_ids, diagnostics) = load(&reparsed, &registry);

        assert!(diagnostics.items.is_empty(), "{:?}", diagnostics.items);
        let node = loaded_ids.node_of("held").expect("the node loaded");
        let inlets = read_field::<f32>(&loaded, node, Part::Inlets, "gain");
        assert_eq!(inlets, 0.75, "its other inlets are restored");
        assert_eq!(
            read_field::<EventHandle<Ping>>(&loaded, node, Part::Inlets, "pings"),
            EventHandle::EMPTY,
            "and its handle inlet is the empty handle"
        );
    }

    #[test]
    fn saving_on_two_different_ticks_produces_the_same_bytes() {
        let registry = test_registry();
        let mut arena = EventArena::default();
        let (mut graph, node, mut ids) = held_graph(arena.publish([Ping(1)]));

        let first = to_ron(&to_document(&graph, &registry, &mut ids).expect("saves")).expect("ron");

        // A new tick: the arena is emptied, the producer republishes, and the
        // node's handle inlet names a batch in a different generation.
        arena.clear();
        set_field(
            &mut graph,
            node,
            "pings",
            &arena.publish([Ping(2), Ping(3)]),
        );
        let second =
            to_ron(&to_document(&graph, &registry, &mut ids).expect("saves")).expect("ron");

        assert_eq!(first, second, "byte-stable across ticks");
    }
}
