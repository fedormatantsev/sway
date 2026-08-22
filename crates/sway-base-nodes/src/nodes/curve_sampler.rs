//! `CurveSampler`: a piecewise curve sampled at a time inlet.
//!
//! Time is clamped to the keys' x-range — it does not wrap. An envelope is
//! typically [`Timer`] into this node. Looping waves are not this node's job.
//!
//! [`Timer`]: crate::Timer

use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{ReflectDeserialize, ReflectSerialize};
use serde::{Deserialize, Serialize};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

/// Authored piecewise keys. Opaque so the engine does not treat them as a
/// variadic inlet (any `Vec` under inlets is truncated to the edge count).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(transparent)]
#[reflect(opaque)]
#[reflect(Default, Serialize, Deserialize)]
pub struct CurveKeys(pub Vec<Vec2>);

/// Samples the authored keys at `time`. Empty keys yield 0. Time is clamped
/// to the keys' x-range, then linearly interpolated.
pub fn curve_sampler_value(time: f32, keys: &[Vec2]) -> f32 {
    sample_piecewise(keys, time)
}

fn sample_piecewise(keys: &[Vec2], t: f32) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    let mut sorted = keys.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    let min_x = sorted[0].x;
    let max_x = sorted.last().expect("non-empty").x;
    let t = t.clamp(min_x, max_x);
    if t <= min_x {
        return sorted[0].y;
    }
    if t >= max_x {
        return sorted.last().expect("non-empty").y;
    }
    for pair in sorted.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t >= a.x && t <= b.x {
            if b.x == a.x {
                return b.y;
            }
            let u = (t - a.x) / (b.x - a.x);
            return a.y + u * (b.y - a.y);
        }
    }
    sorted.last().expect("non-empty").y
}

/// [`CurveSampler`]'s inlets.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
pub struct CurveSamplerIn {
    pub time: f32,
    pub keys: CurveKeys,
}

/// [`CurveSampler`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct CurveSamplerOut {
    pub out: f32,
}

/// Samples an authored piecewise curve at `time`.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct CurveSampler {
    pub inlets: CurveSamplerIn,
    pub state: (),
    pub outlets: CurveSamplerOut,
}

impl NodeKind for CurveSampler {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = curve_sampler_value(self.inlets.time, &self.inlets.keys.0);
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing;
    use sway_graph::graph::{Graph, Node, Part, Port};

    use super::*;
    use crate::nodes::math::{Math, MathIn};

    fn sampler_with(time: f32, keys: Vec<Vec2>) -> CurveSampler {
        CurveSampler {
            inlets: CurveSamplerIn {
                time,
                keys: CurveKeys(keys),
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_piecewise_envelope_is_sampled_and_clamped() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<CurveSampler>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(sampler_with(
            0.5,
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)],
        )));

        testing::tick_once(&mut graph, &world);
        assert!(
            (testing::read_field::<f32>(&graph, node, Part::Outlets, "out") - 0.5).abs() < 1e-6
        );

        testing::set_field(&mut graph, node, "time", &2.0_f32);
        testing::tick_once(&mut graph, &world);
        assert_eq!(
            testing::read_field::<f32>(&graph, node, Part::Outlets, "out"),
            1.0,
            "time past the last key holds that key"
        );

        testing::set_field(&mut graph, node, "time", &(-1.0_f32));
        testing::tick_once(&mut graph, &world);
        assert_eq!(
            testing::read_field::<f32>(&graph, node, Part::Outlets, "out"),
            0.0,
            "time before the first key holds that key"
        );
    }

    #[test]
    fn a_driven_time_reaches_the_outlet_in_one_tick() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<CurveSampler>(&mut registry);
        register_node_kind::<Math>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let time_source = graph.insert(Node::of(Math {
            inlets: MathIn {
                op: crate::nodes::math::MathOp::Add,
                a: 0.5,
                b: 0.0,
            },
            ..Default::default()
        }));
        let node = graph.insert(Node::of(sampler_with(
            0.0,
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)],
        )));
        graph
            .connect(Port::new(time_source, "out"), Port::new(node, "time"), 0)
            .expect("legal");

        testing::tick_once(&mut graph, &world);

        assert!(
            (testing::read_field::<f32>(&graph, node, Part::Outlets, "out") - 0.5).abs() < 1e-6,
            "a driven time must reach the output in ONE tick"
        );
    }

    #[test]
    fn an_empty_piecewise_curve_is_zero() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<CurveSampler>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(CurveSampler::default()));

        testing::tick_once(&mut graph, &world);

        assert_eq!(
            testing::read_field::<f32>(&graph, node, Part::Outlets, "out"),
            0.0
        );
    }

    #[test]
    fn an_equal_write_does_not_dirty_the_node() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<CurveSampler>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(sampler_with(
            0.0,
            vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 1.0)],
        )));

        testing::tick_once(&mut graph, &world);
        graph.drain_dirty();
        testing::tick_once(&mut graph, &world);

        assert!(
            !graph.is_dirty(node),
            "unchanged inlets must hold a steady output, dirtying nothing"
        );
    }
}
