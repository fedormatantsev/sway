//! Consumed post-process passes: what to run, on which targets, this frame.
//!
//! Graph nodes do not process images on the tick. After targets are
//! allocated this pass walks every consumed camera-target chain and publishes
//! one [`EffectPass`] per effect node that can produce, in an order that
//! respects the chain. The GPU plugin in [`super::effect_gpu`] runs them.

use bevy::camera::ManualTextureViewHandle;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use sway_graph::graph::{Graph, NodeId};

use crate::nodes::postprocess::{ColorGrade, DepthOfField, FilmGrain, is_postprocess};
use crate::nodes::protocol;
use crate::nodes::scene::Camera;
use crate::project::cameras::{CameraTargets, producing_chain, source_camera};
use crate::project::{NodeEntities, source_of};

/// How many show frames have been rendered. Grain hashes this, not the tick.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowFrame(pub u32);

/// One fullscreen pass from a source target onto an effect node's target.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectPass {
    pub node: NodeId,
    /// The scene camera whose `Core3d` pass this effect follows.
    pub camera: NodeId,
    pub source: ManualTextureViewHandle,
    pub dest: ManualTextureViewHandle,
    pub kind: EffectKind,
    /// The source is the scene camera's own target. During `PostProcess` the
    /// 3D colour lives on `ViewTarget`; the camera `ManualTextureView` is only
    /// filled at upscaling.
    pub from_camera: bool,
}

/// Uniforms for one [`EffectPass`], packed from the node's inlets.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectKind {
    DepthOfField {
        focal_distance: f32,
        aperture: f32,
    },
    ColorGrade {
        exposure: f32,
        contrast: f32,
        saturation: f32,
        temperature: f32,
        tint: f32,
        hue: f32,
    },
    FilmGrain {
        intensity: f32,
    },
}

/// Every consumed effect pass this frame, in execution order, plus the show
/// frame index grain hashes.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct EffectPasses {
    pub passes: Vec<EffectPass>,
    pub frame: u32,
}

/// Rebuilds [`EffectPasses`] from the graph and the allocated targets.
pub fn publish_effect_passes(
    graph: Res<Graph>,
    targets: Res<CameraTargets>,
    frame: Res<ShowFrame>,
    mut passes: ResMut<EffectPasses>,
) {
    let mut dests: Vec<NodeId> = Vec::new();
    for (id, node) in graph.iter() {
        let consumes = node
            .value()
            .downcast_ref::<crate::nodes::output::Output>()
            .is_some()
            || node
                .value()
                .downcast_ref::<crate::nodes::capture::Capture>()
                .is_some();
        if !consumes {
            continue;
        }
        let Some(producer) = source_of(&graph, id, protocol::CAMERA) else {
            continue;
        };
        collect_effect_dests(&graph, producer, &mut dests);
    }
    // Preview is a consumer too: producing_chain of the previewed node is
    // already in `CameraTargets` once allocated, so walk every allocated
    // effect node as well — that covers preview-only chains.
    for (id, node) in graph.iter() {
        if is_postprocess(node.value()) && targets.handle(id).is_some() {
            collect_effect_dests(&graph, id, &mut dests);
        }
    }
    dests.sort_unstable();
    dests.dedup();

    dests.sort_by_key(|id| chain_depth(&graph, *id));

    let mut next = Vec::new();
    for dest in dests {
        let Some(source_node) = source_of(&graph, dest, protocol::SOURCE) else {
            continue;
        };
        let Some(source) = targets.handle(source_node) else {
            continue;
        };
        let Some(dest_handle) = targets.handle(dest) else {
            continue;
        };
        let Some(kind) = effect_kind(&graph, dest) else {
            continue;
        };
        let Some(camera) = source_camera(&graph, dest) else {
            continue;
        };
        next.push(EffectPass {
            node: dest,
            camera,
            source,
            dest: dest_handle,
            kind,
            from_camera: source_node == camera,
        });
    }

    let next_resource = EffectPasses {
        passes: next,
        frame: frame.0,
    };
    if *passes != next_resource {
        *passes = next_resource;
    }
}

fn collect_effect_dests(graph: &Graph, producer: NodeId, dests: &mut Vec<NodeId>) {
    let Some(chain) = producing_chain(graph, producer) else {
        return;
    };
    for node in chain {
        if graph.get(node).is_some_and(|n| is_postprocess(n.value())) {
            dests.push(node);
        }
    }
}

fn chain_depth(graph: &Graph, mut node: NodeId) -> u32 {
    let mut depth = 0;
    let mut seen = bevy::platform::collections::HashSet::<NodeId>::new();
    while seen.insert(node) {
        if graph
            .get(node)
            .is_some_and(|n| n.value().downcast_ref::<Camera>().is_some())
        {
            return depth;
        }
        match source_of(graph, node, protocol::SOURCE) {
            Some(src) => {
                depth += 1;
                node = src;
            }
            None => return depth,
        }
    }
    depth
}

fn effect_kind(graph: &Graph, node: NodeId) -> Option<EffectKind> {
    let value = graph.get(node)?.value();
    if let Some(dof) = value.downcast_ref::<DepthOfField>() {
        return Some(EffectKind::DepthOfField {
            focal_distance: dof.inlets.focal_distance,
            aperture: dof.inlets.aperture,
        });
    }
    if let Some(grade) = value.downcast_ref::<ColorGrade>() {
        return Some(EffectKind::ColorGrade {
            exposure: grade.inlets.exposure,
            contrast: grade.inlets.contrast,
            saturation: grade.inlets.saturation,
            temperature: grade.inlets.temperature,
            tint: grade.inlets.tint,
            hue: grade.inlets.hue,
        });
    }
    if let Some(grain) = value.downcast_ref::<FilmGrain>() {
        return Some(EffectKind::FilmGrain {
            intensity: grain.inlets.intensity,
        });
    }
    None
}

/// Increments [`ShowFrame`] once per rendered frame of the show.
pub fn tick_show_frame(mut frame: ResMut<ShowFrame>) {
    frame.0 = frame.0.wrapping_add(1);
}

/// Adds [`DepthPrepass`] on a camera that feeds a consumed `DepthOfField`,
/// and takes it off otherwise.
///
/// Bevy's default [`Msaa`] is 4x, and `Camera3d`'s depth target is
/// render-only. Sampling that as `texture_depth_2d` is a wgpu validation
/// error. DoF cameras are therefore single-sample with a sampleable depth
/// texture (design: scene cameras stay single-sample; Bevy's own DoF plugin
/// sets `TEXTURE_BINDING` the same way).
pub fn sync_depth_prepass(
    graph: Res<Graph>,
    targets: Option<Res<CameraTargets>>,
    map: Res<NodeEntities>,
    mut commands: Commands,
    mut cameras: Query<(&mut Camera3d, Option<&DepthPrepass>, Option<&Msaa>)>,
) {
    let Some(targets) = targets else {
        return;
    };
    let mut want: bevy::platform::collections::HashSet<NodeId> =
        bevy::platform::collections::HashSet::new();
    for (id, node) in graph.iter() {
        if node.value().downcast_ref::<DepthOfField>().is_none() {
            continue;
        }
        if targets.handle(id).is_none() {
            continue;
        }
        if let Some(src) = source_of(&graph, id, protocol::SOURCE)
            && graph
                .get(src)
                .is_some_and(|n| n.value().downcast_ref::<Camera>().is_some())
        {
            want.insert(src);
        }
    }

    let sampleable = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;

    for (id, _) in graph.iter() {
        let Some(entity) = map.entity(id) else {
            continue;
        };
        let Ok((mut camera_3d, prepass, msaa)) = cameras.get_mut(entity) else {
            continue;
        };
        let should = want.contains(&id);
        if should {
            if prepass.is_none() {
                commands.entity(entity).insert(DepthPrepass);
            }
            if msaa != Some(&Msaa::Off) {
                commands.entity(entity).insert(Msaa::Off);
            }
            let usages = TextureUsages::from(camera_3d.depth_texture_usages);
            if !usages.contains(TextureUsages::TEXTURE_BINDING) {
                camera_3d.depth_texture_usages = sampleable.into();
            }
        } else if prepass.is_some() {
            commands.entity(entity).remove::<DepthPrepass>();
            if msaa == Some(&Msaa::Off) {
                commands.entity(entity).insert(Msaa::Sample4);
            }
            let usages = TextureUsages::from(camera_3d.depth_texture_usages);
            if usages.contains(TextureUsages::TEXTURE_BINDING) {
                camera_3d.depth_texture_usages = TextureUsages::RENDER_ATTACHMENT.into();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::output::Output;
    use crate::nodes::scene::CameraIn;
    use crate::project::cameras::CameraTargets;
    use bevy::render::renderer::RenderDevice;
    use bevy::render::texture::ManualTextureViews;
    use sway_graph::graph::{Node, Port};

    fn camera(resolution: UVec2) -> Camera {
        Camera {
            inlets: CameraIn {
                resolution,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn app() -> App {
        let gpu = sway_gpu::GpuContext::new(None);
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ));
        app.init_asset::<Mesh>();
        app.init_asset::<Image>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<crate::nodes::sprite_material::SpriteMaterialAsset>();
        app.add_plugins(crate::project::RuntimePlugin);
        app.insert_resource(RenderDevice::from(gpu.device.clone()))
            .init_resource::<ManualTextureViews>();
        app
    }

    fn insert<T: Reflect + TypePath>(app: &mut App, value: T) -> NodeId {
        app.world_mut()
            .resource_mut::<Graph>()
            .insert(Node::of(value))
    }

    fn connect(app: &mut App, from: NodeId, to: NodeId, src: &str, dst: &str) {
        app.world_mut()
            .resource_mut::<Graph>()
            .connect(Port::new(from, src), Port::new(to, dst), 0)
            .expect("legal");
    }

    #[test]
    fn a_grade_on_the_output_schedules_one_pass_onto_the_grade_target() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(1920, 1080)));
        let grade = insert(&mut app, ColorGrade::default());
        let output = insert(&mut app, Output::default());
        connect(&mut app, cam, grade, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, grade, output, protocol::CAMERA, protocol::CAMERA);
        app.update();

        let targets = app.world().resource::<CameraTargets>();
        let cam_h = targets.handle(cam).expect("camera");
        let grade_h = targets.handle(grade).expect("grade");
        let passes = &app.world().resource::<EffectPasses>().passes;
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].node, grade);
        assert_eq!(passes[0].camera, cam);
        assert_eq!(passes[0].source, cam_h);
        assert_eq!(passes[0].dest, grade_h);
        assert!(
            passes[0].from_camera,
            "a grade on the camera must sample ViewTarget, not the upscale dest"
        );
        assert!(matches!(
            passes[0].kind,
            EffectKind::ColorGrade {
                exposure: 0.0,
                contrast: 1.0,
                saturation: 1.0,
                ..
            }
        ));
    }

    #[test]
    fn a_chain_schedules_dof_then_grade_then_grain() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(800, 600)));
        let dof = insert(&mut app, DepthOfField::default());
        let grade = insert(&mut app, ColorGrade::default());
        let grain = insert(&mut app, FilmGrain::default());
        let output = insert(&mut app, Output::default());
        connect(&mut app, cam, dof, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, dof, grade, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, grade, grain, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, grain, output, protocol::CAMERA, protocol::CAMERA);
        app.update();

        let nodes: Vec<NodeId> = app
            .world()
            .resource::<EffectPasses>()
            .passes
            .iter()
            .map(|p| p.node)
            .collect();
        assert_eq!(nodes, vec![dof, grade, grain]);
        let passes = &app.world().resource::<EffectPasses>().passes;
        assert!(matches!(passes[0].kind, EffectKind::DepthOfField { .. }));
        assert!(passes[0].from_camera, "DoF reads the camera ViewTarget");
        assert!(
            !passes[1].from_camera && !passes[2].from_camera,
            "later effects read the previous node's ManualTextureView"
        );
        assert!(matches!(
            passes[2].kind,
            EffectKind::FilmGrain { intensity } if intensity > 0.0
        ));
    }

    #[test]
    fn zero_grain_is_still_scheduled_so_the_pass_can_copy() {
        let mut app = app();
        let cam = insert(&mut app, camera(UVec2::new(64, 64)));
        let grain = insert(
            &mut app,
            FilmGrain {
                inlets: crate::nodes::postprocess::FilmGrainIn {
                    intensity: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let output = insert(&mut app, Output::default());
        connect(&mut app, cam, grain, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, grain, output, protocol::CAMERA, protocol::CAMERA);
        app.update();
        let kind = &app.world().resource::<EffectPasses>().passes[0].kind;
        assert!(matches!(kind, EffectKind::FilmGrain { intensity: 0.0 }));
    }

    #[test]
    fn show_frame_increments_once_per_update() {
        let mut app = app();
        assert_eq!(app.world().resource::<ShowFrame>().0, 0);
        app.update();
        assert_eq!(app.world().resource::<ShowFrame>().0, 1);
        app.update();
        assert_eq!(app.world().resource::<ShowFrame>().0, 2);
    }

    #[test]
    fn depth_prepass_is_only_on_a_camera_feeding_consumed_dof() {
        let mut app = app();
        let cam_dof = insert(&mut app, camera(UVec2::new(128, 128)));
        let cam_plain = insert(&mut app, camera(UVec2::new(128, 128)));
        let dof = insert(&mut app, DepthOfField::default());
        let output = insert(&mut app, Output::default());
        let capture = insert(&mut app, crate::nodes::capture::Capture::default());
        connect(&mut app, cam_dof, dof, protocol::CAMERA, protocol::SOURCE);
        connect(&mut app, dof, output, protocol::CAMERA, protocol::CAMERA);
        connect(
            &mut app,
            cam_plain,
            capture,
            protocol::CAMERA,
            protocol::CAMERA,
        );
        app.update();

        let map = app.world().resource::<NodeEntities>();
        let dof_entity = map.entity(cam_dof).expect("projected");
        let plain_entity = map.entity(cam_plain).expect("projected");
        assert!(app.world().get::<DepthPrepass>(dof_entity).is_some());
        assert!(app.world().get::<DepthPrepass>(plain_entity).is_none());
        assert_eq!(
            app.world().get::<Msaa>(dof_entity),
            Some(&Msaa::Off),
            "DoF samples depth as texture_depth_2d; MSAA depth is a different binding"
        );
        let usages = bevy::render::render_resource::TextureUsages::from(
            app.world()
                .get::<Camera3d>(dof_entity)
                .expect("projected camera")
                .depth_texture_usages,
        );
        assert!(
            usages.contains(bevy::render::render_resource::TextureUsages::TEXTURE_BINDING),
            "ViewDepthTexture must be sampleable for the DoF pass"
        );
        assert_ne!(
            app.world().get::<Msaa>(plain_entity),
            Some(&Msaa::Off),
            "a camera that is not feeding DoF keeps Bevy's default MSAA"
        );
        assert!(
            app.world()
                .get::<bevy::post_process::dof::DepthOfField>(dof_entity)
                .is_none(),
            "Bevy's DepthOfField component must not land on the scene camera"
        );
    }
}
