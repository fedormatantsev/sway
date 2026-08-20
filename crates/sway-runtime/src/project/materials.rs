//! The material projectors: the nodes that own an `Assets<M>` entry and no
//! entity.
//!
//! A material node is the only thing in the process that knows its concrete
//! `M`. These systems build the asset — one per material kind, hand-written,
//! per design D7 — and the generic
//! [`attach_materials`](crate::project::attach_materials) pass puts it on the
//! scene nodes connected to it through [`MaterialNode`], which is the only
//! part that has to stay open to new material kinds.

use bevy::prelude::*;
use bevy::reflect::TypeRegistry;
use sway_graph::graph::{Graph, NodeId};

use crate::nodes::pbr_material::{PbrMaterial, to_standard_material};
use crate::nodes::protocol::{self, ReflectImageSequenceNode};
use crate::nodes::sprite_material::SpriteMaterial;
use crate::project::{dirty_in_graph_order, source_of};
use crate::nodes::sprite_material::{SpriteMaterialAsset, SpriteMaterialUniform, layer_index};

/// Marks every node reading `producer`'s outlets dirty, so an attachment can
/// notice that what it would attach has changed.
fn dirty_consumers(graph: &mut Graph, producer: NodeId) {
    let consumers: Vec<NodeId> = graph
        .edges_from(producer)
        .map(|edge| edge.dst.node)
        .collect();
    for consumer in consumers {
        graph.mark_dirty(consumer);
    }
}

/// Builds and maintains each dirty [`PbrMaterial`]'s `StandardMaterial`.
///
/// **Handle discipline.** Allocate only when the state still holds
/// `Handle::default()` — never "when the asset is missing", because
/// `PbrPlugin` seeds a real fallback material at the default handle and
/// `get_mut` would happily overwrite the engine's shared default — and
/// otherwise mutate in place, so one material node connected to three scene
/// nodes stays one asset and an edit reaches all three.
pub fn project_pbr_materials(
    mut graph: ResMut<Graph>,
    mut assets: ResMut<Assets<StandardMaterial>>,
) {
    let mut allocated = Vec::new();
    for id in dirty_in_graph_order(&graph) {
        let Some(node) = graph.get_mut(id) else {
            continue;
        };
        let Some(material) = node.value_mut().downcast_mut::<PbrMaterial>() else {
            continue;
        };
        let desired = to_standard_material(&material.inlets);
        if material.state.handle != Handle::default()
            && let Some(mut existing) = assets.get_mut(&material.state.handle)
        {
            *existing = desired;
        } else {
            material.state.handle = assets.add(desired);
            material.state.revision += 1;
            allocated.push(id);
        }
    }
    for id in allocated {
        dirty_consumers(&mut graph, id);
    }
}

/// What a [`SpriteMaterial`]'s connected run offers: its texture and how many
/// layers it really has.
fn run(
    graph: &Graph,
    registry: &TypeRegistry,
    node: NodeId,
    path: &str,
) -> Option<(Handle<Image>, u32)> {
    let source = source_of(graph, node, path)?;
    let value = graph.get(source)?.value();
    let data = registry.get_type_data::<ReflectImageSequenceNode>(value.type_id())?;
    let sequence = data.get(value)?;
    // A sequence with no layers has published nothing — either it has not
    // finished loading or it failed to assemble. Either way there is no
    // texture to sample, so it counts as unavailable rather than empty.
    (sequence.layers() > 0).then(|| (sequence.texture().clone(), sequence.layers()))
}

/// Whether two sprite material assets would draw identically.
///
/// `SpriteMaterialAsset` has no `PartialEq` — it is an `AsBindGroup` type —
/// so this is the comparison that keeps a settled material from rewriting,
/// and therefore re-uploading, its uniform every frame.
fn same(a: &SpriteMaterialAsset, b: &SpriteMaterialAsset) -> bool {
    a.uniform == b.uniform
        && a.color_texture == b.color_texture
        && a.depth_texture == b.depth_texture
}

/// Builds and maintains each dirty [`SpriteMaterial`]'s asset.
///
/// **An incomplete material renders nothing** by publishing nothing: the
/// handle is dropped, so the scene node's `MeshMaterial3d` resolves to no
/// asset and material extraction skips the entity. Not a fallback material
/// and not a zero-alpha one — `ImagePlugin` seeds a real 1×1 white image at
/// `Handle::default()`, so an asset published with a missing run would draw a
/// plain white quad, which is exactly the "renders incorrectly" the spec
/// forbids.
///
/// This is the one material whose handle is **not** allocated structurally at
/// creation, and the paragraph above is why. A connection to it is still
/// never left waiting: the marker edge from each `FrameSequence` is a sort
/// constraint, so the sequences are projected first and the handle exists
/// before the scene node's attachment pass runs in the same frame.
pub fn project_sprite_materials(
    mut graph: ResMut<Graph>,
    registry: Res<AppTypeRegistry>,
    mut assets: ResMut<Assets<SpriteMaterialAsset>>,
) {
    let registry = registry.clone();
    let registry = registry.read();
    let mut allocated = Vec::new();

    for id in dirty_in_graph_order(&graph) {
        if graph
            .get(id)
            .is_none_or(|node| node.value().downcast_ref::<SpriteMaterial>().is_none())
        {
            continue;
        }
        let color = run(&graph, &registry, id, protocol::COLOR);
        let depth = run(&graph, &registry, id, protocol::DEPTH);

        let Some(node) = graph.get_mut(id) else {
            continue;
        };
        let Some(material) = node.value_mut().downcast_mut::<SpriteMaterial>() else {
            continue;
        };

        let (Some((color_texture, color_layers)), Some((depth_texture, depth_layers))) =
            (color, depth)
        else {
            material.state.reported = None;
            material.state.layers = 0;
            if material.state.handle != Handle::default() {
                // Dropping the handle is what makes "renders nothing" happen:
                // the asset goes with it.
                material.state.handle = Handle::default();
                material.state.revision += 1;
                allocated.push(id);
            }
            continue;
        };

        // Layer counts only. Differing *resolutions* are legal — both runs
        // are addressed by normalized UVs and the depth run's useful
        // resolution is bounded by the mesh's tessellation, not by the colour
        // run — so nothing here looks at width or height.
        let layers = color_layers.min(depth_layers);
        if color_layers == depth_layers {
            material.state.reported = None;
        } else {
            let message = format!(
                "sprite material {id} has a {color_layers}-layer colour run and a \
                 {depth_layers}-layer depth run; the frame number is bounded by {layers}"
            );
            // Compared against the last message for this node, so a permanent
            // mismatch on an animating material — whose published uniform
            // changes every tick — is logged once, not at the tick rate.
            if material.state.reported.as_deref() != Some(message.as_str()) {
                warn!("{message}");
                material.state.reported = Some(message);
            }
        }

        // sRGB in, linear out. The colour run is sampled through an sRGB view
        // and is already linear where the shader multiplies, so an unconverted
        // tint would multiply two different encodings and mid-tones would come
        // out visibly wrong. Opacity is not a colour and carries no transfer
        // curve.
        let tint = Color::srgb(
            material.inlets.tint.x,
            material.inlets.tint.y,
            material.inlets.tint.z,
        )
        .to_linear();
        let desired = SpriteMaterialAsset {
            uniform: SpriteMaterialUniform {
                tint: Vec4::new(tint.red, tint.green, tint.blue, material.inlets.opacity),
                layer: layer_index(material.inlets.frame, layers),
                depth_pivot: material.inlets.depth_pivot,
                depth_range: material.inlets.depth_range,
            },
            color_texture,
            depth_texture,
        };

        material.state.layers = layers;
        if material.state.handle != Handle::default()
            && let Some(mut existing) = assets.get_mut(&material.state.handle)
        {
            if !same(&existing, &desired) {
                *existing = desired;
            }
        } else {
            material.state.handle = assets.add(desired);
            material.state.revision += 1;
            allocated.push(id);
        }
    }

    drop(registry);
    for id in allocated {
        dirty_consumers(&mut graph, id);
    }
}
