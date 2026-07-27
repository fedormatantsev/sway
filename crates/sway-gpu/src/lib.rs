//! The single place wgpu objects are created (spec §2.8).
//!
//! Every other crate reaches wgpu through `sway_gpu::wgpu`, so a version bump
//! is one manifest line and one crate's problem.

pub mod context;

pub use context::GpuContext;
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
