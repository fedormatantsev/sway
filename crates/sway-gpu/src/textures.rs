//! Offscreen render targets used by the editor's compositor.
//!
//! For M1b Task 2 there was exactly one: the UI layer vello renders into.
//! Task 3 adds [`ViewportTexture`], the texture Bevy renders into.
//! [`CameraTarget`] is the same colour attachment sized from an authored
//! resolution rather than from a window, one per camera the graph declares.

use wgpu::{
    Device, Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureView, TextureViewDescriptor,
};

/// Creates the colour attachment Bevy renders through and the compositor
/// samples: one texture, two views in different formats.
///
/// Bevy writes through the sRGB view (so the hardware encodes its linear
/// output) and the compositor samples through the non-sRGB view (so it reads
/// those encoded bytes without decoding them again). `view_formats` must list
/// the second format at creation or wgpu rejects the view. `COPY_SRC` is
/// carried unconditionally: it is what makes a target readable back, which
/// both the headless render test and the capture path depend on.
fn create_color_target(
    device: &Device,
    label: &str,
    width: u32,
    height: u32,
) -> (Texture, TextureView, TextureView) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
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
        label: Some("sway bevy view (srgb)"),
        format: Some(TextureFormat::Rgba8UnormSrgb),
        ..Default::default()
    });
    let sample_view = texture.create_view(&TextureViewDescriptor {
        label: Some("sway sample view (non-srgb)"),
        format: Some(TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    (texture, bevy_view, sample_view)
}

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

    /// The backing texture, for `COPY_SRC` readback (e.g.
    /// `CommandEncoder::copy_texture_to_buffer`). `bevy_view`/`sample_view`
    /// are views, and wgpu's texture-to-buffer copy needs the texture
    /// itself, not a view of it.
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    fn create(device: &Device, width: u32, height: u32) -> (Texture, TextureView, TextureView) {
        create_color_target(device, "sway viewport texture", width, height)
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

/// Why a camera's authored resolution could not become a render target.
///
/// Refusing rather than clamping is deliberate: a camera whose target cannot
/// be produced renders nothing and is reported, because silently substituting
/// some other size would put a differently-framed image on stage without
/// saying so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// A width or a height of zero. wgpu refuses a zero-extent texture, and
    /// there is no meaningful image to substitute.
    ZeroResolution { width: u32, height: u32 },
    /// Larger than the device's `max_texture_dimension_2d` in at least one
    /// axis. The limit is carried so the diagnostic can name it.
    TooLarge {
        width: u32,
        height: u32,
        limit: u32,
    },
}

impl core::fmt::Display for TargetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroResolution { width, height } => {
                write!(f, "resolution {width}x{height} has a zero component")
            }
            Self::TooLarge {
                width,
                height,
                limit,
            } => write!(
                f,
                "resolution {width}x{height} exceeds this device's maximum 2D texture \
                 dimension of {limit}"
            ),
        }
    }
}

impl core::error::Error for TargetError {}

/// One camera's render target, sized by the graph rather than by a window.
///
/// The same view pair [`ViewportTexture`] carries — Bevy writes through
/// `bevy_view`, the compositor samples `sample_view` — but built from an
/// arbitrary authored resolution, and **not resizable**: a camera whose
/// resolution changes gets a new target, because the old one's views are
/// registered with Bevy under a handle that has to be re-pointed anyway.
///
/// Construction is fallible, which is the difference that matters:
/// `ViewportTexture` takes a window size that is always allocatable, while an
/// authored resolution may be zero or larger than the device allows.
pub struct CameraTarget {
    texture: Texture,
    pub bevy_view: TextureView,
    pub sample_view: TextureView,
    pub width: u32,
    pub height: u32,
}

impl CameraTarget {
    /// Allocates a target of exactly `width` x `height`, or reports why it
    /// could not. Never clamps and never rounds.
    pub fn new(device: &Device, width: u32, height: u32) -> Result<Self, TargetError> {
        if width == 0 || height == 0 {
            return Err(TargetError::ZeroResolution { width, height });
        }
        let limit = device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return Err(TargetError::TooLarge {
                width,
                height,
                limit,
            });
        }
        let (texture, bevy_view, sample_view) =
            create_color_target(device, "sway camera target", width, height);
        Ok(Self {
            texture,
            bevy_view,
            sample_view,
            width,
            height,
        })
    }

    /// The backing texture, for `COPY_SRC` readback — see
    /// [`ViewportTexture::texture`].
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpuContext;

    #[test]
    fn an_authored_resolution_becomes_a_target_of_exactly_that_size() {
        // Not rounded to a power of two, not clamped to the window, not
        // aligned to anything: exactly what was asked for.
        let gpu = GpuContext::new(None);
        let target = CameraTarget::new(&gpu.device, 1000, 543).expect("allocatable");
        assert_eq!((target.width, target.height), (1000, 543));
        assert_eq!(target.texture().width(), 1000);
        assert_eq!(target.texture().height(), 543);
    }

    #[test]
    fn a_zero_component_is_refused_rather_than_clamped() {
        let gpu = GpuContext::new(None);
        let error = CameraTarget::new(&gpu.device, 1920, 0)
            .map(|_| ())
            .expect_err("a zero height has no target");
        assert_eq!(
            error,
            TargetError::ZeroResolution {
                width: 1920,
                height: 0
            }
        );
    }

    #[test]
    fn a_resolution_larger_than_the_device_allows_is_refused_and_names_the_limit() {
        // The diagnostic has to name the limit, so the author can tell "too
        // big" from "wrong". Refusing rather than downsizing is the point:
        // a silently smaller target frames the same but resolves differently.
        let gpu = GpuContext::new(None);
        let limit = gpu.device.limits().max_texture_dimension_2d;
        let error = CameraTarget::new(&gpu.device, limit + 1, 1080)
            .map(|_| ())
            .expect_err("beyond the limit");
        assert_eq!(
            error,
            TargetError::TooLarge {
                width: limit + 1,
                height: 1080,
                limit
            }
        );
        assert!(error.to_string().contains(&limit.to_string()));
    }
}
