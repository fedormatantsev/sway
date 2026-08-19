//! `Lfo`, a new node kind wrapping the wire-model's pure `crate::lfo::lfo_value`
//! (previously exercised only via `tests/traces.rs`, never wired into the old
//! entity/component graph as a `Behaviour`). The elapsed time `lfo_value`
//! reads as its `time` parameter is exactly the kind of memory tasks 4.2 asks
//! to move into the `state` part: it is accumulated tick over tick from
//! `Time<Fixed>`'s `dt`, so the node needs no externally driven time inlet.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::lfo::{Waveform, lfo_value};

/// [`Lfo`]'s inlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct LfoIn {
    /// Cycles per second.
    pub frequency: f32,
    pub shape: Waveform,
    /// An authored phase offset, in cycles.
    pub phase: f32,
    pub amplitude: f32,
}

impl Default for LfoIn {
    fn default() -> Self {
        Self {
            frequency: 1.0,
            shape: Waveform::Sine,
            phase: 0.0,
            amplitude: 1.0,
        }
    }
}

/// [`Lfo`]'s state: elapsed time, in seconds, accumulated from `Time<Fixed>`.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct LfoState {
    pub elapsed: f64,
}

/// [`Lfo`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct LfoOut {
    pub out: f32,
}

/// A free-running low-frequency oscillator, driven by the fixed tick's own
/// clock rather than an externally wired time value.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Lfo {
    pub inlets: LfoIn,
    pub state: LfoState,
    pub outlets: LfoOut,
}

impl NodeKind for Lfo {
    fn evaluate(&mut self, world: &World) {
        let dt = world
            .get_resource::<Time<Fixed>>()
            .map_or(0.0, |time| time.delta_secs());
        self.outlets.out = lfo_value(
            self.inlets.frequency,
            self.inlets.shape,
            self.inlets.phase,
            self.inlets.amplitude,
            self.state.elapsed,
        );
        self.state.elapsed += dt as f64;
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part};

    use super::*;
    use crate::nodes::harness;

    /// Ports `tests/traces/lfo-one-cycle.in.ron` (`tick_hz: 120`,
    /// `ticks: 61`, 2 Hz sine) onto the new node shape: the same
    /// `crate::lfo::lfo_value` the golden trace calls directly, now reached
    /// through a real graph tick with the elapsed time held in `state`
    /// instead of threaded through the test harness by hand.
    #[test]
    fn lfo_reproduces_lfo_value_over_one_cycle() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Lfo>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Lfo {
                inlets: LfoIn {
                    frequency: 2.0,
                    shape: Waveform::Sine,
                    phase: 0.0,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));

        let dt = 1.0 / harness::TICK_HZ;
        let mut expected_elapsed = 0.0_f64;
        for tick in 0..61 {
            harness::tick(&mut graph, &world);

            let actual = harness::read_f32(&graph, node, Part::Outlets, "out");
            let expected = lfo_value(2.0, Waveform::Sine, 0.0, 1.0, expected_elapsed);
            assert!(
                (actual - expected).abs() < 1e-5,
                "tick {tick}: actual={actual} expected={expected}"
            );

            expected_elapsed += dt;
        }
    }

    #[test]
    fn waveforms_are_bipolar_and_amplitude_scaled() {
        for (shape, expected) in [
            (Waveform::Sine, 0.5),
            (Waveform::Triangle, 0.0),
            (Waveform::Saw, -0.25),
            (Waveform::Square, 0.5),
        ] {
            let mut registry = TypeRegistry::new();
            register_node_kind::<Lfo>(&mut registry);
            let world = harness::trace_world(registry);
            let mut graph = Graph::default();
            let node = graph.insert(Node::of(
                Vec2::ZERO,
                Lfo {
                    inlets: LfoIn {
                        frequency: 0.0,
                        shape,
                        phase: 0.25,
                        amplitude: 0.5,
                    },
                    ..Default::default()
                },
            ));

            harness::tick(&mut graph, &world);

            let actual = harness::read_f32(&graph, node, Part::Outlets, "out");
            assert!(
                (actual - expected).abs() < 1e-6,
                "{shape:?}: actual={actual} expected={expected}"
            );
        }
    }
}
