//! `MidiTime`, the new-model replacement for the wire-model `MidiTime`
//! (`crate::midi_time::MidiTime`).
//!
//! `graph`'s "an external time source is an ordinary node" requirement: this
//! is an ordinary node kind — no injection phase, no MIDI type named by
//! `sway-graph` — that reads the `Transport` snapshot resource straight out
//! of `&World` during its own evaluation (design D4).

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::Transport;

/// [`MidiTime`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct MidiTimeOut {
    pub out: f32,
}

/// Publishes the current MIDI transport's PPQ position.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct MidiTime {
    pub inlets: (),
    pub state: (),
    pub outlets: MidiTimeOut,
}

impl NodeKind for MidiTime {
    fn evaluate(&mut self, world: &World) {
        let ppq = world.get_resource::<Transport>().map_or(0.0, |t| t.ppq);
        self.outlets.out = ppq as f32;
    }
}

#[cfg(test)]
mod tests {

    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing::{tick_once as tick, trace_world};
    use sway_graph::graph::{Graph, Node, Part, path};

    use super::*;

    fn world_with_transport(transport: Transport) -> World {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MidiTime>(&mut registry);
        let mut world = trace_world(registry);
        world.insert_resource(transport);
        world
    }

    #[test]
    fn midi_time_publishes_the_transports_ppq() {
        let world = world_with_transport(Transport {
            ppq: 3.25,
            ..Default::default()
        });
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(MidiTime::default()));

        tick(&mut graph, &world);

        let out = path::resolve(graph.get(node).unwrap(), Part::Outlets, "out")
            .and_then(|v| v.try_downcast_ref::<f32>().copied())
            .unwrap();
        assert!((out - 3.25).abs() < 1e-5, "out={out}");
    }

    #[test]
    fn a_missing_transport_resource_reads_as_zero() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MidiTime>(&mut registry);
        // Deliberately no `Transport`.
        let world = trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(MidiTime::default()));

        tick(&mut graph, &world);

        let out = path::resolve(graph.get(node).unwrap(), Part::Outlets, "out")
            .and_then(|v| v.try_downcast_ref::<f32>().copied())
            .unwrap();
        assert_eq!(out, 0.0);
    }

    #[test]
    fn the_graph_is_not_reachable_from_a_midi_times_world() {
        // `graph`: a node cannot reach the graph through `&World`.
        let world = world_with_transport(Transport::default());
        debug_assert!(world.get_resource::<Graph>().is_none());
    }
}
