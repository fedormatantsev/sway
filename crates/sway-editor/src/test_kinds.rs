//! Node-kind fixtures for the graph-model editor tests.
//!
//! `sway-editor` deliberately does not depend on `sway-nodes` (see the crate
//! doc): the whole point of the reflected read path is that the editor needs no
//! editor-side description of any particular node kind, so the kinds it tests
//! against are declared here rather than borrowed from the node library.

use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use bevy_reflect::{Reflect, TypePath, TypeRegistry};
use sway_graph::graph::{Graph, Node, NodeId, NodeKind, Port, ReflectNodeKind, register_node_kind};

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub enum Wave {
    #[default]
    Sine,
    Saw,
}

#[derive(Reflect, Default, Debug)]
pub struct SourceIn {
    pub level: f32,
    pub label: String,
    pub enabled: bool,
    pub shape: Wave,
}

#[derive(Reflect, Default, Debug)]
pub struct SourceOut {
    pub out: f32,
    pub pair: Vec2,
}

/// Two outlets of different types, and one inlet per editable control, so a
/// single kind exercises every control the inspector offers.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Source {
    pub inlets: SourceIn,
    pub state: (),
    pub outlets: SourceOut,
}

impl NodeKind for Source {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = self.inlets.level;
    }
}

#[derive(Reflect, Default, Debug)]
pub struct GateIn {
    pub gate: Option<f32>,
    pub amount: f32,
}

#[derive(Reflect, Default, Debug)]
pub struct GateOut {
    pub out: f32,
}

/// An optional inlet beside a plain one -- two inlets that must stay distinct.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Gate {
    pub inlets: GateIn,
    pub state: (),
    pub outlets: GateOut,
}

impl NodeKind for Gate {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.out = self.inlets.gate.unwrap_or_default() * self.inlets.amount;
    }
}

#[derive(Reflect, Default, Debug)]
pub struct MixerIn {
    pub terms: Vec<f32>,
}

#[derive(Reflect, Default, Debug)]
pub struct MixerOut {
    pub total: f32,
}

/// A variadic inlet: several edges land on `terms`, ordered by slot.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Mixer {
    pub inlets: MixerIn,
    pub state: (),
    pub outlets: MixerOut,
}

impl NodeKind for Mixer {
    fn evaluate(&mut self, _world: &World) {
        self.outlets.total = self.inlets.terms.iter().sum();
    }
}

#[derive(Reflect, Default, Debug)]
pub struct MemoryIn {
    pub rate: f32,
}

#[derive(Reflect, Default, Debug)]
pub struct MemoryState {
    pub phase: f32,
}

#[derive(Reflect, Default, Debug)]
pub struct MemoryOut {
    pub out: f32,
}

/// A kind with a populated `state` part, so a test can prove state is never
/// listed in the inspector.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Memory {
    pub inlets: MemoryIn,
    pub state: MemoryState,
    pub outlets: MemoryOut,
}

impl NodeKind for Memory {
    fn evaluate(&mut self, _world: &World) {
        self.state.phase += self.inlets.rate;
        self.outlets.out = self.state.phase;
    }
}

#[derive(Reflect, Default, Debug)]
pub struct PlacerIn {
    pub position: Vec3,
}

#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Placer {
    pub inlets: PlacerIn,
    pub state: (),
    pub outlets: (),
}

impl NodeKind for Placer {
    fn evaluate(&mut self, _world: &World) {}
}

impl Source {
    pub fn path() -> &'static str {
        <Self as TypePath>::type_path()
    }
}

impl Gate {
    pub fn path() -> &'static str {
        <Self as TypePath>::type_path()
    }
}

impl Mixer {
    pub fn path() -> &'static str {
        <Self as TypePath>::type_path()
    }
}

impl Placer {
    pub fn path() -> &'static str {
        <Self as TypePath>::type_path()
    }
}

impl Memory {
    pub fn path() -> &'static str {
        <Self as TypePath>::type_path()
    }
}

/// A registry holding exactly the fixture kinds.
pub fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();
    register_node_kind::<Source>(&mut registry);
    register_node_kind::<Gate>(&mut registry);
    register_node_kind::<Mixer>(&mut registry);
    register_node_kind::<Placer>(&mut registry);
    register_node_kind::<Memory>(&mut registry);
    registry
}

/// A node of `value`, annotated with the canvas placement the editor would
/// have given it. Placement is editor state carried in the node's annotations,
/// so a fixture that cares where a node sits has to say so the same way the
/// editor does.
pub fn placed<T: Reflect + TypePath>(pos: Vec2, value: T) -> Node {
    let mut node = Node::of(value);
    node.metadata_mut()
        .insert(crate::canvas::CANVAS_POS_KEY.to_string(), Box::new(pos));
    node
}

/// A `Source` feeding a `Gate`: two nodes, one edge, and an inlet
/// (`Gate::amount`) with nothing connected to it.
pub fn source_and_gate() -> (Graph, NodeId, NodeId) {
    let mut graph = Graph::default();
    let source = graph.insert(placed(Vec2::new(0.0, 0.0), Source::default()));
    let gate = graph.insert(placed(Vec2::new(400.0, 0.0), Gate::default()));
    graph
        .connect(Port::new(source, "out"), Port::new(gate, "gate"), 0)
        .expect("f32 -> Option<f32> is a legal optional connection");
    (graph, source, gate)
}

/// Two `Source`s, the first driving the second's `level` inlet -- a plain
/// `f32 -> f32` connection into a field that still has an editing control, so
/// a test can prove a *connected* inlet stays editable.
pub fn chained_sources() -> (Graph, NodeId, NodeId) {
    let mut graph = Graph::default();
    let driver = graph.insert(placed(Vec2::new(0.0, 0.0), Source::default()));
    let driven = graph.insert(placed(Vec2::new(400.0, 0.0), Source::default()));
    graph
        .connect(Port::new(driver, "out"), Port::new(driven, "level"), 0)
        .expect("f32 -> f32 is a direct connection");
    (graph, driver, driven)
}

/// Three `Source`s feeding one `Mixer`'s variadic `terms` inlet at sparse
/// slots, deliberately connected out of slot order.
pub fn variadic_graph() -> (Graph, Vec<NodeId>, NodeId) {
    let mut graph = Graph::default();
    let mixer = graph.insert(placed(Vec2::new(400.0, 0.0), Mixer::default()));
    let mut sources = Vec::new();
    for (index, slot) in [30, 10, 20].into_iter().enumerate() {
        let source = graph.insert(placed(
            Vec2::new(0.0, index as f32 * 100.0),
            Source::default(),
        ));
        graph
            .connect(Port::new(source, "out"), Port::new(mixer, "terms"), slot)
            .expect("f32 -> Vec<f32> is a legal variadic connection");
        sources.push(source);
    }
    (graph, sources, mixer)
}
