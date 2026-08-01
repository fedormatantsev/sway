//! `Grid` — a CPU geometry source. Design §8.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_math::{Vec2, Vec3};
use bevy_reflect::Reflect;
use sway_graph::{NoOutputs, NoSlots, NodeType, PortView, SlotView, TickCtx};

use crate::geometry::{Attribute, Geometry};

#[derive(Reflect, Component)]
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

#[derive(Component, Default)]
pub struct GridState;

pub struct Grid;

impl Grid {
    pub const ROWS: u16 = 0;
    pub const COLS: u16 = 1;
    pub const WIDTH: u16 = 2;
    pub const HEIGHT: u16 = 3;
}

impl NodeType for Grid {
    type Params = GridParams;
    type Outputs = NoOutputs;
    type Slots = NoSlots;
    type Produces = Geometry;
    type State = GridState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("rows", Self::ROWS),
        ("cols", Self::COLS),
        ("width", Self::WIDTH),
        ("height", Self::HEIGHT),
    ];
    const COOKS: bool = true;

    fn register(_app: &mut App) {}

    /// Nothing per-tick: `Grid`'s whole product is its cook.
    fn tick(_world: &mut World, _node: Entity, _ports: &mut PortView, _t: &TickCtx) {}

    fn cook(world: &mut World, node: Entity, _slots: &SlotView) {
        let params = match world.get::<GridParams>(node) {
            Some(p) => (p.rows.max(2), p.cols.max(2), p.width, p.height),
            None => return,
        };
        let (rows, cols, width, height) = params;

        let count = (rows * cols) as usize;
        let mut positions = Vec::with_capacity(count);
        let mut normals = Vec::with_capacity(count);
        let mut uvs = Vec::with_capacity(count);
        for r in 0..rows {
            for c in 0..cols {
                let u = c as f32 / (cols - 1) as f32;
                let v = r as f32 / (rows - 1) as f32;
                positions.push(Vec3::new(
                    (u - 0.5) * width,
                    0.0,
                    (v - 0.5) * height,
                ));
                normals.push(Vec3::Y);
                uvs.push(Vec2::new(u, v));
            }
        }

        let mut indices = Vec::with_capacity(((rows - 1) * (cols - 1) * 6) as usize);
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                let i = r * cols + c;
                indices.extend_from_slice(&[i, i + cols, i + 1]);
                indices.extend_from_slice(&[i + 1, i + cols, i + cols + 1]);
            }
        }

        let mut geo = Geometry::new(count);
        geo.set("P", Attribute::Vec3(Arc::new(positions)));
        geo.set("N", Attribute::Vec3(Arc::new(normals)));
        geo.set("uv", Attribute::Vec2(Arc::new(uvs)));
        geo.set_indices(Some(Arc::new(indices)));
        world.entity_mut(node).insert(geo);
    }

    fn produced_change_tick(world: &World, node: Entity) -> Option<Tick> {
        world
            .get_entity(node)
            .ok()?
            .get_change_ticks::<Geometry>()
            .map(|t| t.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cooked_grid(rows: u32, cols: u32) -> Geometry {
        let mut world = World::new();
        let node = world
            .spawn((
                GridParams {
                    rows,
                    cols,
                    width: 2.0,
                    height: 2.0,
                },
                GridState,
            ))
            .id();
        Grid::cook(&mut world, node, &SlotView::new(&[]));
        world.get::<Geometry>(node).cloned().expect("Grid cooks a Geometry")
    }

    #[test]
    fn a_grid_has_rows_times_cols_points() {
        let g = cooked_grid(3, 4);
        assert_eq!(g.point_count(), 12);
        assert_eq!(g.get("P").map(|a| a.len()), Some(12));
        assert_eq!(g.get("N").map(|a| a.len()), Some(12));
        assert_eq!(g.get("uv").map(|a| a.len()), Some(12));
    }

    #[test]
    fn a_grid_spans_its_width_and_height_centred_on_the_origin() {
        let g = cooked_grid(2, 2);
        let p = g.get("P").and_then(|a| a.as_vec3()).expect("P is Vec3");
        assert_eq!(p[0], Vec3::new(-1.0, 0.0, -1.0));
        assert_eq!(p[3], Vec3::new(1.0, 0.0, 1.0));
    }

    #[test]
    fn a_grid_emits_two_triangles_per_cell() {
        let g = cooked_grid(3, 3);
        // 2x2 cells, 2 triangles each, 3 indices each.
        assert_eq!(g.indices().map(|i| i.len()), Some(24));
    }

    #[test]
    fn a_cook_is_a_pure_function_of_its_params() {
        assert_eq!(
            cooked_grid(3, 3).get("P").and_then(|a| a.as_vec3()).cloned(),
            cooked_grid(3, 3).get("P").and_then(|a| a.as_vec3()).cloned()
        );
    }
}
