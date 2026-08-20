//! `MakeVec3`: three scalar inlets in, one vector outlet out.
//!
//! Named for the assembling rather than for the type it produces — a node
//! kind called `Vec3`, in a crate where `bevy_math::Vec3` is what its own
//! outlet is made of, had to be aliased around at every use.
//!
//! An edge names `"x"` / `"y"` / `"z"` directly on `inlets`, so a single
//! component is driveable without a wire type of its own. That is a different
//! tool from reaching into one consumer's vector inlet: this produces a
//! vector that fans out to many.

use bevy_ecs::world::World;
use bevy_math::Vec3;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

/// [`MakeVec3`]'s inlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct MakeVec3In {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// [`MakeVec3`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct MakeVec3Out {
    pub out: Vec3,
}

/// Assembles a vector from three driveable components. Registered under the
/// short kind name `"MakeVec3"`.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct MakeVec3 {
    pub inlets: MakeVec3In,
    pub state: (),
    pub outlets: MakeVec3Out,
}

impl NodeKind for MakeVec3 {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = Vec3::new(self.inlets.x, self.inlets.y, self.inlets.z);
    }
}

#[cfg(test)]
mod tests {
    
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part, Port};

    use super::*;
    use sway_graph::graph::testing;

    #[test]
    fn a_make_vec3_node_publishes_its_three_components() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MakeVec3>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(MakeVec3 {
            inlets: MakeVec3In {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                ..Default::default()
            },
        ));

        testing::tick_once(&mut graph, &world);

        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out.x"), 1.0);
        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out.y"), 2.0);
        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out.z"), 3.0);
    }

    #[test]
    fn a_float_reaches_one_component_in_one_tick() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<MakeVec3>(&mut registry);
        register_node_kind::<crate::nodes::math::Math>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let source = graph.insert(Node::of(crate::nodes::math::Math {
                inlets: crate::nodes::math::MathIn {
                    op: crate::nodes::math::MathOp::Add,
                    a: 0.75,
                    b: 0.0,
                },
                ..Default::default()
            },
        ));
        let vector = graph.insert(Node::of(MakeVec3::default()));
        graph
            .connect(Port::new(source, "out"), Port::new(vector, "y"), 0)
            .expect("legal");

        testing::tick_once(&mut graph, &world);

        assert_eq!(
            testing::read_field::<f32>(&graph, vector, Part::Outlets, "out.x"),
            0.0
        );
        assert_eq!(
            testing::read_field::<f32>(&graph, vector, Part::Outlets, "out.y"),
            0.75,
            "the inlet must land before the node evaluates, in ONE tick"
        );
        assert_eq!(
            testing::read_field::<f32>(&graph, vector, Part::Outlets, "out.z"),
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
        register_node_kind::<MakeVec3>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(MakeVec3::default()));

        testing::tick_once(&mut graph, &world);
        graph.drain_dirty();
        testing::tick_once(&mut graph, &world);

        assert!(
            !graph.is_dirty(node),
            "re-evaluating with unchanged inlets must not dirty the node"
        );
    }
}
