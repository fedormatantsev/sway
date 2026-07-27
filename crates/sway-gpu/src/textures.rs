//! Offscreen render targets used by the editor's compositor.
//!
//! For M1b Task 2 there was exactly one: the UI layer vello renders into.
//! Task 3 adds [`ViewportTexture`], the texture Bevy renders into.

use wgpu::{
    Device, Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureView, TextureViewDescriptor,
};

/// The UI layer's offscreen render target.
///
/// Vello renders into this at `Rgba8Unorm` (see `imaging_vello`'s
/// `supported_texture_formats`, which only ever offers `Rgba8Unorm`); the
/// compositor later samples it into the surface.
///
/// Usage is `STORAGE_BINDING | TEXTURE_BINDING`, not `RENDER_ATTACHMENT |
/// TEXTURE_BINDING`: `vello::Renderer::render_to_texture` writes through a
/// compute pipeline (a storage-texture write), never through a render pass,
/// so `RENDER_ATTACHMENT` is neither required nor sufficient. This was
/// confirmed against `imaging_vello`'s own internal offscreen target
/// (`imaging_vello::wgpu_support::create_texture`), which uses
/// `STORAGE_BINDING | TEXTURE_BINDING | COPY_SRC` for exactly this texture
/// role; omitting `STORAGE_BINDING` fails with a wgpu validation error at
/// the first `Device::create_bind_group` inside vello's renderer.
pub struct UiTexture {
    // Kept alongside `view` to hold the resource alive and available for any
    // future direct use (e.g. readback); the compositor only ever touches
    // `view`.
    #[allow(dead_code)]
    texture: Texture,
    pub view: TextureView,
    width: u32,
    height: u32,
}

impl UiTexture {
    pub fn new(device: &Device, width: u32, height: u32) -> Self {
        let (texture, view) = Self::create(device, width, height);
        Self {
            texture,
            view,
            width: width.max(1),
            height: height.max(1),
        }
    }

    fn create(device: &Device, width: u32, height: u32) -> (Texture, TextureView) {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("sway ui texture"),
            size: Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        (texture, view)
    }

    /// Recreates the texture (and its view) if `width`/`height` differ from
    /// the current size. A no-op otherwise, so resizing every frame with an
    /// unchanged size is cheap.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return;
        }
        let (texture, view) = Self::create(device, width, height);
        self.texture = texture;
        self.view = view;
        self.width = width;
        self.height = height;
    }
}

/// The texture Bevy renders into.
///
/// Two views of one texture, in different formats: Bevy writes through the
/// sRGB view (so the hardware encodes its linear output), and the compositor
/// samples through the non-sRGB view (so it reads those encoded bytes without
/// decoding them again). `view_formats` must list the second format at
/// creation or wgpu rejects the view.
pub struct ViewportTexture {
    // Held alongside the views to keep the resource alive; nothing reads it
    // directly (mirrors `UiTexture`'s `texture` field).
    #[allow(dead_code)]
    texture: Texture,
    pub bevy_view: TextureView,
    pub sample_view: TextureView,
    pub width: u32,
    pub height: u32,
}

impl ViewportTexture {
    pub fn new(device: &Device, width: u32, height: u32) -> Self {
        let (texture, bevy_view, sample_view) = Self::create(device, width, height);
        Self {
            texture,
            bevy_view,
            sample_view,
            width: width.max(1),
            height: height.max(1),
        }
    }

    fn create(device: &Device, width: u32, height: u32) -> (Texture, TextureView, TextureView) {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("sway viewport texture"),
            size: Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[TextureFormat::Rgba8Unorm],
        });
        let bevy_view = texture.create_view(&TextureViewDescriptor {
            label: Some("sway viewport bevy view (srgb)"),
            format: Some(TextureFormat::Rgba8UnormSrgb),
            ..Default::default()
        });
        let sample_view = texture.create_view(&TextureViewDescriptor {
            label: Some("sway viewport sample view (non-srgb)"),
            format: Some(TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        (texture, bevy_view, sample_view)
    }

    /// Recreates the texture (and both its views) if `width`/`height` differ
    /// from the current size. A no-op otherwise. Callers that hold a Bevy
    /// `App` must follow a resize with `sway_runtime::headless::set_viewport_view`
    /// -- the old views are dropped here and the app's `ManualTextureViews`
    /// entry would otherwise point at a destroyed texture.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return;
        }
        let (texture, bevy_view, sample_view) = Self::create(device, width, height);
        self.texture = texture;
        self.bevy_view = bevy_view;
        self.sample_view = sample_view;
        self.width = width;
        self.height = height;
    }
}
