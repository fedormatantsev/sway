//! Instance, adapter, device and queue creation.
//!
//! The device is requested with the adapter's **entire** non-experimental
//! feature set and full limits, rather than a computed union of what Bevy and
//! vello each need -- a deliberate simplification that satisfies both without
//! either party's requirements being enumerated. Experimental features are
//! subtracted because wgpu 29 refuses to grant them without an unsafe opt-in:
//! on this machine (Apple M4 / Metal) the adapter advertises
//! `EXPERIMENTAL_RAY_QUERY | EXPERIMENTAL_MESH_SHADER |
//! EXPERIMENTAL_COOPERATIVE_MATRIX`, and leaving them in makes
//! `request_device` fail with `ExperimentalFeaturesNotEnabled` (a committed
//! test guards this). This is the most likely place the shared-device route
//! fails, so the request is explicit and the failure is loud rather than a
//! missing-feature panic deep inside a render pass.

use wgpu::{
    Adapter, Backends, Device, DeviceDescriptor, Instance, InstanceDescriptor, Queue,
    PowerPreference, RequestAdapterOptions, Surface,
};

/// The one wgpu context in the process.
pub struct GpuContext {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
}

impl GpuContext {
    /// Creates the process-wide wgpu context.
    ///
    /// `compatible_surface` should be the window surface when one exists, so
    /// the adapter chosen can actually present to it.
    pub fn new(compatible_surface: Option<&Surface<'_>>) -> Self {
        let instance = Instance::new(InstanceDescriptor {
            backends: Backends::from_env().unwrap_or(Backends::PRIMARY),
            ..InstanceDescriptor::new_without_display_handle()
        });

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::HighPerformance,
            compatible_surface,
            force_fallback_adapter: false,
        }))
        .expect("no suitable wgpu adapter");

        // The union. Bevy's own initialisation asks for the adapter's full
        // feature set and then downgrades; vello needs no optional features on
        // the wgpu backend but does need non-default limits for its bind
        // groups. Taking the adapter's limits wholesale satisfies both without
        // guessing which specific limit each one reads.
        //
        // Experimental features (ray query, mesh shader, cooperative matrix, ...)
        // are excluded: wgpu 29 requires an explicit, `unsafe`
        // `ExperimentalFeatures::enabled()` acknowledgement to request them, and
        // requesting them without it is a hard `request_device` error
        // (`ExperimentalFeaturesNotEnabled`) rather than a silent downgrade.
        // Neither Bevy 0.19 nor vello need them for this milestone.
        let features = adapter.features() - wgpu::Features::all_experimental_mask();
        let limits = adapter.limits();

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("sway shared device"),
            required_features: features,
            required_limits: limits,
            // wgpu 29 added `experimental_features` alongside `required_features`;
            // we take the defaults (no experimental features requested).
            ..Default::default()
        }))
        .expect("could not create the shared wgpu device");

        Self { instance, adapter, device, queue }
    }
}

#[cfg(test)]
mod tests {
    use super::GpuContext;

    /// Guards the experimental-features subtraction in `GpuContext::new`,
    /// not "wgpu works" in general.
    ///
    /// On at least one real adapter (Metal on Apple Silicon), `adapter.features()`
    /// advertises `EXPERIMENTAL_RAY_QUERY`, `EXPERIMENTAL_MESH_SHADER`, and
    /// `EXPERIMENTAL_COOPERATIVE_MATRIX`. wgpu 29 will not grant those to a
    /// device without an explicit, `unsafe ExperimentalFeatures::enabled()`
    /// opt-in, so passing `adapter.features()` straight through as
    /// `required_features` makes `request_device` fail with
    /// `ExperimentalFeaturesNotEnabled` — this was reproduced during
    /// development. `GpuContext::new` subtracts
    /// `wgpu::Features::all_experimental_mask()` before requesting the
    /// device specifically to avoid that. If someone later deletes that
    /// subtraction (e.g. while "simplifying" the feature/limit union logic),
    /// this test starts panicking through `GpuContext::new`'s own
    /// `.expect("could not create the shared wgpu device")` and this comment
    /// is the explanation.
    ///
    /// Not `#[ignore]`: this project has no CI and runs only on this
    /// developer's Mac, so a plain `#[test]` that fails loudly with no
    /// adapter present is more useful than a silently-skipped one.
    #[test]
    fn gpu_context_new_succeeds_despite_adapter_advertised_experimental_features() {
        // The real regression guard is that this call does not panic. If
        // `GpuContext::new`'s experimental-features subtraction is ever
        // deleted, and the adapter under test still advertises any
        // `EXPERIMENTAL_*` feature (true for Metal on Apple Silicon, per the
        // doc comment above), `request_device` fails with
        // `ExperimentalFeaturesNotEnabled` and this test panics right here.
        let ctx = GpuContext::new(None);

        // Confirm the device is real and usable, not merely constructed.
        assert!(ctx.device.limits().max_bind_groups > 0);

        // Record what this run's adapter actually advertised, so a failure
        // to reproduce the guarded condition on a different machine is
        // visible in the test output rather than silently vacuous.
        let experimental = ctx.adapter.features() & wgpu::Features::all_experimental_mask();
        println!("adapter-advertised experimental features on this run: {experimental:?}");
    }
}
