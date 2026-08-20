//! Which node the editor is pointed at.
//!
//! Selection is editor state, not graph state: selecting a node changes
//! nothing about what any node *is*, nothing projected from it is respawned or
//! rewritten, and no node is reported as changed. The graph therefore holds
//! none of it.
//!
//! It lives in a crate of its own because two crates share it and neither may
//! depend on the other: the editor's panes set and display it, and the editor
//! viewport's picker sets it while its gizmo follows it.

use bevy_ecs::resource::Resource;
use bevy_ecs::system::{Res, ResMut};
use sway_graph::graph::{Graph, NodeId};

/// The selected node, if any.
///
/// Deliberately not persisted — a reopened project starts with nothing
/// selected.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection(pub Option<NodeId>);

impl Selection {
    /// The selected node, if any.
    pub fn get(self) -> Option<NodeId> {
        self.0
    }

    /// Points at a node, or at nothing.
    pub fn set(&mut self, node: Option<NodeId>) {
        self.0 = node;
    }
}

/// Drops the selection when the node it names is deleted.
///
/// Driven off `Graph::removed()` rather than off the delete gesture, so a node
/// removed by any route — a document reload, the palette, a keystroke — clears
/// it. Peeks rather than drains: the projectors need the same list.
pub fn clear_selection_of_removed_nodes(
    graph: Option<Res<Graph>>,
    mut selection: ResMut<Selection>,
) {
    let (Some(graph), Some(selected)) = (graph, selection.0) else {
        return;
    };
    if graph.removed().contains(&selected) {
        selection.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::system::RunSystemOnce;
    use bevy_ecs::world::World;
    use bevy_reflect::Reflect;
    use sway_graph::graph::{Node, NodeKind, ReflectNodeKind, register_node_kind};

    #[derive(Reflect, Default, Debug)]
    #[reflect(NodeKind)]
    struct Fixture {
        inlets: (),
        state: (),
        outlets: (),
    }
    impl NodeKind for Fixture {
        fn evaluate(&mut self, _world: &World) {}
    }

    fn world_with_a_graph() -> World {
        let mut world = World::new();
        world.insert_resource(Graph::default());
        world.insert_resource(Selection::default());
        world
    }

    #[test]
    fn selecting_a_node_reports_no_change() {
        let mut world = world_with_a_graph();
        let id = {
            let mut graph = world.resource_mut::<Graph>();
            let id = graph.insert(Node::of(Fixture::default()));
            graph.drain_dirty();
            id
        };

        world.resource_mut::<Selection>().set(Some(id));

        assert_eq!(world.resource::<Selection>().get(), Some(id));
        assert!(
            world.resource_mut::<Graph>().drain_dirty().is_empty(),
            "selection lives outside the graph, so nothing can be reported changed"
        );
    }

    #[test]
    fn a_deleted_node_clears_the_selection() {
        let mut world = world_with_a_graph();
        let id = {
            let mut graph = world.resource_mut::<Graph>();
            graph.insert(Node::of(Fixture::default()))
        };
        world.resource_mut::<Selection>().set(Some(id));
        world.resource_mut::<Graph>().remove(id);

        world
            .run_system_once(clear_selection_of_removed_nodes)
            .expect("runs");

        assert_eq!(world.resource::<Selection>().get(), None);
    }

    #[test]
    fn deleting_some_other_node_leaves_the_selection_alone() {
        let mut world = world_with_a_graph();
        let (kept, dropped) = {
            let mut graph = world.resource_mut::<Graph>();
            (
                graph.insert(Node::of(Fixture::default())),
                graph.insert(Node::of(Fixture::default())),
            )
        };
        world.resource_mut::<Selection>().set(Some(kept));
        world.resource_mut::<Graph>().remove(dropped);

        world
            .run_system_once(clear_selection_of_removed_nodes)
            .expect("runs");

        assert_eq!(world.resource::<Selection>().get(), Some(kept));
    }

    #[test]
    fn a_registered_kind_keeps_the_fixture_honest() {
        // `register_node_kind` is what a real selectable node goes through;
        // the fixture is registered the same way so nothing here relies on a
        // shape the graph would refuse.
        let mut registry = bevy_reflect::TypeRegistry::new();
        register_node_kind::<Fixture>(&mut registry);
        assert!(sway_graph::graph::registered_node_kinds(&registry).len() == 1);
    }
}
