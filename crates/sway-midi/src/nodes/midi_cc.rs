//! `MidiCc`: a held MIDI Control Change parameter.
//!
//! A close sibling of [`MidiTime`](crate::MidiTime) — an ordinary node kind
//! that reads a plugin-owned snapshot out of `&World` during its own
//! evaluation, with no injection phase and no MIDI type named by `sway-graph`.
//! Here the snapshot is [`MidiControls`], filled on the drain path before the
//! graph ticks, which is why two nodes on the same controller agree and a node
//! created after a fader moved still publishes that position (design D1).

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::MidiControls;

/// The largest 7-bit MIDI value, as the divisor that maps it to 1.0.
const MIDI_MAX: f32 = 127.0;

/// [`MidiCc`]'s inlets.
///
/// `f32` rather than `u8` so they are both inspector-editable and
/// connectable: connect legality is exact-type, so a `u8` channel would be a
/// knob nothing in the graph could drive (design D3).
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MidiCcIn {
    /// The MIDI channel in the protocol's own 0–15 numbering, not display
    /// 1–16 (design D4). Truncated toward zero and clamped when read.
    pub channel: f32,
    /// The controller number, 0–127. Truncated toward zero and clamped when
    /// read.
    pub cc: f32,
}

impl Default for MidiCcIn {
    fn default() -> Self {
        // Not derived: a derived `Default` would address cc 0, and cc 1 (the
        // mod wheel) is the one a controller is most likely to send.
        Self {
            channel: 0.0,
            cc: 1.0,
        }
    }
}

/// [`MidiCc`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MidiCcOut {
    pub out: f32,
}

/// Publishes the last Control Change on the authored channel and controller
/// number as a held 0–1 parameter, so a fader wires straight into anything
/// that takes an `f32` (design D2).
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct MidiCc {
    pub inlets: MidiCcIn,
    pub state: (),
    pub outlets: MidiCcOut,
}

impl NodeKind for MidiCc {
    fn evaluate(&mut self, world: &World) {
        // No MIDI at all is a zero outlet, not a failed evaluation.
        let raw = world.get_resource::<MidiControls>().map_or(0, |controls| {
            controls.get(self.inlets.channel, self.inlets.cc)
        });
        self.outlets.out = f32::from(raw) / MIDI_MAX;
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing::{read_field, tick_once as tick};
    use sway_graph::graph::{Graph, Node, Part};

    use super::*;

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MidiCc>(&mut registry);
        registry
    }

    fn world_with_controls(controls: MidiControls) -> World {
        let mut world = sway_graph::graph::testing::trace_world(registry());
        world.insert_resource(controls);
        world
    }

    fn node_of(channel: f32, cc: f32) -> MidiCc {
        MidiCc {
            inlets: MidiCcIn { channel, cc },
            ..Default::default()
        }
    }

    fn out_of(graph: &Graph, node: sway_graph::graph::NodeId) -> f32 {
        read_field::<f32>(graph, node, Part::Outlets, "out")
    }

    #[test]
    fn a_matching_full_value_publishes_one() {
        let mut controls = MidiControls::default();
        controls.set(0, 1, 127);
        let world = world_with_controls(controls);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, node), 1.0);
    }

    #[test]
    fn a_mid_value_publishes_that_fraction_of_127() {
        let mut controls = MidiControls::default();
        controls.set(0, 1, 64);
        let world = world_with_controls(controls);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        let out = out_of(&graph, node);
        assert!((out - 64.0 / 127.0).abs() < 1e-6, "out={out}");
    }

    #[test]
    fn nothing_received_yet_is_zero() {
        let world = world_with_controls(MidiControls::default());
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, node), 0.0);
    }

    #[test]
    fn an_unmatched_control_leaves_the_outlet_at_zero() {
        let mut controls = MidiControls::default();
        controls.set(2, 3, 127);
        let world = world_with_controls(controls);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, node), 0.0);
    }

    #[test]
    fn a_missing_snapshot_resource_reads_as_zero() {
        // Deliberately no `MidiControls`: no MIDI input must not fail
        // evaluation.
        let world = sway_graph::graph::testing::trace_world(registry());
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, node), 0.0);
    }

    #[test]
    fn out_of_range_inlets_address_the_last_channel_and_controller() {
        let mut controls = MidiControls::default();
        controls.set(15, 127, 127);
        let world = world_with_controls(controls);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(node_of(20.0, 200.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, node), 1.0);
    }

    #[test]
    fn two_nodes_on_the_same_controller_agree() {
        let mut controls = MidiControls::default();
        controls.set(0, 1, 100);
        let world = world_with_controls(controls);
        let mut graph = Graph::default();
        let first = graph.insert(Node::of(node_of(0.0, 1.0)));
        let second = graph.insert(Node::of(node_of(0.0, 1.0)));

        tick(&mut graph, &world);

        assert_eq!(out_of(&graph, first), out_of(&graph, second));
        assert!((out_of(&graph, first) - 100.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn the_default_inlets_address_the_mod_wheel_on_channel_zero() {
        assert_eq!(
            MidiCcIn::default(),
            MidiCcIn {
                channel: 0.0,
                cc: 1.0
            }
        );
    }
}
