//! Pure CPU grid generation.

use std::sync::Arc;

use bevy_math::{Vec2, Vec3};

use crate::geometry::{Attribute, Geometry};

#[derive(Debug, Clone, Copy)]
pub struct GridParams {
    pub rows: u32,
    pub cols: u32,
    pub width: f32,
    pub height: f32,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            rows: 16,
            cols: 16,
            width: 4.0,
            height: 4.0,
        }
    }
}

pub fn grid(params: GridParams) -> Geometry {
    let rows = params.rows.max(2);
    let cols = params.cols.max(2);
    let count = (rows * cols) as usize;
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut uvs = Vec::with_capacity(count);
    for row in 0..rows {
        for col in 0..cols {
            let u = col as f32 / (cols - 1) as f32;
            let v = row as f32 / (rows - 1) as f32;
            positions.push(Vec3::new(
                (u - 0.5) * params.width,
                0.0,
                (v - 0.5) * params.height,
            ));
            normals.push(Vec3::Y);
            uvs.push(Vec2::new(u, v));
        }
    }
    let mut indices = Vec::with_capacity(((rows - 1) * (cols - 1) * 6) as usize);
    for row in 0..rows - 1 {
        for col in 0..cols - 1 {
            let index = row * cols + col;
            indices.extend_from_slice(&[
                index,
                index + cols,
                index + 1,
                index + 1,
                index + cols,
                index + cols + 1,
            ]);
        }
    }
    let mut geometry = Geometry::new(count);
    geometry.set("P", Attribute::Vec3(Arc::new(positions)));
    geometry.set("N", Attribute::Vec3(Arc::new(normals)));
    geometry.set("uv", Attribute::Vec2(Arc::new(uvs)));
    geometry.set_indices(Some(Arc::new(indices)));
    geometry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_expected_points_and_triangles() {
        let geometry = grid(GridParams {
            rows: 3,
            cols: 4,
            width: 2.0,
            height: 2.0,
        });
        assert_eq!(geometry.point_count(), 12);
        assert_eq!(geometry.indices().map(|indices| indices.len()), Some(36));
    }
}
