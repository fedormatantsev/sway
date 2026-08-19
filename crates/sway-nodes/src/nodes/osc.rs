//! `Oscillator`, the new-model port of the wire-model `Oscillator`
//! (`crate::osc::Oscillator`).
//!
//! This is a faithful port, not a redesign: the old node's evaluation reads
//! `time` (typically driven by `MidiTime`, directly or through a chain) and
//! `period` rather than a self-accumulating clock, because the demo document
//! locks several oscillators to the shared MIDI transport
//! (`midiTime -> lfoA`, `midiTime -> lfoB`, `midiTime -> spriteOsc`,
//! `midiTime -> spriteOsc2`) and needs them to stay in a fixed phase
//! relationship to each other and to the transport, not drift independently.
//! `TimeFrom` / `AmplitudeFrom` do not port — an edge now names `"time"` /
//! `"amplitude"` directly on `inlets`. The node carries no state and reads
//! no `&World`, exactly as the wire-model version did.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::lfo::{Waveform, wave};

/// [`Oscillator`]'s inlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct OscillatorIn {
    pub time: f32,
    pub period: f32,
    pub shape: Waveform,
    pub phase: f32,
    pub amplitude: f32,
}

impl Default for OscillatorIn {
    fn default() -> Self {
        Self {
            time: 0.0,
            period: 4.0,
            shape: Waveform::Sine,
            phase: 0.0,
            amplitude: 1.0,
        }
    }
}

/// [`Oscillator`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct OscillatorOut {
    pub out: f32,
}

/// A generic time-driven oscillator: time, period, shape, a phase offset in
/// cycles, and amplitude.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Oscillator {
    pub inlets: OscillatorIn,
    pub state: (),
    pub outlets: OscillatorOut,
}

impl NodeKind for Oscillator {
    fn evaluate(&mut self, _world: &World) {
        // An authored zero or negative period holds still rather than
        // dividing: the node is infallible.
        let p = if self.inlets.period > 0.0 {
            (self.inlets.time as f64 / self.inlets.period as f64 + self.inlets.phase as f64)
                .rem_euclid(1.0) as f32
        } else {
            self.inlets.phase.rem_euclid(1.0)
        };
        self.outlets.out = wave(self.inlets.shape, p) * self.inlets.amplitude;
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part, Port};

    use super::*;
    use crate::nodes::harness;
    use crate::nodes::math::{Math, MathIn};

    #[test]
    fn oscillator_at_phase_quarter_is_one_with_no_midi() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Oscillator {
                inlets: OscillatorIn {
                    time: 0.0,
                    period: 4.0,
                    shape: Waveform::Sine,
                    phase: 0.25,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out"), 1.0);
    }

    #[test]
    fn a_zero_period_holds_still_rather_than_dividing() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Oscillator {
                inlets: OscillatorIn {
                    time: 5.0,
                    period: 0.0,
                    shape: Waveform::Sine,
                    phase: 0.25,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out"), 1.0);
    }

    /// Pins the demo document's `midiTime -> lfoA` / `midiTime -> spriteOsc`
    /// shape: an upstream node's `out` reaching `Oscillator.time` and
    /// resolving through to `Oscillator.out` in the SAME tick, which is what
    /// lets an oscillator stay locked to the shared transport instead of
    /// drifting on its own clock.
    #[test]
    fn a_driven_time_reaches_the_oscillator_output_in_one_tick() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        register_node_kind::<Math>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        // Stands in for `MidiTime`: any node whose `out` reaches `time`.
        let time_source = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: crate::math::MathOp::Add,
                    a: 1.0,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Oscillator {
                inlets: OscillatorIn {
                    time: 0.0,
                    period: 4.0,
                    shape: Waveform::Sine,
                    phase: 0.0,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));
        graph
            .connect(Port::new(time_source, "out"), Port::new(node, "time"), 0)
            .expect("legal");

        harness::tick(&mut graph, &world);

        assert_eq!(
            harness::read_f32(&graph, node, Part::Outlets, "out"),
            1.0,
            "a quarter-cycle time driven in from an upstream node must reach \
             the output in ONE tick"
        );
    }

    #[test]
    fn a_driven_amplitude_reaches_the_output_in_one_tick() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        register_node_kind::<Math>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let amplitude = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: crate::math::MathOp::Add,
                    a: 0.5,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Oscillator {
                inlets: OscillatorIn {
                    time: 0.0,
                    period: 4.0,
                    shape: Waveform::Sine,
                    phase: 0.25,
                    amplitude: 0.0,
                },
                ..Default::default()
            },
        ));
        graph
            .connect(Port::new(amplitude, "out"), Port::new(node, "amplitude"), 0)
            .expect("legal");

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out"), 0.5);
    }

    #[test]
    fn an_equal_write_does_not_dirty_the_node() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Vec2::ZERO, Oscillator::default()));

        harness::tick(&mut graph, &world);
        graph.drain_dirty();
        harness::tick(&mut graph, &world);

        assert!(
            !graph.is_dirty(node),
            "unchanged inlets must hold a steady output, dirtying nothing"
        );
    }
}
