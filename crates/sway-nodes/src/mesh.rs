//! `MeshNode` — where a `Feeds` chain enters the `ChildOf` tree. Design §8.
//!
//! Parent §2.10 calls this boundary most of what an author needs to
//! understand about the two chain kinds, and it is where the cook gate earns
//! its keep: an ungated version re-uploads a mesh asset every tick for a
//! scene that is not moving.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use sway_geo::Geometry;
use sway_graph::{ContinuousIdx, NodeType, PortView, Slot, SlotView, TickCtx, register_slot};

use crate::material::{MaterialOf, MaterialState};

#[derive(Reflect, Default)]
pub struct MeshNodeSlots {
    pub geo: Slot<Geometry>,
    pub material: Slot<MaterialOf<StandardMaterial>>,
}

/// Scalar rotation ports, for the reason `GroupParams` gives.
#[derive(Reflect, Component)]
pub struct MeshNodeParams {
    pub translation: Vec3,
    pub rotation_x: f32,
    pub rotation_y: f32,
    pub rotation_z: f32,
    pub scale: Vec3,
}

impl Default for MeshNodeParams {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale: Vec3::ONE,
        }
    }
}

#[derive(Reflect, Default)]
pub struct MeshNodeOutputs {}

/// A cheap "did the geometry actually change" signal, checked before
/// touching the mesh asset at all.
///
/// This is `P`'s buffer identity, not its contents: `Arc::as_ptr` equality
/// proves the *same `Arc<Vec<Vec3>>` allocation* arrived again, which is
/// exactly the question a cook chain that shares unchanged attributes by
/// refcount (design §5) needs answered — a pass-through operator hands back
/// the identical `Arc` it received, so "same pointer" and "nothing
/// downstream needs to redo work" coincide for this project's cook
/// discipline. It does *not* prove the contents are unchanged in the
/// general case: a hypothetical operator that mutated a `P` buffer in place
/// (nothing in this codebase does — `Attribute` clones share via `Arc`, and
/// no operator holds a unique `Arc` it writes through) would alias this
/// check, and two structurally-identical `Geometry`s built from separate
/// `Arc::new` calls compare as different, costing an extra upload rather
/// than a wrong one. `point_count` rides along as a second, independent
/// guard against the (extremely unlikely) case of an old allocation being
/// freed and a new, differently-sized buffer reusing the same address.
/// The pointer is stored as `usize` and never dereferenced — only ever
/// compared — so this carries no aliasing or lifetime hazard.
type GeometryFingerprint = (usize, usize);

#[derive(Component, Default)]
pub struct MeshNodeState {
    pub mesh: Option<Handle<Mesh>>,
    fingerprint: Option<GeometryFingerprint>,
}

pub struct MeshNode;

impl MeshNode {
    pub const TRANSLATION: u16 = 0;
    pub const ROTATION_X: u16 = 1;
    pub const ROTATION_Y: u16 = 2;
    pub const ROTATION_Z: u16 = 3;
    pub const SCALE: u16 = 4;
    pub const IN_GEO: u16 = 0;
    pub const IN_MATERIAL: u16 = 1;
}

impl NodeType for MeshNode {
    type Params = MeshNodeParams;
    type Outputs = MeshNodeOutputs;
    type Slots = MeshNodeSlots;
    type Produces = ();
    type State = MeshNodeState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("translation", Self::TRANSLATION),
        ("rotation_x", Self::ROTATION_X),
        ("rotation_y", Self::ROTATION_Y),
        ("rotation_z", Self::ROTATION_Z),
        ("scale", Self::SCALE),
    ];
    const SLOT_ORDINALS: &'static [(&'static str, u16)] =
        &[("geo", Self::IN_GEO), ("material", Self::IN_MATERIAL)];
    const SPATIAL: bool = true;
    const COOKS: bool = true;

    fn register(app: &mut App) {
        app.register_type::<Vec3>();
        register_slot::<Geometry>(app);
        register_slot::<MaterialOf<StandardMaterial>>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _t: &TickCtx) {
        let translation: Vec3 = ports.read(ContinuousIdx(Self::TRANSLATION as u32));
        let rx: f32 = ports.read(ContinuousIdx(Self::ROTATION_X as u32));
        let ry: f32 = ports.read(ContinuousIdx(Self::ROTATION_Y as u32));
        let rz: f32 = ports.read(ContinuousIdx(Self::ROTATION_Z as u32));
        let scale: Vec3 = ports.read(ContinuousIdx(Self::SCALE as u32));
        let want = Transform {
            translation,
            rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
            scale,
        };
        match world.get_mut::<Transform>(node) {
            Some(mut transform) => {
                transform.set_if_neq(want);
            }
            None => {
                world.entity_mut(node).insert(want);
            }
        }
    }

    fn cook(world: &mut World, node: Entity, slots: &SlotView) {
        if let Some(source) = slots.source(Self::IN_MATERIAL)
            && let Some(handle) = world.get::<MaterialState>(source).and_then(|s| s.handle.clone())
        {
            let current = world
                .get::<MeshMaterial3d<StandardMaterial>>(node)
                .map(|m| m.0.clone());
            if current.as_ref() != Some(&handle) {
                world.entity_mut(node).insert(MeshMaterial3d(handle));
            }
        }

        let Some(source) = slots.source(Self::IN_GEO) else {
            return;
        };
        let Some(geo) = world.get::<Geometry>(source).cloned() else {
            return;
        };
        let Some(fingerprint) = geometry_fingerprint(&geo) else {
            return;
        };

        // Read, compare, and only then touch the asset: an unconditional
        // `get_mut` + write re-marks the mesh `Modified` for a geometry that
        // did not change, re-running everything downstream that watches the
        // mesh asset for a scene that is not moving (parent §2.11).
        let existing_fingerprint = world.get::<MeshNodeState>(node).and_then(|s| s.fingerprint);
        if existing_fingerprint == Some(fingerprint) {
            return;
        }

        let Some(mesh) = geometry_to_mesh(&geo) else {
            return;
        };

        let existing_handle = world.get::<MeshNodeState>(node).and_then(|s| s.mesh.clone());
        match existing_handle {
            Some(handle) => {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                if let Some(mut slot) = meshes.get_mut(&handle) {
                    *slot = mesh;
                }
            }
            None => {
                let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
                world
                    .entity_mut(node)
                    .insert((Mesh3d(handle.clone()), Visibility::default()));
                if let Some(mut state) = world.get_mut::<MeshNodeState>(node) {
                    state.mesh = Some(handle);
                }
            }
        }
        if let Some(mut state) = world.get_mut::<MeshNodeState>(node) {
            state.fingerprint = Some(fingerprint);
        }
    }
}

/// `P`'s buffer identity plus the point count. See `GeometryFingerprint` for
/// what this does and does not prove. `None` when there is no `P` — the same
/// condition under which `geometry_to_mesh` also declines to produce a mesh.
fn geometry_fingerprint(geo: &Geometry) -> Option<GeometryFingerprint> {
    let positions = geo.get("P")?.as_vec3()?;
    Some((geo.point_count(), Arc::as_ptr(positions) as usize))
}

/// Planar attribute columns → a `bevy` mesh. `P` is required; `N` and `uv`
/// are filled with defaults when absent so a bare point set still draws.
fn geometry_to_mesh(geo: &Geometry) -> Option<Mesh> {
    let positions = geo.get("P")?.as_vec3()?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        positions.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>(),
    );
    let normals: Vec<[f32; 3]> = match geo.get("N").and_then(|a| a.as_vec3()) {
        Some(n) => n.iter().map(|n| [n.x, n.y, n.z]).collect(),
        None => vec![[0.0, 1.0, 0.0]; positions.len()],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    let uvs: Vec<[f32; 2]> = match geo.get("uv").and_then(|a| a.as_vec2()) {
        Some(uv) => uv.iter().map(|uv| [uv.x, uv.y]).collect(),
        None => vec![[0.0, 0.0]; positions.len()],
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(indices) = geo.indices() {
        mesh.insert_indices(Indices::U32(indices.as_ref().clone()));
    }
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sway_geo::{Attribute, Geometry};
    use std::sync::Arc;
    use sway_graph::{SlotSource, SlotView};

    fn quad() -> Geometry {
        let mut g = Geometry::new(4);
        g.set(
            "P",
            Attribute::Vec3(Arc::new(vec![
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
            ])),
        );
        g.set("N", Attribute::Vec3(Arc::new(vec![Vec3::Y; 4])));
        g.set("uv", Attribute::Vec2(Arc::new(vec![Vec2::ZERO; 4])));
        g.set_indices(Some(Arc::new(vec![0, 2, 1, 1, 2, 3])));
        g
    }

    fn app_with_mesh() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>();
        let source = app.world_mut().spawn(quad()).id();
        let node = app
            .world_mut()
            .spawn((MeshNodeParams::default(), MeshNodeState::default()))
            .id();
        (app, source, node)
    }

    /// Runs `MeshNode::cook` and then drives one `app.update()`.
    ///
    /// `Assets::get_mut`/`add` only queue an `AssetEvent`; Bevy 0.19 flushes
    /// that queue into `Messages<AssetEvent<A>>` from the `asset_events`
    /// system, which `init_asset` registers on `PostUpdate` (verified against
    /// vendored `bevy_asset-0.19.0`, `src/lib.rs:661-664`). Calling
    /// `MeshNode::cook` directly, as the brief's version of this helper did,
    /// never runs that schedule, so `count_modified` reads an eternally empty
    /// `Messages` resource and returns 0 regardless of what `cook` did —
    /// confirmed by temporarily deleting the fingerprint gate below and
    /// observing `re_cooking_unchanged_geometry_does_not_modify_the_asset`
    /// still pass. `material.rs`'s `tick_with` hits the identical hazard and
    /// resolves it the same way.
    fn cook(app: &mut App, node: Entity, source: Entity) {
        let slots = [
            Some(SlotSource { entity: source, plan_index: 0 }),
            None,
        ];
        MeshNode::cook(app.world_mut(), node, &SlotView::new(&slots));
        app.update();
    }

    fn count_modified(app: &mut App) -> usize {
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<Mesh>>>()
            .drain()
            .filter(|e| matches!(e, AssetEvent::Modified { .. }))
            .count()
    }

    #[test]
    fn cooking_uploads_the_geometry_as_a_mesh() {
        let (mut app, source, node) = app_with_mesh();
        cook(&mut app, node, source);

        let handle = app
            .world()
            .get::<Mesh3d>(node)
            .map(|m| m.0.clone())
            .expect("the node inserts Mesh3d");
        let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
        assert_eq!(mesh.count_vertices(), 4);
        assert_eq!(mesh.indices().map(|i| i.len()), Some(6));
    }

    #[test]
    fn re_cooking_unchanged_geometry_does_not_modify_the_asset() {
        // The failure parent §2.11 names: re-uploading a mesh every tick for
        // a scene that is not moving. The gate normally prevents the second
        // cook from being called at all; this asserts the node is not
        // *itself* the thing that would churn if it were.
        let (mut app, source, node) = app_with_mesh();
        cook(&mut app, node, source);
        let _ = count_modified(&mut app);

        cook(&mut app, node, source);

        assert_eq!(count_modified(&mut app), 0);
    }

    #[test]
    fn a_material_slot_source_becomes_mesh_material_3d() {
        let (mut app, source, node) = app_with_mesh();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let material_node = app
            .world_mut()
            .spawn(MaterialState { handle: Some(handle.clone()) })
            .id();

        let slots = [
            Some(SlotSource { entity: source, plan_index: 0 }),
            Some(SlotSource { entity: material_node, plan_index: 1 }),
        ];
        MeshNode::cook(app.world_mut(), node, &SlotView::new(&slots));

        assert_eq!(
            app.world().get::<MeshMaterial3d<StandardMaterial>>(node).map(|m| m.0.clone()),
            Some(handle)
        );
    }
}
