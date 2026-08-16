//! Evaluation order. Spec §3.

use std::cmp::{Ord, Ordering};
use std::collections::{BinaryHeap, HashMap, HashSet};

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;

use crate::diagnostics::GraphDiagnostics;
use crate::wire::PropagateFn;

/// `Entity::Ord` is DESCENDING in raw spawn index for bevy_ecs 0.19.0: its
/// NonMaxU32 niche encoding stores the bitwise complement of the index.
/// A plain BinaryHeap (max-heap) over Entity's native Ord therefore pops
/// in ascending raw-index order -- do NOT wrap this in `Reverse`, and do
/// NOT "simplify" this to Reverse<Entity> without re-verifying against
/// whatever Bevy version is pinned at the time.
#[derive(Copy, Clone, Eq, PartialEq)]
struct AscendingEntity(Entity);

impl Ord for AscendingEntity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0) // Uses Entity's descending-index Ord; max-heap pops ascending
    }
}

impl PartialOrd for AscendingEntity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One wire instance, flattened for the sort. `run` is the wire type's
/// monomorphised propagate; the sort ignores it.
#[derive(Clone, Copy)]
pub struct Link {
    pub src: Entity,
    pub dst: Entity,
    pub run: PropagateFn,
    pub wire: &'static str,
}

pub struct Sorted {
    /// Evaluation order: the acyclic part first, then any cycle members in
    /// ascending entity order.
    pub order: Vec<Entity>,
    /// Entities participating in a cycle. Empty in a well-formed graph.
    pub cycles: Vec<Entity>,
}

/// Kahn's algorithm over entities.
///
/// Ties are broken by ascending `Entity` so the order is deterministic — the
/// editor shows it, and §6's tests assert on it.
///
/// A cycle never stops the render (spec §3.3): its members are appended after
/// the acyclic part and read the previous tick's value.
pub fn topological_order(vertices: &[Entity], links: &[Link]) -> Sorted {
    let mut indegree: HashMap<Entity, usize> = vertices.iter().map(|&e| (e, 0)).collect();
    let mut successors: HashMap<Entity, Vec<Entity>> = HashMap::new();

    for link in links {
        // A link naming an entity outside `vertices` is ignored: the sort
        // orders what exists.
        if !indegree.contains_key(&link.src) || !indegree.contains_key(&link.dst) {
            continue;
        }
        *indegree.get_mut(&link.dst).expect("dst is a vertex") += 1;
        successors.entry(link.src).or_default().push(link.dst);
    }

    let mut ready: BinaryHeap<AscendingEntity> = indegree
        .iter()
        .filter(|(_, degree)| *degree == &0)
        .map(|(entity, _)| AscendingEntity(*entity))
        .collect();

    let mut order: Vec<Entity> = Vec::with_capacity(vertices.len());
    while let Some(AscendingEntity(entity)) = ready.pop() {
        order.push(entity);
        for &next in successors.get(&entity).map_or(&[][..], |v| v.as_slice()) {
            let degree = indegree.get_mut(&next).expect("successor is a vertex");
            *degree -= 1;
            if *degree == 0 {
                ready.push(AscendingEntity(next));
            }
        }
    }

    let placed: HashSet<Entity> = order.iter().copied().collect();
    let mut cycles: Vec<Entity> = vertices
        .iter()
        .copied()
        .filter(|e| !placed.contains(e))
        .collect();
    // Compensate for Entity::Ord being descending-in-index: reverse the sort to get
    // ascending raw-index order, consistent with the main order's tie-breaking.
    cycles.sort_by(|a, b| b.cmp(a));
    order.extend(cycles.iter().copied());

    Sorted { order, cycles }
}

use bevy_ecs::resource::Resource;

use crate::registry_wires::{BehaviourFn, BehaviourRegistry, WireRegistry};

/// One unit of work. A step carries its own fn pointer: the list is
/// heterogeneous and `Wire::propagate` is not object-safe (spec §3.1).
///
/// Data, not a closure — the editor shows the order and the tests assert on
/// it.
#[derive(Clone, Copy)]
pub enum Step {
    Propagate {
        run: PropagateFn,
        src: Entity,
        dst: Entity,
        wire: &'static str,
    },
    Run {
        run: BehaviourFn,
        entity: Entity,
    },
}

#[derive(Resource, Default)]
pub struct GraphOrder {
    pub steps: Vec<Step>,
}

/// Set whenever the wiring changes. Starts `true`, so the first tick builds.
#[derive(Resource)]
pub struct TopologyDirty(pub bool);

impl Default for TopologyDirty {
    fn default() -> Self {
        Self(true)
    }
}

/// Rebuilds `GraphOrder` when the topology has changed. Authoring-time only
/// (spec §3.2): during a show nothing sets the flag and this is one bool read.
pub fn rebuild_order(world: &mut World) {
    if !world.resource::<TopologyDirty>().0 {
        return;
    }

    let wires = world.remove_resource::<WireRegistry>().unwrap_or_default();
    let behaviours = world
        .remove_resource::<BehaviourRegistry>()
        .unwrap_or_default();

    let mut links: Vec<Link> = Vec::new();
    for entry in &wires.entries {
        (entry.collect)(world, &mut links);
    }

    let mut behaviour_steps: Vec<(Entity, BehaviourFn)> = Vec::new();
    for entry in &behaviours.entries {
        let mut found = Vec::new();
        (entry.collect)(world, &mut found);
        behaviour_steps.extend(found.into_iter().map(|entity| (entity, entry.run)));
    }

    // Vertices are entities (spec §2.5): everything a wire touches, plus
    // everything carrying a behaviour.
    let mut vertices: Vec<Entity> = Vec::new();
    for link in &links {
        vertices.push(link.src);
        vertices.push(link.dst);
    }
    vertices.extend(behaviour_steps.iter().map(|(entity, _)| *entity));
    vertices.sort();
    vertices.dedup();

    let sorted = topological_order(&vertices, &links);

    let mut diagnostics = GraphDiagnostics {
        cycles: sorted.cycles.clone(),
        ..Default::default()
    };
    for entry in &wires.entries {
        let mut instances: Vec<Link> = Vec::new();
        (entry.collect)(world, &mut instances);
        for link in instances {
            if !(entry.has_source)(world, link.src) {
                diagnostics.missing_source.push((link.src, entry.name));
            }
            if !(entry.has_target)(world, link.dst) {
                diagnostics.missing_target.push((link.dst, entry.name));
            }
        }
    }

    // Per entity, in evaluation order: propagate everything inbound, THEN run
    // its behaviours. That ordering is what lets a driven behaviour see this
    // tick's inputs.
    let mut inbound: HashMap<Entity, Vec<usize>> = HashMap::new();
    for (index, link) in links.iter().enumerate() {
        inbound.entry(link.dst).or_default().push(index);
    }
    let mut behaviours_of: HashMap<Entity, Vec<BehaviourFn>> = HashMap::new();
    for (entity, run) in behaviour_steps {
        behaviours_of.entry(entity).or_default().push(run);
    }

    let mut steps: Vec<Step> = Vec::new();
    for entity in sorted.order {
        for &index in inbound.get(&entity).map_or(&[][..], |v| v.as_slice()) {
            let link = links[index];
            steps.push(Step::Propagate {
                run: link.run,
                src: link.src,
                dst: link.dst,
                wire: link.wire,
            });
        }
        for &run in behaviours_of.get(&entity).map_or(&[][..], |v| v.as_slice()) {
            steps.push(Step::Run { run, entity });
        }
    }

    world.insert_resource(wires);
    world.insert_resource(behaviours);
    world.insert_resource(diagnostics);
    world.insert_resource(GraphOrder { steps });
    world.resource_mut::<TopologyDirty>().0 = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(index: u32) -> Entity {
        Entity::from_raw_u32(index).expect("valid entity index")
    }

    fn noop(_: &mut World, _: Entity, _: Entity) {}

    fn link(src: Entity, dst: Entity) -> Link {
        Link {
            src,
            dst,
            run: noop,
            wire: "test",
        }
    }

    #[test]
    fn a_chain_is_ordered_source_first() {
        // The design's central claim: a chain resolves in one tick, which is
        // only true if the order puts every producer before its consumer.
        let (a, b, c) = (e(3), e(1), e(2));
        let sorted = topological_order(&[a, b, c], &[link(a, b), link(b, c)]);

        assert_eq!(sorted.order, vec![a, b, c]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn independent_vertices_are_ordered_by_entity_for_determinism() {
        let (a, b, c) = (e(2), e(1), e(3));
        let sorted = topological_order(&[a, b, c], &[]);

        assert_eq!(sorted.order, vec![e(1), e(2), e(3)]);
    }

    #[test]
    fn fan_out_puts_the_producer_before_every_consumer() {
        let (src, x, y) = (e(9), e(1), e(2));
        let sorted = topological_order(&[src, x, y], &[link(src, x), link(src, y)]);

        assert_eq!(sorted.order[0], src);
        assert_eq!(&sorted.order[1..], &[x, y]);
    }

    #[test]
    fn a_cycle_is_reported_and_its_members_appended() {
        // Spec §3.3: never stop the render. The acyclic part keeps its order
        // and the cycle's members follow, deterministically.
        let (free, a, b) = (e(1), e(2), e(3));
        let sorted = topological_order(&[free, a, b], &[link(a, b), link(b, a)]);

        assert_eq!(sorted.order, vec![free, a, b]);
        assert_eq!(sorted.cycles, vec![a, b]);
    }

    #[test]
    fn a_link_naming_an_unknown_entity_is_ignored() {
        // A wire whose producer was despawned since the last rebuild must not
        // strand its consumer at indegree 1 forever.
        let (a, gone) = (e(1), e(77));
        let sorted = topological_order(&[a], &[link(gone, a)]);

        assert_eq!(sorted.order, vec![a]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn two_wires_between_the_same_pair_still_order_correctly() {
        // Duplicate constraints must decrement indegree once each.
        let (a, b) = (e(1), e(2));
        let sorted = topological_order(&[a, b], &[link(a, b), link(a, b)]);

        assert_eq!(sorted.order, vec![a, b]);
        assert!(sorted.cycles.is_empty());
    }

    #[test]
    fn entity_ord_is_descending_in_raw_index_for_same_generation() {
        // Pins the bevy_ecs quirk AscendingEntity and the cycle sort both
        // compensate for. If a Bevy upgrade changes this, this test fails
        // loudly instead of silently flipping topological_order's determinism.
        assert!(
            e(1) > e(2),
            "if this fails, Entity::Ord's encoding changed -- \
            re-verify AscendingEntity and the cycles.sort_by direction"
        );
    }

    // --- rebuild_order ------------------------------------------------

    use crate::registry_wires::{register_behaviour, register_wire};
    use crate::test_wires::{FloatOut, Gain, GainFrom, spawn_float, spawn_gain};
    use bevy_app::App;

    fn rebuild_app() -> App {
        let mut app = App::new();
        app.init_resource::<TopologyDirty>();
        app.init_resource::<GraphOrder>();
        register_wire::<GainFrom>(&mut app);
        app
    }

    /// Reads the order back as inspectable pairs. Deliberately does not
    /// compare fn pointers: equal `fn` items are not guaranteed to have equal
    /// addresses across codegen units.
    fn step_shapes(app: &App) -> Vec<(&'static str, Entity, Entity)> {
        app.world()
            .resource::<GraphOrder>()
            .steps
            .iter()
            .map(|step| match *step {
                Step::Propagate { src, dst, .. } => ("propagate", src, dst),
                Step::Run { entity, .. } => ("run", entity, entity),
            })
            .collect()
    }

    #[test]
    fn a_rebuild_emits_propagate_before_the_behaviour_that_consumes_it() {
        // The ordering rule the whole design turns on.
        let mut app = rebuild_app();
        register_behaviour::<Gain>(&mut app, |_, _, _| {});
        let src = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        rebuild_order(app.world_mut());

        assert_eq!(
            step_shapes(&app),
            vec![("propagate", src, dst), ("run", dst, dst)]
        );
    }

    #[test]
    fn a_propagate_step_carries_the_wire_name() {
        let mut app = rebuild_app();
        let src = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        rebuild_order(app.world_mut());

        let Step::Propagate { wire, .. } = app.world().resource::<GraphOrder>().steps[0] else {
            panic!("expected a propagation step");
        };
        assert_eq!(wire, "factor");
    }

    #[test]
    fn a_rebuild_clears_the_dirty_flag() {
        let mut app = rebuild_app();
        rebuild_order(app.world_mut());
        assert!(!app.world().resource::<TopologyDirty>().0);
    }

    #[test]
    fn a_clean_topology_is_not_rebuilt() {
        let mut app = rebuild_app();
        rebuild_order(app.world_mut());

        // Wire something up but do NOT mark dirty: the order must not notice.
        let src = spawn_float(app.world_mut(), 2.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));
        rebuild_order(app.world_mut());

        assert!(step_shapes(&app).is_empty(), "a clean flag means no work");
    }

    #[test]
    fn a_two_hop_chain_is_ordered_end_to_end() {
        let mut app = rebuild_app();
        let a = spawn_float(app.world_mut(), 1.0);
        // `b` is both a consumer and a producer.
        let b = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(b).insert(FloatOut(0.0));
        let c = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(b).insert(GainFrom(a));
        app.world_mut().entity_mut(c).insert(GainFrom(b));

        rebuild_order(app.world_mut());

        assert_eq!(
            step_shapes(&app),
            vec![("propagate", a, b), ("propagate", b, c)]
        );
    }

    // --- diagnostics ---------------------------------------------------

    #[test]
    fn a_cycle_is_reported_in_the_diagnostics() {
        let mut app = rebuild_app();
        app.init_resource::<crate::diagnostics::GraphDiagnostics>();
        // Two Gains driving each other: each is both source and target.
        let a = spawn_gain(app.world_mut(), 0.0);
        let b = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(a).insert(FloatOut(0.0));
        app.world_mut().entity_mut(b).insert(FloatOut(0.0));
        app.world_mut().entity_mut(a).insert(GainFrom(b));
        app.world_mut().entity_mut(b).insert(GainFrom(a));

        rebuild_order(app.world_mut());

        let diagnostics = app
            .world()
            .resource::<crate::diagnostics::GraphDiagnostics>();
        let mut cycles = diagnostics.cycles.clone();
        cycles.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(cycles, want);
        assert!(!diagnostics.is_clean());
    }

    #[test]
    fn a_producer_without_the_source_component_is_reported() {
        let mut app = rebuild_app();
        app.init_resource::<crate::diagnostics::GraphDiagnostics>();
        let bare = app.world_mut().spawn_empty().id();
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(bare));

        rebuild_order(app.world_mut());

        let diagnostics = app
            .world()
            .resource::<crate::diagnostics::GraphDiagnostics>();
        assert_eq!(diagnostics.missing_source, vec![(bare, "factor")]);
    }

    #[test]
    fn a_well_formed_graph_reports_nothing() {
        let mut app = rebuild_app();
        app.init_resource::<crate::diagnostics::GraphDiagnostics>();
        let src = spawn_float(app.world_mut(), 1.0);
        let dst = spawn_gain(app.world_mut(), 0.0);
        app.world_mut().entity_mut(dst).insert(GainFrom(src));

        rebuild_order(app.world_mut());

        assert!(
            app.world()
                .resource::<crate::diagnostics::GraphDiagnostics>()
                .is_clean()
        );
    }
}
