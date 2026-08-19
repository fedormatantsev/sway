//! Residual types for leftover widget fields that still name the old wire
//! identity. The presenter reads [`crate::EditorUi::apply_graph`] (design D11);
//! these exist only so the canvas/tree/inspector compile while those fields
//! are unused.

use bevy_ecs::entity::Entity;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct InletView {
    pub wire: &'static str,
    pub connected: bool,
    pub accepts_from: Vec<Entity>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Float,
    Int,
    Bool,
    Enum(Vec<String>),
    Str,
    Vec2,
    Vec3,
    Opaque,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum TreeGroup {
    Scene,
    Graph,
    Edges,
    Other,
}
