//! `Math` and `Remap`: binary arithmetic and range rescaling, each a pure
//! function of its own inlets.
//!
//! An edge names `"a"` / `"input"` directly on `inlets`; there are no
//! per-field wire types.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

/// Which operation [`Math`] applies.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    #[default]
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

/// The arithmetic itself. Division by zero yields `0.0` rather than an
/// infinity: a node's outlet feeds a transform, and a NaN there propagates
/// through the whole scene.
pub fn math_value(op: MathOp, a: f32, b: f32) -> f32 {
    match op {
        MathOp::Add => a + b,
        MathOp::Sub => a - b,
        MathOp::Mul => a * b,
        MathOp::Div if b == 0.0 => 0.0,
        MathOp::Div => a / b,
        MathOp::Min => a.min(b),
        MathOp::Max => a.max(b),
    }
}

/// Rescales `value` from one range to another, extrapolating unless clamped.
/// A degenerate input range yields `out_min` rather than a division by zero.
pub fn remap_value(
    mut value: f32,
    in_min: f32,
    in_max: f32,
    out_min: f32,
    out_max: f32,
    clamp: bool,
) -> f32 {
    if clamp {
        value = value.clamp(in_min.min(in_max), in_min.max(in_max));
    }
    if in_min == in_max {
        out_min
    } else {
        out_min + (value - in_min) / (in_max - in_min) * (out_max - out_min)
    }
}

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
    // --- the arithmetic, directly -----------------------------------------

    #[test]
    fn math_supports_every_operation_and_zero_division() {
        use super::{MathOp, math_value};
        for (op, expected) in [
            (MathOp::Add, 8.0),
            (MathOp::Sub, 4.0),
            (MathOp::Mul, 12.0),
            (MathOp::Div, 3.0),
            (MathOp::Min, 2.0),
            (MathOp::Max, 6.0),
        ] {
            assert_eq!(math_value(op, 6.0, 2.0), expected);
        }
        assert_eq!(math_value(MathOp::Div, 6.0, 0.0), 0.0);
    }

    #[test]
    fn remap_can_extrapolate_clamp_and_handle_degenerate_ranges() {
        use super::remap_value;
        assert_eq!(remap_value(15.0, 0.0, 10.0, -1.0, 1.0, false), 2.0);
        assert_eq!(remap_value(15.0, 0.0, 10.0, -1.0, 1.0, true), 1.0);
        assert_eq!(remap_value(4.0, 2.0, 2.0, 7.0, 9.0, false), 7.0);
    }

    #[test]
    fn the_math_enum_defaults_to_its_first_variant() {
        use super::MathOp;
        assert_eq!(MathOp::default(), MathOp::Add);
    }

    
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part, Port};

    use super::*;
    use sway_graph::graph::testing;

    #[test]
    fn math_computes_from_its_authored_and_driven_inlets() {
        // "LFO x 2" is one Math with b left unwired.
        let mut registry = TypeRegistry::new();
        register_node_kind::<Math>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.0,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        testing::set_field(&mut graph, source, "a", &3.0f32);
        let node = graph.insert(Node::of(Math {
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

        testing::tick_once(&mut graph, &world);

        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out"), 6.0);
    }

    #[test]
    fn remap_rescales_its_driven_input() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Math>(&mut registry);
        register_node_kind::<Remap>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.5,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let node = graph.insert(Node::of(Remap {
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

        testing::tick_once(&mut graph, &world);

        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out"), 5.0);
    }

    #[test]
    fn an_oscillator_math_remap_chain_matches_the_shared_pure_functions() {
        // Oscillator(1.0 Hz, Sine) -> Math(Add, b=1.0) -> Remap(0..2 -> -1..1,
        // clamped). The Oscillator's `time` inlet is driven each tick by
        // `set_field` to simulate a clock source, matching the pure-function
        // reference from `chain-math-remap.in.ron`.
        use crate::nodes::osc::{Oscillator, OscillatorIn};
        use crate::nodes::osc::{Waveform, oscillator_value};

        let mut registry = TypeRegistry::new();
        register_node_kind::<Oscillator>(&mut registry);
        register_node_kind::<Math>(&mut registry);
        register_node_kind::<Remap>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();

        let osc = graph.insert(Node::of(Oscillator {
                inlets: OscillatorIn {
                    time: 0.0,
                    period: 1.0,
                    shape: Waveform::Sine,
                    phase: 0.0,
                    amplitude: 1.0,
                },
                ..Default::default()
            },
        ));
        let math = graph.insert(Node::of(Math {
                inlets: MathIn {
                    op: MathOp::Add,
                    a: 0.0,
                    b: 1.0,
                },
                ..Default::default()
            },
        ));
        let remap = graph.insert(Node::of(Remap {
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
            .connect(Port::new(osc, "out"), Port::new(math, "a"), 0)
            .expect("legal");
        graph
            .connect(Port::new(math, "out"), Port::new(remap, "input"), 0)
            .expect("legal");

        let dt = 1.0 / testing::TICK_HZ;
        let mut expected_time = 0.0_f64;
        for tick in 0..31 {
            testing::set_field(&mut graph, osc, "time", &(expected_time as f32));
            testing::tick_once(&mut graph, &world);

            let expected_osc = oscillator_value(1.0, Waveform::Sine, 0.0, 1.0, expected_time);
            let expected_math = math_value(MathOp::Add, expected_osc, 1.0);
            let expected_remap = remap_value(expected_math, 0.0, 2.0, -1.0, 1.0, true);

            let actual_osc = testing::read_field::<f32>(&graph, osc, Part::Outlets, "out");
            let actual_math = testing::read_field::<f32>(&graph, math, Part::Outlets, "out");
            let actual_remap = testing::read_field::<f32>(&graph, remap, Part::Outlets, "out");

            assert!(
                (actual_osc - expected_osc).abs() < 1e-5,
                "tick {tick}: osc actual={actual_osc} expected={expected_osc}"
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
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Math::default()));

        testing::tick_once(&mut graph, &world);
        graph.drain_dirty();
        testing::tick_once(&mut graph, &world);

        assert!(!graph.is_dirty(node));
    }
}
