//! `PlaneMesh` — a tessellated quad, subdivided independently per axis.

use bevy::mesh::PlaneMeshBuilder;
use bevy::prelude::*;
use sway_graph::EditorPos;

/// A quad facing +Z, authored with an independent subdivision count along
/// each of its two axes.
///
/// `PlaneMeshBuilder`'s own fields are `subdivisions_x` / `subdivisions_z`,
/// named for a plane whose default normal is +Y — for that plane the X and Z
/// axes are the ones spanned by the quad. This node fixes the normal at +Z
/// instead (design D2), because the material that consumes this mesh (see the
/// `runtime` capability) displaces vertices along the mesh's own normals, and
/// a fixed axis is what lets the authored extent line up with a texture's
/// horizontal and vertical axes. Under that rotation the builder's X axis
/// stays the mesh's horizontal axis but its Z axis becomes the mesh's
/// vertical axis, so `subdivisions_x` / `subdivisions_z` would name the wrong
/// thing here — `horizontal` / `vertical` name the quad as authored and as
/// seen in the atlas. See `build_plane_meshes` for the exact mapping.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(Transform, Visibility, Mesh3d, EditorPos)]
pub struct PlaneMesh {
    pub size: Vec2,
    pub horizontal: u32,
    pub vertical: u32,
}

impl Default for PlaneMesh {
    fn default() -> Self {
        Self {
            size: Vec2::ONE,
            // D2: cost is flat across the useful range (~41k triangles at
            // 63×63, ~655k at 255×255), so this is a knob to turn by eye
            // rather than a budget to compute.
            horizontal: 63,
            vertical: 63,
        }
    }
}

/// An ordinary `Changed<T>` system — the second row of the behaviour table
/// (architecture §2): it consumes nothing the graph produces within a tick.
///
/// `PlaneMeshBuilder::new(Dir3::Z, ..)` builds a plane whose default (+Y)
/// normal has been rotated onto +Z. That rotation fixes the builder's X axis
/// in place and carries its Z axis onto -Y, so the resulting mesh's
/// horizontal axis is governed by `subdivisions_x` and its vertical axis by
/// `subdivisions_z` — the mapping `horizontal -> subdivisions_x`,
/// `vertical -> subdivisions_z` below, per D2's naming rationale.
pub fn build_plane_meshes(
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut planes: Query<(&PlaneMesh, &mut Mesh3d), Changed<PlaneMesh>>,
) {
    for (plane, mut mesh) in &mut planes {
        let built = PlaneMeshBuilder::new(Dir3::Z, plane.size)
            .subdivisions_x(plane.horizontal)
            .subdivisions_z(plane.vertical)
            .build();
        // Not `set_if_neq`, unlike `load_mesh_assets`: `meshes.add` always
        // mints a fresh asset id, so the new handle never compares equal to
        // the old one and the guard would be dead weight. `Changed<PlaneMesh>`
        // on the query is the only guard a rebuild needs.
        *mesh = Mesh3d(mesh_assets.add(built));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use std::collections::BTreeSet;

    /// `AssetPlugin` plus the one asset type, which is all the build system
    /// needs — no device, no renderer. Mirrors `mesh_asset.rs`'s `asset_app`.
    fn asset_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Mesh>();
        app.add_systems(Update, build_plane_meshes);
        app
    }

    /// Reads back the built mesh's positions for the entity, panicking with a
    /// clear message if the system never ran or the handle doesn't resolve.
    fn positions(app: &App, entity: Entity) -> Vec<Vec3> {
        let handle = app
            .world()
            .get::<Mesh3d>(entity)
            .expect("#[require] supplies Mesh3d")
            .0
            .clone();
        let mesh = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&handle)
            .expect("the handle resolves to a built mesh");
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("PlaneMeshBuilder always writes positions")
            .as_float3()
            .expect("positions are stored as f32x3")
            .iter()
            .map(|&p| Vec3::from(p))
            .collect()
    }

    /// Counts how many distinct values (to the nearest tenth of a millimetre,
    /// to swallow float noise) appear along one axis of the built positions.
    fn distinct_along(positions: &[Vec3], axis: impl Fn(Vec3) -> f32) -> usize {
        positions
            .iter()
            .map(|&p| (axis(p) * 10_000.0).round() as i64)
            .collect::<BTreeSet<_>>()
            .len()
    }

    #[test]
    fn independent_subdivision_counts_produce_more_vertex_density_on_the_wider_axis() {
        // Scenario "Subdivision counts produce a tessellated grid". Catches a
        // horizontal/vertical mix-up: if the fields were swapped, this would
        // assert the opposite inequality and still need to fail.
        let mut app = asset_app();
        let entity = app
            .world_mut()
            .spawn(PlaneMesh {
                size: Vec2::ONE,
                horizontal: 3,
                vertical: 1,
            })
            .id();

        app.update();

        let positions = positions(&app, entity);
        let horizontal_density = distinct_along(&positions, |p| p.x);
        let vertical_density = distinct_along(&positions, |p| p.y);
        assert_eq!(
            horizontal_density, 5,
            "3 subdivisions -> 5 distinct x values"
        );
        assert_eq!(vertical_density, 3, "1 subdivision -> 3 distinct y values");
        assert!(
            horizontal_density > vertical_density,
            "horizontal: 3 must tessellate the X axis more densely than vertical: 1 tessellates Y"
        );
    }

    #[test]
    fn zero_subdivisions_on_both_axes_is_a_flat_quad_of_four_vertices() {
        // Scenario "Zero subdivisions is a flat quad".
        let mut app = asset_app();
        let entity = app
            .world_mut()
            .spawn(PlaneMesh {
                size: Vec2::ONE,
                horizontal: 0,
                vertical: 0,
            })
            .id();

        app.update();

        assert_eq!(positions(&app, entity).len(), 4);
    }

    #[test]
    fn changing_only_vertical_on_an_existing_plane_mesh_leaves_horizontal_density_alone() {
        // Scenario "Subdivision counts are independent". Catches a system
        // that rebuilds from scratch with the wrong field, or one that
        // couples the two axes together.
        let mut app = asset_app();
        let entity = app
            .world_mut()
            .spawn(PlaneMesh {
                size: Vec2::ONE,
                horizontal: 4,
                vertical: 0,
            })
            .id();
        app.update();
        let horizontal_density_before = distinct_along(&positions(&app, entity), |p| p.x);

        app.world_mut()
            .get_mut::<PlaneMesh>(entity)
            .expect("present")
            .vertical = 5;
        app.update();

        let positions_after = positions(&app, entity);
        assert_eq!(
            distinct_along(&positions_after, |p| p.x),
            horizontal_density_before,
            "editing vertical must not change horizontal density"
        );
        assert_eq!(
            distinct_along(&positions_after, |p| p.y),
            7,
            "vertical: 5 -> 7 distinct y values"
        );
    }
}
