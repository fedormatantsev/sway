//! `Oscillator`, the new-model port of the wire-model `Oscillator`
//! (`crate::osc::Oscillator`).
//!
//! This is a faithful port, not a redesign: the old node's evaluation reads
//! `time` (typically driven by `MidiTime`, directly or through a chain) and
//! `period` rather than a self-accumulating clock. This lets several
//! oscillators share a single transport-locked time source and stay in a fixed
//! phase relationship to each other and to the transport rather than drifting
//! independently.
//! `TimeFrom` / `AmplitudeFrom` do not port — an edge now names `"time"` /
//! `"amplitude"` directly on `inlets`. The node carries no state and reads
//! no `&World`, exactly as the wire-model version did.

use core::f32::consts::TAU;

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
}

pub fn oscillator_value(
    period: f32,
    shape: Waveform,
    phase: f32,
    amplitude: f32,
    time: f64,
) -> f32 {
    let p = if period > 0.0 {
        (time / period as f64 + phase as f64).rem_euclid(1.0) as f32
    } else {
        phase.rem_euclid(1.0)
    };
    wave(shape, p) * amplitude
}

pub(crate) fn wave(shape: Waveform, phase: f32) -> f32 {
    match shape {
        Waveform::Sine => (phase * TAU).sin(),
        Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        Waveform::Saw => 2.0 * phase - 1.0,
        Waveform::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

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

    /// Verifies the `midiTime -> oscillator` wiring shape: an upstream node's
    /// `out` reaching `Oscillator.time` and resolving through to
    /// `Oscillator.out` in the SAME tick, which is what lets an oscillator
    /// stay locked to the shared transport instead of drifting on its own
    /// clock.
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
    fn dropped_samples_do_not_change_absolute_time_output() {
        let period = 1.0 / 2.25;
        let direct = oscillator_value(period, Waveform::Triangle, 0.17, 0.8, 99.0 / 120.0);
        let after_gap = oscillator_value(period, Waveform::Triangle, 0.17, 0.8, 99.0 / 120.0);
        assert_eq!(direct, after_gap);
    }

    #[test]
    fn waveforms_are_bipolar_and_amplitude_scaled() {
        for (shape, expected) in [
            (Waveform::Sine, 0.5),
            (Waveform::Triangle, 0.0),
            (Waveform::Saw, -0.25),
            (Waveform::Square, 0.5),
        ] {
            assert!((oscillator_value(0.0, shape, 0.25, 0.5, 0.0) - expected).abs() < 1e-6);
        }
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
