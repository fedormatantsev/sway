//! Composites one or more source textures onto a target through
//! `assets/shaders/composite.wgsl`.
//!
//! Two render pipelines exist — opaque and alpha-blended — because blend
//! state is baked into a `wgpu::RenderPipeline` at creation time; there is no
//! way to select it per-draw.
//!
//! `draw` (below) is `pub(crate)`, reachable only through `Frame::composite`
//! (see `frame.rs`) — `Compositor` itself stays `pub` so a caller can own one
//! across frames (constructing the pipelines and the per-quad buffer/bind
//! group cache is not something to redo every frame), but the only way to
//! actually composite is through a `Frame`, which is also the only place a
//! `wgpu::CommandEncoder` gets created. That keeps all wgpu object creation
//! inside `sway-gpu`, per the crate's one job.

use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBindingType,
    BufferDescriptor, BufferUsages, Color, ColorTargetState, ColorWrites, CommandEncoder, Device,
    FilterMode, FragmentState, LoadOp, MipmapFilterMode, MultisampleState, Operations,
    PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TextureFormat,
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

/// Per-quad-position GPU resources, reused frame over frame instead of
/// recreated. Position in `Compositor::slots` corresponds to a quad's index
/// in the `quads` slice passed to `draw` (quad 0 is always the Bevy
/// viewport, quad 1 always the UI layer, once Task 3 adds the viewport) --
/// not to any identity of the source texture, so a slot's bind group is
/// rebuilt whenever the view it was built against stops matching the
/// incoming quad's view (e.g. after a resize recreates a texture).
struct QuadSlot {
    /// Holds the NDC bounds uniform. Rewritten via `queue.write_buffer` every
    /// frame (the destination rect can change even when the source view
    /// doesn't); never recreated.
    rect_buffer: Buffer,
    bind_group: Option<BindGroup>,
    /// The view `bind_group` was built against, so a changed view (a resized
    /// texture was recreated, not just resized in place) is detected and the
    /// bind group rebuilt. `wgpu::TextureView` is cheap to clone (an Arc-style
    /// handle) and implements `Eq` by the underlying resource identity, so
    /// this comparison is exact, not heuristic.
    cached_view: Option<TextureView>,
}

impl QuadSlot {
    fn new(device: &Device) -> Self {
        let rect_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("sway composite quad rect"),
            size: 16, // vec4<f32>
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            rect_buffer,
            bind_group: None,
            cached_view: None,
        }
    }
}

/// Draws textured quads (the Bevy viewport, then the UI layer) onto a
/// target through `composite.wgsl`. Owned across frames by the caller (see
/// `frame.rs`); the pipelines, sampler, and per-quad buffer/bind group cache
/// are all built once and reused, not recreated per frame.
pub struct Compositor {
    bind_group_layout: BindGroupLayout,
    sampler: Sampler,
    pipeline_opaque: RenderPipeline,
    pipeline_blend: RenderPipeline,
    slots: Vec<QuadSlot>,
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
            slots: Vec::new(),
        }
    }

    /// Draws every quad, in order, into a single render pass over `target`.
    /// The target is cleared to opaque black first, so a quad with
    /// `blend: true` composites over that (or over an earlier quad already
    /// drawn in this call) rather than over whatever `target` held before.
    ///
    /// `pub(crate)`: only `Frame::composite` calls this, so a
    /// `wgpu::CommandEncoder` (created by `Frame`) never needs to exist
    /// outside `sway-gpu`.
    pub(crate) fn draw(
        &mut self,
        encoder: &mut CommandEncoder,
        device: &Device,
        queue: &Queue,
        target: &TextureView,
        quads: &[Quad],
    ) {
        let (width, height) = {
            let texture = target.texture();
            (texture.width(), texture.height())
        };

        while self.slots.len() < quads.len() {
            self.slots.push(QuadSlot::new(device));
        }

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

        for (i, quad) in quads.iter().enumerate() {
            let slot = &mut self.slots[i];

            let bounds = to_ndc(quad.dst, width as f32, height as f32);
            queue.write_buffer(&slot.rect_buffer, 0, bytemuck::cast_slice(&bounds));

            let view_changed = slot.cached_view.as_ref() != Some(quad.view);
            if slot.bind_group.is_none() || view_changed {
                slot.bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
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
                            resource: slot.rect_buffer.as_entire_binding(),
                        },
                    ],
                }));
                slot.cached_view = Some(quad.view.clone());
            }

            let pipeline = if quad.blend {
                &self.pipeline_blend
            } else {
                &self.pipeline_opaque
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, slot.bind_group.as_ref().unwrap(), &[]);
            pass.draw(0..6, 0..1);
        }
    }
}
