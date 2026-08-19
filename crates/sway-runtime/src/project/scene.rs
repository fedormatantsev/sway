//! The scene projector: the nodes that own an entity.
//!
//! One entity per scene node, spawned when the node appears and despawned
//! when it goes. Nothing about a projected entity is authored in the world —
//! every component here is derived from the graph, and the next pass restores
//! anything that was changed behind the graph's back.

use bevy::ecs::system::EntityCommands;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::{DirectionalLight as BevyDirectionalLight, PointLight as BevyPointLight, *};
use bevy::reflect::{Reflect, TypeRegistry};
use sway_graph::graph::{Graph, NodeId};

use crate::nodes::protocol::{self, ReflectMaterialNode, ReflectMeshNode};
use crate::nodes::scene::{self, Camera, DirectionalLight, Group, MeshNode, PointLight};
use crate::project::{NodeEntities, nodes_in_graph_order, source_of};

/// What material a scene entity currently carries, and how to take it off.
///
/// The `detach` pointer is recorded rather than asked for later because an
/// attachment **outlives the node that made it**: deleting a material node
/// drops it from the graph before anything can ask it to detach, while the
/// entity it wrote to still carries the component.
#[derive(Component, Clone, Copy, Debug)]
pub struct MaterialAttachment {
    /// The material node that attached.
    pub node: NodeId,
    /// That node's revision at the time, so an animating material — whose
    /// asset changes in place — does not re-insert its component every tick.
    pub revision: u64,
    detach: fn(&mut EntityCommands),
}

impl MaterialAttachment {
    /// Runs the recorded removal.
    pub fn detach(&self, commands: &mut EntityCommands) {
        (self.detach)(commands);
    }
}

/// Whether a node value is one of the five scene node kinds.
///
/// The set is closed (design D7), so this is an explicit list rather than a
/// trait: adding a sixth kind is a deliberate edit here, not something a new
/// type can opt into.
fn is_scene_node(value: &dyn Reflect) -> bool {
    value.downcast_ref::<MeshNode>().is_some()
        || value.downcast_ref::<Group>().is_some()
        || value.downcast_ref::<Camera>().is_some()
        || value.downcast_ref::<DirectionalLight>().is_some()
        || value.downcast_ref::<PointLight>().is_some()
}

/// The transform a scene node authors, assembled from its three pose inlets.
fn authored_transform(value: &dyn Reflect) -> Option<Transform> {
    if let Some(node) = value.downcast_ref::<MeshNode>() {
        Some(scene::pose(
            node.inlets.translation,
            node.inlets.rotation,
            node.inlets.scale,
        ))
    } else if let Some(node) = value.downcast_ref::<Group>() {
        Some(scene::pose(
            node.inlets.translation,
            node.inlets.rotation,
            node.inlets.scale,
        ))
    } else if let Some(node) = value.downcast_ref::<Camera>() {
        Some(scene::pose(
            node.inlets.translation,
            node.inlets.rotation,
            node.inlets.scale,
        ))
    } else if let Some(node) = value.downcast_ref::<DirectionalLight>() {
        Some(scene::pose(
            node.inlets.translation,
            node.inlets.rotation,
            node.inlets.scale,
        ))
    } else {
        value.downcast_ref::<PointLight>().map(|node| {
            scene::pose(
                node.inlets.translation,
                node.inlets.rotation,
                node.inlets.scale,
            )
        })
    }
}

/// Spawns an entity for every scene node that does not have one yet.
///
/// A full scan rather than a dirty-set walk: it is one map lookup per node,
/// and it means a node can never be left unprojected because its dirty flag
/// was cleared by something else first.
pub fn spawn_scene_entities(
    mut commands: Commands,
    graph: Res<Graph>,
    mut map: ResMut<NodeEntities>,
) {
    for id in nodes_in_graph_order(&graph) {
        let Some(node) = graph.get(id) else {
            continue;
        };
        if !is_scene_node(node.value()) || map.entity(id).is_some() {
            continue;
        }
        // `Transform` brings `GlobalTransform`, and `Visibility` brings the
        // inherited/computed pair — the two things a child needs before its
        // parent can propagate to it.
        let entity = commands
            .spawn((Transform::default(), Visibility::default()))
            .id();
        map.insert(id, entity);
    }
}

/// Writes every scene entity's own components from its node.
///
/// Transforms are compared and written only when they differ, which is both
/// the never-write-an-equal-value rule and what makes "the world is not an
/// authoring surface" observable: a `Transform` changed directly in the world
/// is restored on the next pass, and the graph is untouched.
///
/// The mesh is **edge-derived** and therefore recomputed every pass rather
/// than driven by the dirty set: deleting a mesh node drops its edge without
/// dirtying the scene node at the other end.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // ECS system params
pub fn project_scene_entities(
    mut commands: Commands,
    graph: Res<Graph>,
    registry: Res<AppTypeRegistry>,
    map: Res<NodeEntities>,
    transforms: Query<&Transform>,
    meshes: Query<&Mesh3d>,
    cameras: Query<&Camera3d>,
    directional: Query<&BevyDirectionalLight>,
    point: Query<&BevyPointLight>,
) {
    let type_registry = registry.clone();
    let registry = type_registry.read();

    for id in nodes_in_graph_order(&graph) {
        let Some(entity) = map.entity(id) else {
            continue;
        };
        let Some(node) = graph.get(id) else {
            continue;
        };
        let value = node.value();
        let Some(desired) = authored_transform(value) else {
            continue;
        };
        if transforms.get(entity).ok() != Some(&desired) {
            commands.entity(entity).insert(desired);
        }
        let dirty = graph.is_dirty(id);

        if value.downcast_ref::<MeshNode>().is_some() {
            let handle = mesh_handle(&graph, &registry, id);
            let current = meshes.get(entity).ok().map(|mesh| &mesh.0);
            match handle {
                Some(handle) if current != Some(&handle) => {
                    commands.entity(entity).insert(Mesh3d(handle));
                }
                None if current.is_some() => {
                    commands.entity(entity).remove::<Mesh3d>();
                }
                _ => {}
            }
        } else if value.downcast_ref::<Camera>().is_some() && cameras.get(entity).is_err() {
            // The render target is not set here: `headless::retarget_cameras`
            // points every camera at the viewport texture each `Update`.
            commands.entity(entity).insert(Camera3d::default());
        } else if let Some(light) = value.downcast_ref::<DirectionalLight>() {
            // Lights carry no edge-derived state, so the dirty set is the
            // right trigger; `is_err` covers the pass that spawned them.
            if dirty || directional.get(entity).is_err() {
                commands.entity(entity).insert(light.inlets.to_light());
            }
        } else if let Some(light) = value.downcast_ref::<PointLight>()
            && (dirty || point.get(entity).is_err())
        {
            commands.entity(entity).insert(light.inlets.to_light());
        }
    }
}

/// The mesh handle connected to a scene node's `mesh` inlet, if any.
fn mesh_handle(graph: &Graph, registry: &TypeRegistry, node: NodeId) -> Option<Handle<Mesh>> {
    let source = source_of(graph, node, protocol::MESH)?;
    let value = graph.get(source)?.value();
    let data = registry.get_type_data::<ReflectMeshNode>(value.type_id())?;
    let handle = data.get(value)?.handle().clone();
    (handle != Handle::default()).then_some(handle)
}

/// Applies each connected material node's own material to the scene node it
/// is connected to, and takes it off again when the connection goes.
///
/// Nothing here knows what kind of material it is handling: the node inserts
/// `MeshMaterial3d<M>` itself through [`protocol::MaterialNode`], which is
/// what makes adding a material kind need no change to the scene node kind or
/// to this pass.
pub fn attach_materials(
    mut commands: Commands,
    graph: Res<Graph>,
    registry: Res<AppTypeRegistry>,
    map: Res<NodeEntities>,
    attachments: Query<&MaterialAttachment>,
) {
    let type_registry = registry.clone();
    let registry = type_registry.read();

    for id in nodes_in_graph_order(&graph) {
        let Some(entity) = map.entity(id) else {
            continue;
        };
        let Some(node) = graph.get(id) else {
            continue;
        };
        // Only a `MeshNode` declares a material port. A `Group` refuses one
        // by not having the inlet at all, so `source_of` finds nothing.
        if node.value().downcast_ref::<MeshNode>().is_none() {
            continue;
        }

        let current = attachments.get(entity).ok().copied();
        let desired = source_of(&graph, id, protocol::MATERIAL).and_then(|source| {
            let value = graph.get(source)?.value();
            let data = registry.get_type_data::<ReflectMaterialNode>(value.type_id())?;
            Some((source, data.get(value)?))
        });

        let settled = match (&desired, &current) {
            (Some((source, material)), Some(attached)) => {
                attached.node == *source && attached.revision == material.revision()
            }
            (None, None) => true,
            _ => false,
        };
        if settled {
            continue;
        }

        let mut entity_commands = commands.entity(entity);
        if let Some(attached) = current {
            attached.detach(&mut entity_commands);
            entity_commands.remove::<MaterialAttachment>();
        }
        if let Some((source, material)) = desired {
            material.attach(&mut entity_commands);
            entity_commands.insert(MaterialAttachment {
                node: source,
                revision: material.revision(),
                detach: material.detach(),
            });
        }
    }
}

/// Projects `children` edges into Bevy parenting.
///
/// Transform propagation stays Bevy's: this inserts `ChildOf` and nothing
/// else, and only where a child connection exists — a scene node with no
/// child connection is never given a parent, so its placement is absolute.
///
/// Recomputed every pass for the same reason the mesh is: a deleted group
/// takes its edges with it without dirtying the children that were under it.
pub fn project_children(
    mut commands: Commands,
    graph: Res<Graph>,
    map: Res<NodeEntities>,
    parents: Query<&ChildOf>,
) {
    let desired = desired_parents(&graph);

    for (id, entity) in map.iter() {
        let want = desired.get(&id).and_then(|parent| map.entity(*parent));
        let have = parents.get(entity).ok().map(ChildOf::parent);
        if want == have {
            continue;
        }
        match want {
            Some(parent) => {
                commands.entity(entity).insert(ChildOf(parent));
            }
            None => {
                commands.entity(entity).remove::<ChildOf>();
            }
        }
    }
}

/// Which node each child should sit under, cycles excluded.
///
/// `Graph::connect` refuses a self-connection but not a longer cycle, and a
/// cycle in `ChildOf` is an infinite loop inside Bevy's own hierarchy rather
/// than a diagnostic — so a node that can reach itself by walking up is left
/// unparented instead.
fn desired_parents(graph: &Graph) -> HashMap<NodeId, NodeId> {
    let mut desired: HashMap<NodeId, NodeId> = HashMap::default();
    for edge in graph.edges() {
        if edge.dst.path == protocol::CHILDREN {
            desired.insert(edge.src.node, edge.dst.node);
        }
    }

    let cyclic: Vec<NodeId> = desired
        .keys()
        .copied()
        .filter(|start| {
            let mut seen = HashSet::new();
            let mut at = *start;
            while let Some(parent) = desired.get(&at) {
                if !seen.insert(*parent) || *parent == *start {
                    return true;
                }
                at = *parent;
            }
            false
        })
        .collect();
    for node in cyclic {
        desired.remove(&node);
    }
    desired
}

/// Despawns the entity of every node the graph reports removed, and releases
/// what it owned.
///
/// A producer's asset is released by the node's own drop: the strong handle
/// lived in its `state`, and `Graph::remove` took the node with it.
///
/// **Children are unparented first.** `EntityWorldMut::despawn` takes
/// descendants with it, so despawning a deleted group would otherwise despawn
/// the still-live scene nodes under it.
///
/// A removal also marks every surviving node dirty: it drops every edge
/// naming the removed node *without* dirtying the node at the other end, and
/// re-projecting everything once is far cheaper than a second bookkeeping
/// structure that would have to stay in step.
pub fn despawn_removed_nodes(
    mut commands: Commands,
    mut graph: ResMut<Graph>,
    mut map: ResMut<NodeEntities>,
    children: Query<(Entity, &ChildOf)>,
) {
    let removed = graph.drain_removed();
    if removed.is_empty() {
        return;
    }

    let mut despawned: HashSet<Entity> = HashSet::default();
    for id in removed {
        if let Some(entity) = map.remove(id) {
            despawned.insert(entity);
        }
    }

    for (child, parent) in &children {
        if despawned.contains(&parent.parent()) && !despawned.contains(&child) {
            commands.entity(child).remove::<ChildOf>();
        }
    }
    for entity in &despawned {
        commands.entity(*entity).despawn();
    }

    for id in graph.node_ids() {
        graph.mark_dirty(id);
    }
}
