//! Residual types for leftover widget fields that still name the old wire
//! identity. The presenter reads [`crate::EditorUi::apply_graph`]; these exist
//! only so the canvas/tree compile while those fields drain.

use bevy_ecs::entity::Entity;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct InletView {
    pub wire: &'static str,
    pub connected: bool,
    pub accepts_from: Vec<Entity>,
}
