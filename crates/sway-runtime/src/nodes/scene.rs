//! The scene nodes — the fixed set that places things in the world.
//!
//! `MeshNode`, `Group`, `Camera`, `DirectionalLight`, `PointLight`, and
//! nothing else (design D7: "the scene node set is closed"). Each owns one
//! entity; none owns an asset.
//!
//! **The duplication is deliberate.** `translation`, `rotation`, `scale` and
//! `children` appear on all five, and there is no shared trait and no base
//! struct for them. A few lines of duplication is cheaper than a supertype,
//! which would be the first crack in a closed set — the moment scene nodes
//! can be assembled from parts, the set is open.
//!
//! **`Group` refuses geometry** by simply not declaring a `mesh` or
//! `material` port. `Graph::connect` then refuses the connection with
//! `ConnectError::MissingDestinationPath`, which is the `nodes` spec's "a
//! group refuses geometry" scenario — enforced by the schema rather than by a
//! rule somewhere else.
//!
//! **Bevy's own names are imported under aliases** (`BevyDirectionalLight`,
//! `BevyPointLight`) so the node kind and the component it projects into are
//! never confused for one another.

use bevy::ecs::world::World;
use bevy::math::{Quat, UVec2, Vec3};
use bevy::prelude::{
    Color, DirectionalLight as BevyDirectionalLight, PointLight as BevyPointLight,
};
use bevy::reflect::Reflect;
use bevy::reflect::std_traits::ReflectDefault;
use bevy::transform::components::Transform;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{
    CameraTargetOut, MeshSource, SceneChild, SceneMaterial, SceneNodeOut,
};

/// A camera's resolution when the author has not said otherwise (design D9).
/// An HDMI show is the target, so this is the useful default rather than a
/// small one.
pub const DEFAULT_CAMERA_RESOLUTION: UVec2 = UVec2::new(1920, 1080);

/// The Bevy `Transform` a scene node projects, assembled from its three pose
/// inlets. A derived `Default` on `Vec3` would zero the scale; identity pose
/// is taken from [`Transform::default`].
pub(crate) fn pose(translation: Vec3, rotation: Quat, scale: Vec3) -> Transform {
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn identity_pose() -> (Vec3, Quat, Vec3) {
    let transform = Transform::default();
    (transform.translation, transform.rotation, transform.scale)
}

/// A colour authored the way a human types one, as sRGB components.
pub(crate) fn to_color(value: Vec3) -> Color {
    Color::srgb(value.x, value.y, value.z)
}

// ---------------------------------------------------------------------------
// MeshNode
// ---------------------------------------------------------------------------

/// [`MeshNode`]'s inlets.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct MeshNodeIn {
    /// Pose is three inlets, not one `Transform`: a `Vec3` outlet connects to
    /// `translation` or `scale`, and the gizmo writes the same fields.
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// The geometry port. A marker inlet: pure schema (design D6). The
    /// projector reads the edge and asks the producer for its handle through
    /// [`protocol::MeshNode`](crate::nodes::protocol::MeshNode); it never
    /// reads this field.
    pub mesh: MeshSource,
    /// The material port. Also pure schema — and non-variadic, so connecting
    /// a second material evicts the first, which is what keeps a scene node
    /// from ever carrying two material kinds at once.
    pub material: SceneMaterial,
    /// The child port. Variadic: many scene nodes may connect here, and the
    /// slot orders them. Pure schema again — the projector reads the edges.
    pub children: Vec<SceneChild>,
}

impl Default for MeshNodeIn {
    fn default() -> Self {
        let (translation, rotation, scale) = identity_pose();
        Self {
            translation,
            rotation,
            scale,
            mesh: MeshSource,
            material: SceneMaterial,
            children: Vec::new(),
        }
    }
}

/// A placement of geometry in the scene. Carries no geometry and no material
/// of its own: a scene node with no material connected renders nothing.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct MeshNode {
    pub inlets: MeshNodeIn,
    pub state: (),
    pub outlets: SceneNodeOut,
}

impl NodeKind for MeshNode {
    fn evaluate(&mut self, _world: &World) {}
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

/// [`Group`]'s inlets: pose and children, and nothing else.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct GroupIn {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub children: Vec<SceneChild>,
}

impl Default for GroupIn {
    fn default() -> Self {
        let (translation, rotation, scale) = identity_pose();
        Self {
            translation,
            rotation,
            scale,
            children: Vec::new(),
        }
    }
}

/// A placement that draws nothing and moves what is connected under it.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Group {
    pub inlets: GroupIn,
    pub state: (),
    pub outlets: SceneNodeOut,
}

impl NodeKind for Group {
    fn evaluate(&mut self, _world: &World) {}
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/// [`Camera`]'s inlets.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct CameraIn {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// The size of the target this camera renders into, in pixels. Authored,
    /// and independent of every window, editor pane and other camera: nothing
    /// outside the graph may change it (`nodes`: "A camera declares the
    /// resolution it renders at").
    ///
    /// A zero component produces no target, which is reported once and renders
    /// nothing rather than falling back to some other size.
    pub resolution: UVec2,
    pub children: Vec<SceneChild>,
}

impl Default for CameraIn {
    fn default() -> Self {
        let (translation, rotation, scale) = identity_pose();
        Self {
            translation,
            rotation,
            scale,
            resolution: DEFAULT_CAMERA_RESOLUTION,
            children: Vec::new(),
        }
    }
}

/// The scene's camera, as opposed to the editor's own. What this node carries
/// is identity — which of the cameras in the world a consumer looks through —
/// and the resolution it renders at.
///
/// Its render target is allocated by
/// [`headless::retarget_cameras`](crate::headless) from `resolution`, and only
/// for a camera something actually consumes; field of view and clear colour
/// stay at Bevy's defaults until something asks otherwise.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct Camera {
    pub inlets: CameraIn,
    pub state: (),
    /// Both outlets: a camera is a placement in the scene *and* something an
    /// output or a capture node can connect to.
    pub outlets: CameraTargetOut,
}

impl NodeKind for Camera {
    fn evaluate(&mut self, _world: &World) {}
}

// ---------------------------------------------------------------------------
// DirectionalLight
// ---------------------------------------------------------------------------

/// [`DirectionalLight`]'s inlets. Defaults are Bevy's own, taken across at
/// construction rather than restated, so they cannot drift.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct DirectionalLightIn {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// sRGB, like every other colour inlet.
    pub color: Vec3,
    pub illuminance: f32,
    pub shadows: bool,
    pub children: Vec<SceneChild>,
}

impl Default for DirectionalLightIn {
    fn default() -> Self {
        let light = BevyDirectionalLight::default();
        let (translation, rotation, scale) = identity_pose();
        Self {
            translation,
            rotation,
            scale,
            // White, stated exactly: round-tripping Bevy's own `Color::WHITE`
            // through sRGB components lands on 0.99999994, which would show
            // up in every hand-written document.
            color: Vec3::ONE,
            illuminance: light.illuminance,
            shadows: light.shadow_maps_enabled,
            children: Vec::new(),
        }
    }
}

/// A directional light in the scene.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct DirectionalLight {
    pub inlets: DirectionalLightIn,
    pub state: (),
    pub outlets: SceneNodeOut,
}

impl NodeKind for DirectionalLight {
    fn evaluate(&mut self, _world: &World) {}
}

impl DirectionalLightIn {
    /// The Bevy component these inlets describe.
    pub fn to_light(&self) -> BevyDirectionalLight {
        BevyDirectionalLight {
            color: to_color(self.color),
            illuminance: self.illuminance,
            shadow_maps_enabled: self.shadows,
            ..BevyDirectionalLight::default()
        }
    }
}

// ---------------------------------------------------------------------------
// PointLight
// ---------------------------------------------------------------------------

/// [`PointLight`]'s inlets. Defaults are Bevy's own.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct PointLightIn {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// sRGB, like every other colour inlet.
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub radius: f32,
    pub shadows: bool,
    pub children: Vec<SceneChild>,
}

impl Default for PointLightIn {
    fn default() -> Self {
        let light = BevyPointLight::default();
        let (translation, rotation, scale) = identity_pose();
        Self {
            translation,
            rotation,
            scale,
            // See `DirectionalLightIn::default`.
            color: Vec3::ONE,
            intensity: light.intensity,
            range: light.range,
            radius: light.radius,
            shadows: light.shadow_maps_enabled,
            children: Vec::new(),
        }
    }
}

/// A point light in the scene.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct PointLight {
    pub inlets: PointLightIn,
    pub state: (),
    pub outlets: SceneNodeOut,
}

impl NodeKind for PointLight {
    fn evaluate(&mut self, _world: &World) {}
}

impl PointLightIn {
    /// The Bevy component these inlets describe.
    pub fn to_light(&self) -> BevyPointLight {
        BevyPointLight {
            color: to_color(self.color),
            intensity: self.intensity,
            range: self.range,
            radius: self.radius,
            shadow_maps_enabled: self.shadows,
            ..BevyPointLight::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_declares_no_geometry_and_no_material_port() {
        // The `nodes` spec's "a group refuses geometry" is enforced by the
        // schema: `Graph::connect` resolves the destination path on the
        // *inlets* value, so a port that does not exist is a refusal. This
        // pins the absence, which is otherwise invisible.
        use bevy::reflect::{Typed, structs::Struct};
        let bevy::reflect::TypeInfo::Struct(info) = GroupIn::type_info() else {
            panic!("GroupIn is a struct");
        };
        let names: Vec<&str> = info.iter().map(|field| field.name()).collect();
        assert_eq!(names, vec!["translation", "rotation", "scale", "children"]);

        // And the same read through a value, which is what `connect` does.
        let group = GroupIn::default();
        assert!(group.field("mesh").is_none());
        assert!(group.field("material").is_none());
    }

    #[test]
    fn every_scene_node_declares_a_children_inlet_and_a_child_outlet() {
        // "Every scene node MUST accept children." A kind that quietly
        // dropped the port would still compile and would simply never be a
        // parent.
        use bevy::reflect::structs::Struct;
        fn has_children(inlets: &dyn Struct) -> bool {
            inlets.field("children").is_some()
        }
        assert!(has_children(&MeshNodeIn::default()));
        assert!(has_children(&GroupIn::default()));
        assert!(has_children(&CameraIn::default()));
        assert!(has_children(&DirectionalLightIn::default()));
        assert!(has_children(&PointLightIn::default()));

        assert_eq!(SceneNodeOut::default().child, SceneChild);
        // A camera's outlets carry the child port too, in a part of their own
        // — it is a scene node as well as a source of frames.
        assert_eq!(CameraTargetOut::default().child, SceneChild);
    }

    #[test]
    fn a_camera_defaults_to_1920_by_1080_and_offers_its_frames() {
        use crate::nodes::protocol::CameraTarget;
        let inlets = CameraIn::default();
        assert_eq!(inlets.resolution, UVec2::new(1920, 1080));
        assert_eq!(Camera::default().outlets.camera, CameraTarget);
    }

    #[test]
    fn adding_the_resolution_inlet_left_the_pose_inlets_alone() {
        // Pose is what the gizmo writes and what a `Vec3` outlet connects to;
        // a resolution field landing on top of one of them would be silent.
        use bevy::reflect::{Typed, structs::Struct};
        let bevy::reflect::TypeInfo::Struct(info) = CameraIn::type_info() else {
            panic!("CameraIn is a struct");
        };
        let names: Vec<&str> = info.iter().map(|field| field.name()).collect();
        assert_eq!(
            names,
            vec!["translation", "rotation", "scale", "resolution", "children"]
        );

        let identity = Transform::default();
        let inlets = CameraIn::default();
        assert_eq!(inlets.translation, identity.translation);
        assert_eq!(inlets.rotation, identity.rotation);
        assert_eq!(inlets.scale, identity.scale);
        assert!(CameraIn::default().field("translation").is_some());
    }

    #[test]
    fn pose_inlets_default_to_identity_not_zero_scale() {
        // A derived `Default` on `Vec3` would zero the scale and collapse
        // every unauthored placement. Identity is Bevy's `Transform::default`.
        let identity = Transform::default();
        let inlets = MeshNodeIn::default();
        assert_eq!(inlets.translation, identity.translation);
        assert_eq!(inlets.rotation, identity.rotation);
        assert_eq!(inlets.scale, identity.scale);
    }

    #[test]
    fn light_defaults_are_bevys_own() {
        assert_eq!(
            DirectionalLightIn::default().illuminance,
            BevyDirectionalLight::default().illuminance
        );
        assert_eq!(
            PointLightIn::default().intensity,
            BevyPointLight::default().intensity
        );
    }
}
