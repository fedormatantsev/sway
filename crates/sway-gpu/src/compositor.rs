//! Composites one or more source textures onto a target through
//! `assets/shaders/composite.wgsl`.
//!
//! Two render pipelines exist — opaque and alpha-blended — because blend
//! state is baked into a `wgpu::RenderPipeline` at creation time; there is no
//! way to select it per-draw.

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, BufferBindingType,
    BufferUsages, Color, ColorTargetState, ColorWrites, CommandEncoder, Device, FilterMode,
    FragmentState, LoadOp, MipmapFilterMode, MultisampleState, Operations,
    PipelineLayoutDescriptor, PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TextureFormat,
    TextureSampleType, TextureView, TextureViewDimension, VertexState,
};

/// One textured quad to composite: `view` is sampled full-frame (UV 0..1)
/// into `dst`, a destination rectangle in the target's physical pixels.
pub struct Quad<'a> {
    pub view: &'a TextureView,
    pub dst: kurbo::Rect,
    pub blend: bool,
}

/// Converts a destination rectangle in physical pixels to the shader's NDC
/// bounds (`-1..1`, y up). Note the y flip and the y0/y1 swap: kurbo's y
/// grows downward, NDC's grows upward.
fn to_ndc(dst: kurbo::Rect, width: f32, height: f32) -> [f32; 4] {
    [
        (dst.x0 as f32 / width) * 2.0 - 1.0,
        1.0 - (dst.y1 as f32 / height) * 2.0,
        (dst.x1 as f32 / width) * 2.0 - 1.0,
        1.0 - (dst.y0 as f32 / height) * 2.0,
    ]
}

/// Draws textured quads (the Bevy viewport, then the UI layer) onto a
/// target through `composite.wgsl`.
pub struct Compositor {
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_opaque: RenderPipeline,
    pipeline_blend: RenderPipeline,
}

impl Compositor {
    pub fn new(device: &Device, surface_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("sway composite shader"),
            source: ShaderSource::Wgsl(include_str!("../assets/shaders/composite.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("sway composite bind group layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("sway composite pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("sway composite sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            ..SamplerDescriptor::default()
        });

        let make_pipeline = |label: &str, blend: Option<BlendState>| {
            device.create_render_pipeline(&RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(ColorTargetState {
                        format: surface_format,
                        blend,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let pipeline_opaque = make_pipeline("sway composite (opaque)", None);
        let pipeline_blend = make_pipeline(
            "sway composite (alpha blend)",
            Some(BlendState::ALPHA_BLENDING),
        );

        Self {
            bind_group_layout,
            sampler,
            pipeline_opaque,
            pipeline_blend,
        }
    }

    /// Draws every quad, in order, into a single render pass over `target`.
    /// The target is cleared to opaque black first, so a quad with
    /// `blend: true` composites over that (or over an earlier quad already
    /// drawn in this call) rather than over whatever `target` held before.
    pub fn draw(
        &self,
        encoder: &mut CommandEncoder,
        device: &Device,
        target: &TextureView,
        quads: &[Quad],
    ) {
        let (width, height) = {
            let texture = target.texture();
            (texture.width(), texture.height())
        };

        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sway composite pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        for quad in quads {
            let bounds = to_ndc(quad.dst, width as f32, height as f32);
            let rect_buffer = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("sway composite quad rect"),
                contents: bytemuck::cast_slice(&bounds),
                usage: BufferUsages::UNIFORM,
            });
            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("sway composite bind group"),
                layout: &self.bind_group_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(quad.view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: rect_buffer.as_entire_binding(),
                    },
                ],
            });

            let pipeline = if quad.blend {
                &self.pipeline_blend
            } else {
                &self.pipeline_opaque
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}
