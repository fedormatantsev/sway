//! Fullscreen effect passes in Bevy's `Core3d` graph.
//!
//! Camera-sourced passes sample the camera's [`ViewTarget`] (the 3D colour
//! lives there during `PostProcess`; the camera [`ManualTextureView`] is only
//! filled at upscaling). Later chain nodes sample the previous effect's
//! destination. Destinations are always the effect node's own target — never
//! a `ViewTarget` ping-pong, which would overwrite the camera's published
//! feed. The Bevy `DepthOfField` / `ColorGrading` components are never
//! attached to the scene camera.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::camera::ManualTextureViewHandle;
use bevy::core_pipeline::{Core3dSystems, FullscreenShader, schedule::Core3d};
use bevy::prelude::*;
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferInitDescriptor,
    BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState,
    Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    ShaderType, TextureFormat, TextureSampleType,
    binding_types::{sampler, texture_2d, texture_depth_2d, uniform_buffer},
    encase::UniformBuffer as EncaseUniform,
};
use bevy::render::renderer::{CurrentView, RenderContext, RenderDevice};
use bevy::render::sync_world::RenderEntity;
use bevy::render::texture::ManualTextureViews;
use bevy::render::view::{
    ColorGrading, ColorGradingGlobal, ColorGradingSection, ColorGradingUniform, ViewDepthTexture,
    ViewTarget,
};
use bevy::render::{Extract, ExtractSchedule, RenderApp, RenderStartup};

use bevy::render::render_resource::encase::internal::WriteInto;

use crate::project::NodeEntities;
use crate::project::effects::{EffectKind, EffectPasses};

macro_rules! grade_shader_path {
    () => {
        "../../assets/shaders/color_grade.wgsl"
    };
}
macro_rules! grain_shader_path {
    () => {
        "../../assets/shaders/film_grain.wgsl"
    };
}
macro_rules! dof_shader_path {
    () => {
        "../../assets/shaders/depth_of_field.wgsl"
    };
}

/// Super 35 sensor height, matching Bevy `PhysicalCameraParameters::default`.
const SENSOR_HEIGHT: f32 = 0.01866;
/// Bevy `PerspectiveProjection::default` near plane, for reverse-Z view Z.
const DEFAULT_NEAR: f32 = 0.1;
/// Bevy `DepthOfField::default` cap on blur diameter, in pixels.
const MAX_COC: f32 = 64.0;

pub struct EffectGpuPlugin;

impl Plugin for EffectGpuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, grade_shader_path!());
        embedded_asset!(app, grain_shader_path!());
        embedded_asset!(app, dof_shader_path!());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExtractedEffects>()
            .add_systems(ExtractSchedule, extract_effects)
            .add_systems(RenderStartup, init_effect_pipelines)
            .add_systems(Core3d, run_effect_passes.in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Clone, Copy, ShaderType)]
struct GradeParams {
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    hue: f32,
    _pad0: f32,
    _pad1: f32,
}

#[derive(Clone, Copy, ShaderType)]
struct GrainParams {
    intensity: f32,
    frame: f32,
    _pad0: f32,
    _pad1: f32,
}

#[derive(Clone, Copy, ShaderType)]
struct DofParams {
    focal_distance: f32,
    focal_length: f32,
    coc_scale: f32,
    max_coc: f32,
    near: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

#[derive(Clone)]
struct ExtractedPass {
    camera_entity: Entity,
    source: ManualTextureViewHandle,
    dest: ManualTextureViewHandle,
    kind: EffectKind,
    from_camera: bool,
}

#[derive(Resource, Default, Clone)]
struct ExtractedEffects {
    passes: Vec<ExtractedPass>,
    frame: u32,
}

fn extract_effects(
    mut commands: Commands,
    passes: Extract<Res<EffectPasses>>,
    nodes: Extract<Res<NodeEntities>>,
    render_entities: Extract<Query<&RenderEntity>>,
    existing: Option<ResMut<ExtractedEffects>>,
) {
    let extracted = ExtractedEffects {
        passes: passes
            .passes
            .iter()
            .filter_map(|pass| {
                let main = nodes.entity(pass.camera)?;
                let render = render_entities.get(main).ok()?.id();
                Some(ExtractedPass {
                    camera_entity: render,
                    source: pass.source,
                    dest: pass.dest,
                    kind: pass.kind.clone(),
                    from_camera: pass.from_camera,
                })
            })
            .collect(),
        frame: passes.frame,
    };
    if let Some(mut existing) = existing {
        *existing = extracted;
    } else {
        commands.insert_resource(extracted);
    }
}

#[derive(Resource)]
struct EffectPipelines {
    color_layout: BindGroupLayoutDescriptor,
    grain_layout: BindGroupLayoutDescriptor,
    dof_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    color: CachedRenderPipelineId,
    grain: CachedRenderPipelineId,
    dof: CachedRenderPipelineId,
}

fn init_effect_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    pipeline_cache: Res<PipelineCache>,
) {
    let color_layout = BindGroupLayoutDescriptor::new(
        "sway_color_grade_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<GradeParams>(false),
            ),
        ),
    );
    let grain_layout = BindGroupLayoutDescriptor::new(
        "sway_film_grain_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<GrainParams>(false),
            ),
        ),
    );
    let dof_layout = BindGroupLayoutDescriptor::new(
        "sway_depth_of_field_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_depth_2d(),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<DofParams>(false),
            ),
        ),
    );

    let sampler = render_device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    let vertex = fullscreen_shader.to_vertex_state();

    let color = queue_fullscreen_pipeline(
        &pipeline_cache,
        &color_layout,
        vertex.clone(),
        asset_server.load(
            AssetPath::from_path_buf(embedded_path!(grade_shader_path!())).with_source("embedded"),
        ),
        "sway_color_grade",
    );
    let grain = queue_fullscreen_pipeline(
        &pipeline_cache,
        &grain_layout,
        vertex.clone(),
        asset_server.load(
            AssetPath::from_path_buf(embedded_path!(grain_shader_path!())).with_source("embedded"),
        ),
        "sway_film_grain",
    );
    let dof = queue_fullscreen_pipeline(
        &pipeline_cache,
        &dof_layout,
        vertex,
        asset_server.load(
            AssetPath::from_path_buf(embedded_path!(dof_shader_path!())).with_source("embedded"),
        ),
        "sway_depth_of_field",
    );

    commands.insert_resource(EffectPipelines {
        color_layout,
        grain_layout,
        dof_layout,
        sampler,
        color,
        grain,
        dof,
    });
}

fn queue_fullscreen_pipeline(
    cache: &PipelineCache,
    layout: &BindGroupLayoutDescriptor,
    vertex: bevy::render::render_resource::VertexState,
    shader: Handle<Shader>,
    label: &'static str,
) -> CachedRenderPipelineId {
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some(label.into()),
        layout: vec![layout.clone()],
        vertex,
        fragment: Some(FragmentState {
            shader,
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::Rgba8UnormSrgb,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    })
}

/// Packs the six colour-grade inlets into Bevy's [`ColorGradingUniform`].
///
/// Exposure / temperature / tint / hue / post-saturation come from
/// [`ColorGradingGlobal`]; contrast is applied to every section;
/// lift / gamma / gain stay at identity (design D3).
pub fn pack_color_grading(
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    hue: f32,
) -> ColorGradingUniform {
    ColorGrading::with_identical_sections(
        ColorGradingGlobal {
            exposure,
            temperature,
            tint,
            hue,
            post_saturation: saturation,
            ..Default::default()
        },
        ColorGradingSection {
            contrast,
            ..Default::default()
        },
    )
    .into()
}

fn encode_uniform<T: ShaderType + WriteInto>(value: &T) -> Vec<u8> {
    let mut buffer = EncaseUniform::new(Vec::new());
    buffer.write(value).expect("uniform encodes");
    buffer.into_inner()
}

fn grade_params(kind: &EffectKind) -> Option<GradeParams> {
    let EffectKind::ColorGrade {
        exposure,
        contrast,
        saturation,
        temperature,
        tint,
        hue,
    } = kind
    else {
        return None;
    };
    let packed = pack_color_grading(*exposure, *contrast, *saturation, *temperature, *tint, *hue);
    Some(GradeParams {
        exposure: packed.exposure,
        contrast: packed.contrast.x,
        saturation: packed.post_saturation,
        temperature: *temperature,
        tint: *tint,
        hue: packed.hue,
        _pad0: 0.0,
        _pad1: 0.0,
    })
}

fn dof_params(focal_distance: f32, aperture: f32) -> DofParams {
    // Same CoC scale Bevy uses, so Gaussian blur tracks aperture the way
    // `dof.wgsl` would. Focal length from Super 35 + default 45° FOV.
    let fov = std::f32::consts::FRAC_PI_4;
    let focal_length = 0.5 * SENSOR_HEIGHT / (0.5 * fov).tan();
    let coc_scale = focal_length * focal_length / (SENSOR_HEIGHT * aperture.max(0.001));
    DofParams {
        focal_distance,
        focal_length,
        coc_scale,
        max_coc: MAX_COC,
        near: DEFAULT_NEAR,
        _pad0: 0.0,
        _pad1: 0.0,
        _pad2: 0.0,
    }
}

/// Circle of confusion in pixels. Keep in lockstep with `depth_of_field.wgsl`.
fn circle_of_confusion_px(view_z: f32, framebuffer_height: f32, params: &DofParams) -> f32 {
    let candidate = params.coc_scale * (view_z - params.focal_distance).abs()
        / (view_z.max(1.0e-6) * (params.focal_distance - params.focal_length).max(1.0e-7));
    (candidate * framebuffer_height).clamp(0.0, params.max_coc)
}

fn run_effect_passes(
    view: Res<CurrentView>,
    extracted: Res<ExtractedEffects>,
    pipelines: Option<Res<EffectPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    views: Res<ManualTextureViews>,
    depth: Query<&ViewDepthTexture>,
    view_targets: Query<&ViewTarget>,
    mut ctx: RenderContext,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    let depth_view = depth.get(view.0).ok();
    let view_target = view_targets.get(view.0).ok();

    for pass in extracted
        .passes
        .iter()
        .filter(|pass| pass.camera_entity == view.0)
    {
        let Some(dest) = views.get(&pass.dest) else {
            continue;
        };
        let color = if pass.from_camera {
            match view_target {
                Some(vt) => vt.main_texture_view(),
                None => continue,
            }
        } else {
            match views.get(&pass.source) {
                Some(source) => &source.texture_view,
                None => continue,
            }
        };
        match &pass.kind {
            EffectKind::ColorGrade { .. } => {
                let Some(pipeline) = pipeline_cache.get_render_pipeline(pipelines.color) else {
                    continue;
                };
                let Some(params) = grade_params(&pass.kind) else {
                    continue;
                };
                let bytes = encode_uniform(&params);
                let buffer = ctx
                    .render_device()
                    .create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("sway color grade uniform"),
                        contents: &bytes,
                        usage: BufferUsages::UNIFORM,
                    });
                let bind_group = ctx.render_device().create_bind_group(
                    "sway color grade",
                    &pipeline_cache.get_bind_group_layout(&pipelines.color_layout),
                    &BindGroupEntries::sequential((
                        color,
                        &pipelines.sampler,
                        buffer.as_entire_binding(),
                    )),
                );
                draw_fullscreen(&mut ctx, pipeline, &bind_group, &dest.texture_view);
            }
            EffectKind::FilmGrain { intensity } => {
                let Some(pipeline) = pipeline_cache.get_render_pipeline(pipelines.grain) else {
                    continue;
                };
                let params = GrainParams {
                    intensity: *intensity,
                    frame: extracted.frame as f32,
                    _pad0: 0.0,
                    _pad1: 0.0,
                };
                let bytes = encode_uniform(&params);
                let buffer = ctx
                    .render_device()
                    .create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("sway film grain uniform"),
                        contents: &bytes,
                        usage: BufferUsages::UNIFORM,
                    });
                let bind_group = ctx.render_device().create_bind_group(
                    "sway film grain",
                    &pipeline_cache.get_bind_group_layout(&pipelines.grain_layout),
                    &BindGroupEntries::sequential((
                        color,
                        &pipelines.sampler,
                        buffer.as_entire_binding(),
                    )),
                );
                draw_fullscreen(&mut ctx, pipeline, &bind_group, &dest.texture_view);
            }
            EffectKind::DepthOfField {
                focal_distance,
                aperture,
            } => {
                let Some(pipeline) = pipeline_cache.get_render_pipeline(pipelines.dof) else {
                    continue;
                };
                let Some(depth_tex) = depth_view else {
                    continue;
                };
                let params = dof_params(*focal_distance, *aperture);
                let bytes = encode_uniform(&params);
                let buffer = ctx
                    .render_device()
                    .create_buffer_with_data(&BufferInitDescriptor {
                        label: Some("sway dof uniform"),
                        contents: &bytes,
                        usage: BufferUsages::UNIFORM,
                    });
                let bind_group = ctx.render_device().create_bind_group(
                    "sway depth of field",
                    &pipeline_cache.get_bind_group_layout(&pipelines.dof_layout),
                    &BindGroupEntries::sequential((
                        color,
                        depth_tex.view(),
                        &pipelines.sampler,
                        buffer.as_entire_binding(),
                    )),
                );
                draw_fullscreen(&mut ctx, pipeline, &bind_group, &dest.texture_view);
            }
        }
    }
}

fn draw_fullscreen(
    ctx: &mut RenderContext,
    pipeline: &bevy::render::render_resource::RenderPipeline,
    bind_group: &bevy::render::render_resource::BindGroup,
    dest: &bevy::render::render_resource::TextureView,
) {
    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("sway effect pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: dest,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    render_pass.set_pipeline(pipeline);
    render_pass.set_bind_group(0, bind_group, &[]);
    render_pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_color_grade_packs_a_noop_uniform() {
        let packed = pack_color_grading(0.0, 1.0, 1.0, 0.0, 0.0, 0.0);
        assert_eq!(packed.exposure, 0.0);
        assert_eq!(packed.hue, 0.0);
        assert_eq!(packed.post_saturation, 1.0);
        assert_eq!(packed.contrast, Vec3::ONE);
        assert_eq!(packed.saturation, Vec3::ONE);
        assert_eq!(packed.gamma, Vec3::ONE);
        assert_eq!(packed.gain, Vec3::ONE);
        assert_eq!(packed.lift, Vec3::ZERO);
    }

    #[test]
    fn a_metre_in_front_of_the_focal_plane_has_a_visible_circle_of_confusion() {
        let p = dof_params(10.0, 1.0);
        let z = 1.0_f32;
        // Same arithmetic as `depth_of_field.wgsl` (must stay in lockstep).
        let coc = circle_of_confusion_px(z, 1080.0, &p);
        assert!(
            coc > 4.0,
            "DoF copies the centre sample below 0.5px; a foreground object must blur, got {coc}px"
        );
    }

    #[test]
    fn an_object_on_the_focal_plane_stays_sharp() {
        let p = dof_params(10.0, 1.0);
        let coc = circle_of_confusion_px(10.0, 1080.0, &p);
        assert!(coc < 0.5, "in-focus pixels must early-out, got {coc}px");
    }
}
