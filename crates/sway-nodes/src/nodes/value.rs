//! `Vec3`, the new-model replacement for the wire-model `Vec3Value`
//! (`crate::value::Vec3Value`). See `crates/sway-nodes/src/value.rs`.
//!
//! The per-axis wires `Vec3XFrom` / `Vec3YFrom` / `Vec3ZFrom` do not port —
//! an edge now names `"x"` / `"y"` / `"z"` directly on `inlets`.

use bevy::math::Vec3 as MathVec3;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

/// [`Vec3`]'s inlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct Vec3In {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// [`Vec3`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct Vec3Out {
    pub out: MathVec3,
}

/// A vector literal whose components are driveable. Registered under the
/// short kind name `"Vec3"`.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Vec3 {
    pub inlets: Vec3In,
    pub state: (),
    pub outlets: Vec3Out,
}

impl NodeKind for Vec3 {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = MathVec3::new(self.inlets.x, self.inlets.y, self.inlets.z);
    }
}

#[cfg(test)]
mod tests {
    
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part, Port};

    use super::*;
    use crate::nodes::harness;

    #[test]
    fn a_vec3_node_publishes_its_three_fields() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Vec3>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Vec3 {
                inlets: Vec3In {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                ..Default::default()
            },
        ));

        harness::tick(&mut graph, &world);

        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out.x"), 1.0);
        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out.y"), 2.0);
        assert_eq!(harness::read_f32(&graph, node, Part::Outlets, "out.z"), 3.0);
    }

    #[test]
    fn a_float_reaches_a_vec3_axis_in_one_tick() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Vec3>(&mut registry);
        register_node_kind::<crate::nodes::math::Math>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(crate::nodes::math::Math {
                inlets: crate::nodes::math::MathIn {
                    op: crate::math::MathOp::Add,
                    a: 0.75,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let vector = graph.insert(Node::of(Vec3::default()));
        graph
            .connect(Port::new(source, "out"), Port::new(vector, "y"), 0)
            .expect("legal");

        harness::tick(&mut graph, &world);

        assert_eq!(
            harness::read_f32(&graph, vector, Part::Outlets, "out.x"),
            0.0
        );
        assert_eq!(
            harness::read_f32(&graph, vector, Part::Outlets, "out.y"),
            0.75,
            "the inlet must land before the node evaluates, in ONE tick"
        );
        assert_eq!(
            harness::read_f32(&graph, vector, Part::Outlets, "out.z"),
            0.0
        );
    }

    // Regression guard: `graph::tick::evaluate`'s dirty check compares a
    // node's current concrete value against a `to_dynamic()` snapshot
    // (`crates/sway-graph/src/graph/tick.rs`), and this outlet's `Vec3` field
    // is exactly the shape that previously tripped an asymmetry in glam's
    // generated `PartialReflect::reflect_partial_eq` (concrete-vs-dynamic
    // compared `Some(false)` even when equal, dynamic-vs-concrete compared
    // `Some(true)`). `reflect_equal` in `sway-graph`'s `graph::model` now
    // checks both argument orders, so an equal `Vec3` write must no longer
    // dirty the node — pinned here from the `sway-nodes` side.
    #[test]
    fn an_equal_write_does_not_dirty_the_node() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Vec3>(&mut registry);
        let world = harness::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Vec3::default()));

        harness::tick(&mut graph, &world);
        graph.drain_dirty();
        harness::tick(&mut graph, &world);

        assert!(
            !graph.is_dirty(node),
            "re-evaluating with unchanged inlets must not dirty the node"
        );
    }
}
