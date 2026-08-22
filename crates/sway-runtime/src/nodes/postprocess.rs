//! Post-process node kinds: a camera-target in, a camera-target out.
//!
//! **Not scene nodes.** Same closed set as [`Output`](super::output::Output)
//! and [`Capture`](super::capture::Capture): no pose, no children, no
//! `SceneNodeOut`. They sit on the camera-target protocol so
//! `Camera → DepthOfField → ColorGrade → FilmGrain → Output` is ordinary
//! wiring, and `Camera → Output` is still legal.
//!
//! `evaluate` is empty. These nodes do not process images on the tick; the
//! runtime walks the chain after projection and runs fullscreen passes.

use bevy::ecs::world::World;
use bevy::reflect::Reflect;
use bevy::reflect::std_traits::ReflectDefault;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{CameraFeedOut, CameraTarget};

/// Bevy's default focal distance, so a freshly added node is visibly in
/// focus at typical scene scales (`DepthOfField::default`).
const DEFAULT_FOCAL_DISTANCE: f32 = 10.0;
/// Bevy `PhysicalCameraParameters` default f-stops.
const DEFAULT_APERTURE: f32 = 1.0;
/// Visible without a further edit, not so strong it crushes the image.
const DEFAULT_GRAIN_INTENSITY: f32 = 0.1;

/// [`DepthOfField`]'s inlets.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct DepthOfFieldIn {
    pub source: CameraTarget,
    pub focal_distance: f32,
    pub aperture: f32,
}

impl Default for DepthOfFieldIn {
    fn default() -> Self {
        Self {
            source: CameraTarget,
            focal_distance: DEFAULT_FOCAL_DISTANCE,
            aperture: DEFAULT_APERTURE,
        }
    }
}

/// Simulates lens focus on a camera's colour and depth.
///
/// The source MUST be a [`Camera`](super::scene::Camera): a colour-only
/// producer has no depth to sample, and that is reported rather than run.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct DepthOfField {
    pub inlets: DepthOfFieldIn,
    pub state: (),
    pub outlets: CameraFeedOut,
}

impl NodeKind for DepthOfField {
    fn evaluate(&mut self, _world: &World) {}
}

/// [`ColorGrade`]'s inlets. Defaults are identity: adding the node without
/// editing it leaves the source colours unchanged.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct ColorGradeIn {
    pub source: CameraTarget,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
    pub tint: f32,
    pub hue: f32,
}

impl Default for ColorGradeIn {
    fn default() -> Self {
        Self {
            source: CameraTarget,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            temperature: 0.0,
            tint: 0.0,
            hue: 0.0,
        }
    }
}

/// Parametric colour grading of a camera target.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct ColorGrade {
    pub inlets: ColorGradeIn,
    pub state: (),
    pub outlets: CameraFeedOut,
}

impl NodeKind for ColorGrade {
    fn evaluate(&mut self, _world: &World) {}
}

/// [`FilmGrain`]'s inlets.
#[derive(Reflect, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct FilmGrainIn {
    pub source: CameraTarget,
    pub intensity: f32,
}

impl Default for FilmGrainIn {
    fn default() -> Self {
        Self {
            source: CameraTarget,
            intensity: DEFAULT_GRAIN_INTENSITY,
        }
    }
}

/// Luminance film grain over a camera target. Intensity zero is a no-op;
/// the default is visible. Grain phase comes from the show frame clock, not
/// a time inlet.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, Default)]
pub struct FilmGrain {
    pub inlets: FilmGrainIn,
    pub state: (),
    pub outlets: CameraFeedOut,
}

impl NodeKind for FilmGrain {
    fn evaluate(&mut self, _world: &World) {}
}

/// True when this node's value is one of the post-process kinds.
pub fn is_postprocess(value: &dyn bevy::reflect::Reflect) -> bool {
    value.downcast_ref::<DepthOfField>().is_some()
        || value.downcast_ref::<ColorGrade>().is_some()
        || value.downcast_ref::<FilmGrain>().is_some()
}

/// True when this node produces a camera target: a [`Camera`](super::scene::Camera)
/// or a post-process node. The preview picker and the present path both walk
/// this set.
pub fn is_camera_target_producer(value: &dyn bevy::reflect::Reflect) -> bool {
    value.downcast_ref::<super::scene::Camera>().is_some() || is_postprocess(value)
}

/// Toolbar name for a camera-target producer. Effects are not cameras.
pub fn preview_label(value: &dyn bevy::reflect::Reflect) -> Option<&'static str> {
    if value.downcast_ref::<super::scene::Camera>().is_some() {
        Some("Camera")
    } else if value.downcast_ref::<DepthOfField>().is_some() {
        Some("DoF")
    } else if value.downcast_ref::<ColorGrade>().is_some() {
        Some("Grade")
    } else if value.downcast_ref::<FilmGrain>().is_some() {
        Some("Grain")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::output::Output;
    use crate::nodes::protocol;
    use crate::nodes::scene::Camera;
    use bevy::reflect::{Typed, structs::Struct};
    use sway_graph::graph::{ConnectError, Graph, Node, Port};

    fn inlet_names<T: Typed>() -> Vec<&'static str> {
        let bevy::reflect::TypeInfo::Struct(info) = T::type_info() else {
            panic!("{} is a struct", T::type_path());
        };
        info.iter().map(|field| field.name()).collect()
    }

    fn assert_not_a_placement<T: Default + Struct>(inlets: &T) {
        assert!(inlets.field("mesh").is_none(), "draws nothing");
        assert!(inlets.field("material").is_none());
        assert!(
            inlets.field("children").is_none(),
            "not a placement, so nothing sits under it"
        );
        assert!(inlets.field("translation").is_none(), "and it has no pose");
        assert!(inlets.field("rotation").is_none());
        assert!(inlets.field("scale").is_none());
    }

    fn assert_camera_feed_outlets<T: Typed>() {
        let bevy::reflect::TypeInfo::Struct(info) = T::type_info() else {
            panic!("{} is a struct", T::type_path());
        };
        let outlets = info.field("outlets").expect("every node kind has outlets");
        assert_eq!(
            outlets.type_path(),
            "sway_runtime::nodes::protocol::CameraFeedOut"
        );
    }

    #[test]
    fn depth_of_field_declares_source_and_lens_inlets_and_nothing_a_scene_node_has() {
        assert_eq!(
            inlet_names::<DepthOfFieldIn>(),
            vec!["source", "focal_distance", "aperture"]
        );
        assert_not_a_placement(&DepthOfFieldIn::default());
        assert_camera_feed_outlets::<DepthOfField>();
        let d = DepthOfFieldIn::default();
        assert_eq!(d.focal_distance, 10.0);
        assert_eq!(d.aperture, 1.0);
    }

    #[test]
    fn color_grade_defaults_are_identity() {
        assert_eq!(
            inlet_names::<ColorGradeIn>(),
            vec![
                "source",
                "exposure",
                "contrast",
                "saturation",
                "temperature",
                "tint",
                "hue"
            ]
        );
        assert_not_a_placement(&ColorGradeIn::default());
        assert_camera_feed_outlets::<ColorGrade>();
        let g = ColorGradeIn::default();
        assert_eq!(g.exposure, 0.0);
        assert_eq!(g.contrast, 1.0);
        assert_eq!(g.saturation, 1.0);
        assert_eq!(g.temperature, 0.0);
        assert_eq!(g.tint, 0.0);
        assert_eq!(g.hue, 0.0);
    }

    #[test]
    fn film_grain_default_intensity_is_visible() {
        assert_eq!(inlet_names::<FilmGrainIn>(), vec!["source", "intensity"]);
        assert_not_a_placement(&FilmGrainIn::default());
        assert_camera_feed_outlets::<FilmGrain>();
        assert!(FilmGrainIn::default().intensity > 0.0);
    }

    #[test]
    fn a_camera_chains_through_color_grade_to_output() {
        let mut graph = Graph::default();
        let cam = graph.insert(Node::of(Camera::default()));
        let grade = graph.insert(Node::of(ColorGrade::default()));
        let output = graph.insert(Node::of(Output::default()));
        graph
            .connect(
                Port::new(cam, protocol::CAMERA),
                Port::new(grade, protocol::SOURCE),
                0,
            )
            .expect("camera feeds a grade");
        graph
            .connect(
                Port::new(grade, protocol::CAMERA),
                Port::new(output, protocol::CAMERA),
                0,
            )
            .expect("grade feeds output");
    }

    #[test]
    fn a_camera_still_connects_straight_to_output() {
        let mut graph = Graph::default();
        let cam = graph.insert(Node::of(Camera::default()));
        let output = graph.insert(Node::of(Output::default()));
        graph
            .connect(
                Port::new(cam, protocol::CAMERA),
                Port::new(output, protocol::CAMERA),
                0,
            )
            .expect("existing documents keep working");
    }

    #[test]
    fn color_grade_to_depth_of_field_is_type_legal() {
        // The node refuses to *run* this; connect legality is still types.
        let mut graph = Graph::default();
        let grade = graph.insert(Node::of(ColorGrade::default()));
        let dof = graph.insert(Node::of(DepthOfField::default()));
        graph
            .connect(
                Port::new(grade, protocol::CAMERA),
                Port::new(dof, protocol::SOURCE),
                0,
            )
            .expect("types match; the diagnostic is later");
    }

    #[test]
    fn a_mesh_does_not_connect_to_a_postprocess_source() {
        use crate::nodes::mesh::PlaneMesh;
        let mut graph = Graph::default();
        let mesh = graph.insert(Node::of(PlaneMesh::default()));
        let grade = graph.insert(Node::of(ColorGrade::default()));
        let error = graph
            .connect(
                Port::new(mesh, protocol::MESH),
                Port::new(grade, protocol::SOURCE),
                0,
            )
            .expect_err("a mesh is not a camera target");
        assert!(matches!(error, ConnectError::IncompatibleTypes { .. }));
    }

    #[test]
    fn preview_labels_distinguish_effects_from_cameras() {
        assert_eq!(preview_label(&Camera::default()), Some("Camera"));
        assert_eq!(preview_label(&DepthOfField::default()), Some("DoF"));
        assert_eq!(preview_label(&ColorGrade::default()), Some("Grade"));
        assert_eq!(preview_label(&FilmGrain::default()), Some("Grain"));
        assert_eq!(preview_label(&Output::default()), None);
    }
}
