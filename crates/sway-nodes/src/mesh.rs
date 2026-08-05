//! Pure geometry-to-mesh conversion retained for future asset wires.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use sway_geo::Geometry;

pub fn geometry_to_mesh(geometry: &Geometry) -> Option<Mesh> {
    let positions = geometry.get("P")?.as_vec3()?;
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        positions
            .iter()
            .map(|position| [position.x, position.y, position.z])
            .collect::<Vec<_>>(),
    );
    let normals = geometry
        .get("N")
        .and_then(|attribute| attribute.as_vec3())
        .map(|values| {
            values
                .iter()
                .map(|value| [value.x, value.y, value.z])
                .collect()
        })
        .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    let uvs = geometry
        .get("uv")
        .and_then(|attribute| attribute.as_vec2())
        .map(|values| values.iter().map(|value| [value.x, value.y]).collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    if let Some(indices) = geometry.indices() {
        mesh.insert_indices(Indices::U32(indices.as_ref().clone()));
    }
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use sway_geo::Attribute;

    #[test]
    fn geometry_attributes_become_mesh_buffers() {
        let mut geometry = Geometry::new(3);
        geometry.set(
            "P",
            Attribute::Vec3(Arc::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y])),
        );
        geometry.set_indices(Some(Arc::new(vec![0, 1, 2])));
        let mesh = geometry_to_mesh(&geometry).unwrap();
        assert_eq!(mesh.count_vertices(), 3);
        assert_eq!(mesh.indices().map(|indices| indices.len()), Some(3));
    }
}
