//! Geometry attribute tables and pure CPU operators.

pub mod displace;
pub mod geometry;
pub mod grid;

pub use displace::displace;
pub use geometry::{Attribute, Geometry};
pub use grid::{GridParams, grid};
