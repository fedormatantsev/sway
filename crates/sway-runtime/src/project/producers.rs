//! The producer projectors: the nodes that own an asset and no entity.
//!
//! Each allocates its handle **structurally** — the first time the node is
//! projected — and updates the asset's *contents* per tick while the node is
//! dirty (design D7). The handle is then stable, so a consumer that already
//! holds it picks every later edit up without the handle moving. That is what
//! makes one mesh node serving three placements load once, and what makes a
//! hot-reloaded frame reach every material without rewiring anything.

use bevy::asset::{AssetEvent, AssetId, LoadedFolder};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::WgpuLimits;
use sway_graph::graph::{Graph, NodeId};

use crate::nodes::frame_sequence::FrameSequence;
use crate::nodes::frame_sequence::{assemble_layers, changed_id, sort_frames_by_name};
use crate::nodes::mesh::{MeshAsset, PlaneMesh, build_plane};
use crate::project::{dirty_in_graph_order, nodes_in_graph_order};

/// Marks every node reading `producer`'s outlets dirty.
///
/// A producer whose *published handle* changed has changed what its consumers
/// would read, and a marker edge propagates nothing — so nothing else would
/// have dirtied them. Called only when something actually moved, so this
/// cannot spin.
fn dirty_consumers(graph: &mut Graph, producer: NodeId) {
    let consumers: Vec<NodeId> = graph
        .edges_from(producer)
        .map(|edge| edge.dst.node)
        .collect();
    for consumer in consumers {
        graph.mark_dirty(consumer);
    }
}

/// Loads each dirty [`MeshAsset`]'s path.
///
/// `AssetServer::load` returns its handle synchronously, so the handle exists
/// from the first pass and only its *content* is ever pending — which is the
/// whole of design D7's "a connection is never waiting on a handle that does
/// not exist yet".
pub fn project_mesh_assets(mut graph: ResMut<Graph>, asset_server: Res<AssetServer>) {
    let mut published = Vec::new();
    for id in dirty_in_graph_order(&graph) {
        let Some(node) = graph.get_mut(id) else {
            continue;
        };
        let Some(mesh) = node.value_mut().downcast_mut::<MeshAsset>() else {
            continue;
        };
        // What a palette click produces before anyone types a path. Asking
        // the asset server to load "" logs an error every frame.
        if mesh.inlets.path.is_empty() || mesh.state.loaded == mesh.inlets.path {
            continue;
        }
        mesh.state.loaded = mesh.inlets.path.clone();
        mesh.state.handle = asset_server.load(mesh.inlets.path.clone());
        published.push(id);
    }
    for id in published {
        dirty_consumers(&mut graph, id);
    }
}

/// Rebuilds each dirty [`PlaneMesh`].
///
/// The mesh is mutated in place after the first build, so a scene node
/// already holding the handle sees the new tessellation without the handle —
/// and therefore its `Mesh3d` — changing.
pub fn project_plane_meshes(mut graph: ResMut<Graph>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut published = Vec::new();
    for id in dirty_in_graph_order(&graph) {
        let Some(node) = graph.get_mut(id) else {
            continue;
        };
        let Some(plane) = node.value_mut().downcast_mut::<PlaneMesh>() else {
            continue;
        };
        let built = build_plane(&plane.inlets);
        // Never "when the asset is missing": the default handle is the
        // engine's own, and `get_mut` would happily overwrite it.
        if plane.state.handle != Handle::default()
            && let Some(mut existing) = meshes.get_mut(&plane.state.handle)
        {
            *existing = built;
        } else {
            plane.state.handle = meshes.add(built);
            published.push(id);
        }
    }
    for id in published {
        dirty_consumers(&mut graph, id);
    }
}

/// Enumerates each [`FrameSequence`]'s folder and publishes it as one array
/// texture.
///
/// Driven by the dirty set *and* by asset messages: a folder that finishes
/// enumerating, or a frame that hot-reloads, changes the outcome without
/// touching the graph at all. An attempt is cleared before it runs rather
/// than after, so a sequence whose frames are still in flight waits for the
/// message announcing their arrival instead of retrying every frame.
///
/// Publishing mutates the texture in place once allocated, so a `file_watcher`
/// reload reaches every consuming material without the handle changing.
#[allow(clippy::too_many_arguments)] // resources a folder assembly genuinely needs
pub fn project_frame_sequences(
    mut graph: ResMut<Graph>,
    asset_server: Res<AssetServer>,
    folders: Res<Assets<LoadedFolder>>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
    mut folder_messages: MessageReader<AssetEvent<LoadedFolder>>,
    mut image_messages: MessageReader<AssetEvent<Image>>,
) {
    let touched_folders: Vec<AssetId<LoadedFolder>> =
        folder_messages.read().filter_map(changed_id).collect();
    // The published texture is never one of these: it is in no folder, so
    // this system's own writes cannot re-trigger an assembly and spin.
    let touched_images: Vec<AssetId<Image>> =
        image_messages.read().filter_map(changed_id).collect();

    // `RenderDevice` is reachable from the main world in Bevy 0.19. It is
    // `Option` only because a device-free test app has no such resource;
    // there the honest bound is wgpu's own default, the floor any device is
    // guaranteed to meet.
    let max_layers = device
        .map(|device| device.limits().max_texture_array_layers)
        .unwrap_or_else(|| WgpuLimits::default().max_texture_array_layers);

    // Every sequence, not just the dirty ones: an asset message names an
    // asset, not a node, and the node it belongs to may be perfectly clean.
    let mut published = Vec::new();
    for id in nodes_in_graph_order(&graph) {
        let dirty = graph.is_dirty(id);
        let Some(node) = graph.get_mut(id) else {
            continue;
        };
        let Some(sequence) = node.value_mut().downcast_mut::<FrameSequence>() else {
            continue;
        };
        if sequence.inlets.folder.is_empty() {
            continue;
        }

        if sequence.state.folder_path != sequence.inlets.folder {
            sequence.state.folder_path = sequence.inlets.folder.clone();
            sequence.state.folder = asset_server.load_folder(&sequence.inlets.folder);
            sequence.state.pending = true;
            sequence.state.reported = None;
        } else if dirty {
            // The colour space moved without the folder moving: the frames
            // are still the right ones, only their interpretation changed.
            sequence.state.pending = true;
        }

        let folder = folders.get(&sequence.state.folder);
        if touched_folders.contains(&sequence.state.folder.id()) {
            sequence.state.pending = true;
        }
        if let Some(folder) = folder
            && !touched_images.is_empty()
            && folder
                .handles
                .iter()
                .any(|handle| touched_images.iter().any(|id| *id == handle.id().typed()))
        {
            sequence.state.pending = true;
        }

        if !sequence.state.pending {
            continue;
        }
        sequence.state.pending = false;

        let Some(folder) = folder else {
            continue;
        };

        let mut frames: Vec<(String, Handle<Image>)> = Vec::with_capacity(folder.handles.len());
        for handle in &folder.handles {
            let name = handle
                .path()
                .map(|path| path.path().to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Ok(image) = handle.clone().try_typed::<Image>() {
                frames.push((name, image));
            }
        }
        sort_frames_by_name(&mut frames);

        let assembled = {
            let mut resolved: Vec<(&str, &Image)> = Vec::with_capacity(frames.len());
            let mut ready = true;
            for (name, handle) in &frames {
                match images.get(handle) {
                    Some(image) => resolved.push((name.as_str(), image)),
                    // A frame still in flight is not an error, and nothing is
                    // published until every one has arrived.
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            assemble_layers(&resolved, sequence.inlets.color_space, max_layers)
        };

        let assembled = match assembled {
            Ok(image) => image,
            Err(error) => {
                let message = format!("frame sequence '{}' {error}", sequence.inlets.folder);
                if sequence.state.reported.as_deref() != Some(message.as_str()) {
                    error!("{message}");
                    sequence.state.reported = Some(message);
                }
                // Publish nothing rather than something wrong: a material
                // whose run is unavailable renders nothing, which is the
                // visible failure the spec asks for.
                if sequence.state.texture != Handle::default() || sequence.state.layers != 0 {
                    sequence.state.texture = Handle::default();
                    sequence.state.layers = 0;
                    published.push(id);
                }
                continue;
            }
        };
        sequence.state.reported = None;

        let layers = assembled.texture_descriptor.size.depth_or_array_layers;
        if sequence.state.texture != Handle::default()
            && let Some(mut existing) = images.get_mut(&sequence.state.texture)
        {
            *existing = assembled;
            if sequence.state.layers != layers {
                sequence.state.layers = layers;
                published.push(id);
            }
        } else {
            sequence.state.texture = images.add(assembled);
            sequence.state.layers = layers;
            published.push(id);
        }
    }

    for id in published {
        dirty_consumers(&mut graph, id);
    }
}
