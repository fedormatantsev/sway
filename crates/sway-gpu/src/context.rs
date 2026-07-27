//! Instance, adapter, device and queue creation.
//!
//! The device is requested with the **union** of what Bevy and vello need. This
//! is the most likely place the shared-device route fails, so the request is
//! explicit and the failure is loud rather than a missing-feature panic deep
//! inside a render pass.

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
