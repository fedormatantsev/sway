//! Geometry attribute tables and the CPU operators over them.
//! Spec: docs/superpowers/specs/2026-08-01-m2b-scene-composition-design.md

pub mod displace;
pub mod geometry;
pub mod grid;

pub use displace::{Displace, DisplaceParams, DisplaceState};
pub use geometry::{Attribute, Geometry};
pub use grid::{Grid, GridParams, GridState};

use bevy_app::{App, Plugin};

/// Registers the CPU geometry operators.
pub struct GeoNodesPlugin;

impl Plugin for GeoNodesPlugin {
    fn build(&self, app: &mut App) {
        sway_graph::register_node_type::<Grid>(app);
        sway_graph::register_node_type::<Displace>(app);
    }
}
