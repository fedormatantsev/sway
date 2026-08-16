//! Context shared by everything the graph runs this tick.

use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::reflect::ReflectComponent;
use bevy_ecs::resource::Resource;
use bevy_math::Vec2;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

/// Where the editor draws this entity, in graph-canvas space.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component, Default, PartialEq)]
pub struct EditorPos(pub Vec2);

/// The entity the editor is currently pointed at.
///
/// One owner for three views: the scene tree, the graph canvas and the
/// viewport all render from this and all write to it through
/// `EditorCommand::Select`. Before M7 the tree and the canvas each held
/// their own answer and reconciled every frame, which is what made a
/// tree-row selection flicker back when the entity had no canvas node.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection(pub Option<Entity>);

/// Marks an entity as implementation detail, not document content: never a
/// scene-tree row, never a node-inspector entry.
///
/// Exists for the transform gizmo (spec M7-8). Bevy's own gizmo renderer
/// (`bevy_gizmos_render::transform_gizmo_render`) spawns ~10 mesh entities
/// carrying `Transform`, which `sway-editor`'s `capture_tree`/`capture_nodes`
/// would otherwise show as fake scene rows. `sway-editor` cannot name the
/// renderer's own marker types (`TransformGizmoRoot`, `TransformGizmoMeshMarker`)
/// to filter them out directly, because that would pull in `bevy_gizmos`,
/// which depends on `bevy_render` — a dependency `sway-editor` must not carry.
/// `sway-runtime`, which already depends on `bevy_gizmos`, tags those entities
/// with this marker instead; `sway-editor`, which cannot see the renderer's
/// types but already depends on `sway-graph`, filters on this one.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct HiddenFromEditor;

/// Context shared by every behaviour run this tick.
pub struct TickCtx {
    /// The fixed timestep, in seconds.
    pub dt: f32,
    /// Absolute start of this tick's window, in seconds.
    pub tick_start: f64,
    /// Monotonically increasing tick counter, starting at 0.
    pub tick_index: u64,
}
