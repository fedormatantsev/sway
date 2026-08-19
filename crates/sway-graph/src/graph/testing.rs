//! Fixture node kinds for the graph model's own tests.
//!
//! Every one of them follows design D3: a struct with exactly `inlets`,
//! `state` and `outlets`, using `()` where a part is empty.

use bevy_ecs::world::World;
use bevy_math::Vec2;
use bevy_reflect::{Reflect, TypeRegistry};
use bevy_time::{Fixed, Time};

use crate::graph::registry::{NodeKind, ReflectNodeKind, register_node_kind};

/// A zero-sized outlet type. Standing in for the protocol markers
/// (`SceneMaterial`, `MeshSource`, …) that `sway-runtime` will declare.
#[derive(Reflect, Default, Clone, Copy, PartialEq, Debug)]
pub struct Marker;

// --- Source: a pure pass-through ---------------------------------------

/// `Source`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct Base {
    /// The value to publish.
    pub base: f32,
}

/// `Source`'s state.
#[derive(Reflect, Default, Debug)]
pub struct Published {
    /// How many times the value was republished.
    pub publishes: u32,
}

/// `Source`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct Out {
    /// The published value.
    pub out: f32,
    /// A valueless port.
    pub marker: Marker,
}

/// Publishes its inlet unchanged. Writes nothing when nothing changed, which
/// is what the equal-write tests turn on.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Source {
    /// Inlets.
    pub inlets: Base,
    /// State.
    pub state: Published,
    /// Outlets.
    pub outlets: Out,
}

impl NodeKind for Source {
    fn evaluate(&mut self, _world: &World) {
        if self.outlets.out != self.inlets.base {
            self.outlets.out = self.inlets.base;
            self.state.publishes += 1;
        }
    }
}

// --- Counter: accumulating state ---------------------------------------

/// `Counter`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct Step {
    /// Added to the running total every tick.
    pub step: f32,
}

/// `Counter`'s state.
#[derive(Reflect, Default, Debug)]
pub struct Accumulator {
    /// The running total.
    pub total: f32,
}

/// `Counter`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct Total {
    /// The running total, published.
    pub total: f32,
}

/// Adds its inlet to a running total every tick.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Counter {
    /// Inlets.
    pub inlets: Step,
    /// State.
    pub state: Accumulator,
    /// Outlets.
    pub outlets: Total,
}

impl NodeKind for Counter {
    fn evaluate(&mut self, _world: &World) {
        self.state.total += self.inlets.step;
        self.outlets.total = self.state.total;
    }
}

// --- Sink: optional and marker inlets, no state ------------------------

/// `Sink`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct SinkIn {
    /// A plain value inlet.
    pub value: f32,
    /// A deliberately incompatible inlet, for the illegal-connection tests.
    pub flag: bool,
    /// An optional inlet: absent, not defaulted, when nothing is connected.
    pub maybe: Option<f32>,
    /// A marker inlet — pure schema; it declares the port exists.
    pub marker: Marker,
}

/// `Sink`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct SinkOut {
    /// Whatever arrived on `value`.
    pub seen: f32,
    /// Whether `maybe` was present this tick.
    pub had_maybe: bool,
}

/// A node with no state at all: its `state` part is `()`.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Sink {
    /// Inlets.
    pub inlets: SinkIn,
    /// State — empty, and addressed exactly like a populated one.
    pub state: (),
    /// Outlets.
    pub outlets: SinkOut,
}

impl NodeKind for Sink {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.seen = self.inlets.value;
        self.outlets.had_maybe = self.inlets.maybe.is_some();
    }
}

// --- Fan: a variadic inlet ---------------------------------------------

/// `Fan`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct FanIn {
    /// A variadic inlet. Its length is derived from the edge set.
    pub values: Vec<f32>,
}

/// `Fan`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct FanOut {
    /// The sum of everything on `values`.
    pub sum: f32,
    /// The values, in the order they arrived.
    pub seen: Vec<f32>,
}

/// Sums a variadic inlet.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Fan {
    /// Inlets.
    pub inlets: FanIn,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: FanOut,
}

impl NodeKind for Fan {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.sum = self.inlets.values.iter().sum();
        self.outlets.seen.clone_from(&self.inlets.values);
    }
}

// --- Nested: a compound inlet ------------------------------------------

/// `Nested`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct Point {
    /// A compound value, addressed one component at a time.
    pub point: Vec2,
}

/// Carries a compound value so nested paths have something to address.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Nested {
    /// Inlets.
    pub inlets: Point,
    /// State.
    pub state: (),
    /// Outlets.
    pub outlets: Point,
}

impl NodeKind for Nested {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.point = self.inlets.point;
    }
}

// --- Clock: an external time source as an ordinary node ----------------

/// `Clock`'s state.
#[derive(Reflect, Default, Debug)]
pub struct Ticks {
    /// How many ticks have run.
    pub ticks: u32,
}

/// `Clock`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct ClockOut {
    /// Seconds accumulated at the fixed timestep.
    pub elapsed: f32,
}

/// Reads the fixed timestep out of `&World` during its own evaluation — no
/// injection phase and no clock type named by the graph
/// (`graph`: an external time source is an ordinary node).
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Clock {
    /// Inlets.
    pub inlets: (),
    /// State.
    pub state: Ticks,
    /// Outlets.
    pub outlets: ClockOut,
}

impl NodeKind for Clock {
    fn evaluate(&mut self, world: &World) {
        // A node cannot reach the graph through `&World`: `resource_scope` has
        // taken it out for the duration of the tick.
        debug_assert!(
            world.get_resource::<crate::graph::model::Graph>().is_none(),
            "the graph must not be reachable from a node's `&World`"
        );
        let dt = world
            .get_resource::<Time<Fixed>>()
            .map_or(0.0, |time| time.delta_secs());
        self.state.ticks += 1;
        self.outlets.elapsed += dt;
    }
}

// --- Placer: a glam-typed outlet ---------------------------------------

/// `Placer`'s inlets.
#[derive(Reflect, Default, Debug)]
pub struct PlacerIn {
    /// The height to place at.
    pub y: f32,
}

/// `Placer`'s outlets.
#[derive(Reflect, Default, Debug)]
pub struct PlacerOut {
    /// A glam-typed outlet.
    ///
    /// Deliberately a `Vec3`: glam's `reflect_partial_eq` answers by
    /// downcasting, so this is the shape that exposes the asymmetry
    /// `reflect_equal` compensates for. A scalar fixture cannot catch it, and
    /// almost every scene node the projectors consume carries one of these.
    pub at: bevy_math::Vec3,
}

/// A node whose outlet is a glam type rather than a scalar.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Placer {
    /// Inlets.
    pub inlets: PlacerIn,
    /// State — empty.
    pub state: (),
    /// Outlets.
    pub outlets: PlacerOut,
}

impl NodeKind for Placer {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.at = bevy_math::Vec3::new(0.0, self.inlets.y, 0.0);
    }
}

/// A registry with every fixture kind registered.
pub fn test_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    registry.register::<Vec2>();
    register_node_kind::<Source>(&mut registry);
    register_node_kind::<Counter>(&mut registry);
    register_node_kind::<Sink>(&mut registry);
    register_node_kind::<Fan>(&mut registry);
    register_node_kind::<Nested>(&mut registry);
    register_node_kind::<Clock>(&mut registry);
    register_node_kind::<Placer>(&mut registry);
    registry
}
