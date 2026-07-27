//! The single place wgpu objects are created (spec §2.8).
//!
//! Every other crate reaches wgpu through `sway_gpu::wgpu`, so a version bump
//! is one manifest line and one crate's problem.

pub mod compositor;
pub mod context;
pub mod surface;
pub mod textures;
pub mod ui_render;

pub use compositor::{Compositor, Quad};
pub use context::GpuContext;
pub use surface::WindowSurface;
pub use textures::UiTexture;
pub use ui_render::UiRenderer;
pub use wgpu;

#[cfg(test)]
mod version_gate {
    /// The M1b go/no-go gate for device sharing, asserted at compile time.
    ///
    /// Bevy 0.19 and `imaging_vello`'s `vello-0-9` feature must resolve to the
    /// *same* `wgpu` crate, or `RenderDevice::from` cannot accept the device
    /// vello was built against. If cargo ever resolves two `wgpu` versions,
    /// this function stops compiling with a type mismatch naming both — which
    /// is a far better failure than a runtime error about an unrelated
    /// resource, and is why spec §2.8 asks for duplicate detection at all.
    #[test]
    fn bevy_and_vello_share_one_wgpu() {
        fn _same_device(d: imaging_vello::wgpu::Device) -> bevy::render::renderer::RenderDevice {
            bevy::render::renderer::RenderDevice::from(d)
        }
        fn _same_queue(q: imaging_vello::wgpu::Queue) -> wgpu::Queue {
            q
        }
    }
}

#[cfg(test)]
mod shader_validation {
    //! `sway-runtime`'s equivalent harness is `#[cfg(test)]`-private and cannot
    //! be reached across crates, so this is a deliberate second copy rather
    //! than an oversight. It is small, and the alternative — making the other
    //! crate's test helpers public API — is worse.

    fn validate_wgsl(name: &str, src: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(src)
            .map_err(|e| format!("{name}: parse failed:\n{}", e.emit_to_string(src)))?;
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .map_err(|e| format!("{name}: validation failed: {e:?}"))?;
        Ok(())
    }

    #[test]
    fn composite_shader_validates() {
        let src = include_str!("../assets/shaders/composite.wgsl");
        validate_wgsl("composite.wgsl", src).unwrap();
    }

    #[test]
    fn validator_rejects_a_type_error() {
        let bad = "@fragment fn fragment() -> @location(0) vec4<f32> { return vec3<f32>(1.0); }";
        assert!(validate_wgsl("bad", bad).is_err());
    }
}
