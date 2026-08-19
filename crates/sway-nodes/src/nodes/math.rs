//! `Math` and `Remap`, the new-model replacements for the wire-model types of
//! the same name in `crate::value`. The arithmetic itself is shared, unported
//! code: `crate::math::{math_value, remap_value}`.
//!
//! `MathAFrom` / `RemapInputFrom` do not port — an edge now names `"a"` /
//! `"input"` directly on `inlets`.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::math::{MathOp, math_value, remap_value};

/// [`Math`]'s inlets. `b` is authorable and a wire may still override it —
/// "LFO x 2" is one `Math` with `b: 2.0` left unwired, which is why there is
/// no separate `Const` kind.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct MathIn {
    pub op: MathOp,
    pub a: f32,
    pub b: f32,
}

/// [`Math`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct MathOut {
    pub out: f32,
}

/// Binary arithmetic.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Math {
    pub inlets: MathIn,
    pub state: (),
    pub outlets: MathOut,
}

impl NodeKind for Math {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = math_value(self.inlets.op, self.inlets.a, self.inlets.b);
    }
}

/// [`Remap`]'s inlets. `input` is a field rather than an implicit inlet so a
/// wire has something to write, exactly as `Math.a` does.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct RemapIn {
    pub input: f32,
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub clamp: bool,
}

impl Default for RemapIn {
    fn default() -> Self {
        Self {
            input: 0.0,
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            clamp: false,
        }
    }
}

/// [`Remap`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct RemapOut {
    pub out: f32,
}

/// Rescales `input` from one range to another.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Remap {
    pub inlets: RemapIn,
    pub state: (),
    pub outlets: RemapOut,
}

impl NodeKind for Remap {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = remap_value(
            self.inlets.input,
            self.inlets.in_min,
            self.inlets.in_max,
            self.inlets.out_min,
            self.inlets.out_max,
            self.inlets.clamp,
        );
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

    #[test]
    fn math_computes_from_its_authored_and_driven_inlets() {
        // "LFO x 2" is one Math with b left unwired.
        let mut registry = TypeRegistry::new();
        register_node_kind::<Math>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.0,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        harness::set_field(&mut graph, source, "a", &3.0f32);
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: MathOp::Mul,
                    a: 0.0,
                    b: 2.0,
                },
                ..Default::default()
            },
        ));
        graph
            .connect(Port::new(source, "out"), Port::new(node, "a"), 0)
            .expect("legal");

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out"), 6.0);
    }

    #[test]
    fn remap_rescales_its_driven_input() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Math>(&mut registry);
        register_node_kind::<Remap>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.5,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let node = graph.insert(Node::of(
            Vec2::ZERO,
            Remap {
                inlets: RemapIn {
                    input: 0.0,
                    in_min: 0.0,
                    in_max: 1.0,
                    out_min: 0.0,
                    out_max: 10.0,
                    clamp: true,
                },
                ..Default::default()
            },
        ));
        graph
            .connect(Port::new(source, "out"), Port::new(node, "input"), 0)
            .expect("legal");

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out"), 5.0);
    }

    #[test]
    fn a_lfo_math_remap_chain_matches_the_shared_pure_functions() {
        // Ports `chain-math-remap.in.ron` (tick_hz 120, 31 ticks) onto the new
        // node shapes: Lfo(1.0 Hz) -> Math(Add, b=1.0) -> Remap(0..2 -> -1..1,
        // clamped). Same math as the golden trace, driven through the real
        // graph tick instead of called directly.
        let mut registry = TypeRegistry::new();
        register_node_kind::<crate::nodes::lfo::Lfo>(&mut registry);
        register_node_kind::<Math>(&mut registry);
        register_node_kind::<Remap>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();

        let lfo = graph.insert(Node::of(
            Vec2::ZERO,
            crate::nodes::lfo::Lfo {
                inlets: crate::nodes::lfo::LfoIn {
                    frequency: 1.0,
                    shape: crate::lfo::Waveform::Sine,
                    phase: 0.0,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));
        let math = graph.insert(Node::of(
            Vec2::ZERO,
            Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.0,
                    b: 1.0,
                },
                ..Default::default()
            },
        ));
        let remap = graph.insert(Node::of(
            Vec2::ZERO,
            Remap {
                inlets: RemapIn {
                    input: 0.0,
                    in_min: 0.0,
                    in_max: 2.0,
                    out_min: -1.0,
                    out_max: 1.0,
                    clamp: true,
                },
                ..Default::default()
            },
        ));
        graph
            .connect(Port::new(lfo, "out"), Port::new(math, "a"), 0)
            .expect("legal");
        graph
            .connect(Port::new(math, "out"), Port::new(remap, "input"), 0)
            .expect("legal");

        let dt = 1.0 / harness::TICK_HZ;
        let mut expected_time = 0.0_f64;
        for tick in 0..31 {
            harness::tick(&mut graph, &world);

            let expected_lfo =
                crate::lfo::lfo_value(1.0, crate::lfo::Waveform::Sine, 0.0, 1.0, expected_time);
            let expected_math = math_value(MathOp::Add, expected_lfo, 1.0);
            let expected_remap = remap_value(expected_math, 0.0, 2.0, -1.0, 1.0, true);

            let actual_lfo = harness::read_f32(&graph, lfo, Part::Outlets, "out");
            let actual_math = harness::read_f32(&graph, math, Part::Outlets, "out");
            let actual_remap = harness::read_f32(&graph, remap, Part::Outlets, "out");

            assert!(
                (actual_lfo - expected_lfo).abs() < 1e-5,
                "tick {tick}: lfo actual={actual_lfo} expected={expected_lfo}"
            );
            assert!(
                (actual_math - expected_math).abs() < 1e-5,
                "tick {tick}: math actual={actual_math} expected={expected_math}"
            );
            assert!(
                (actual_remap - expected_remap).abs() < 1e-5,
                "tick {tick}: remap actual={actual_remap} expected={expected_remap}"
            );

            expected_time += dt;
        }
    }

    #[test]
    fn the_math_and_remap_inlets_never_write_an_equal_value() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Math>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Vec2::ZERO, Math::default()));

        harness::tick(&mut graph, &world);
        graph.drain_dirty();
        harness::tick(&mut graph, &world);

        assert!(!graph.is_dirty(node));
    }
}
