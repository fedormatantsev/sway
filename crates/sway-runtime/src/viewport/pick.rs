//! Click-to-select. Spec M7-6.

use bevy::camera::Camera;
use bevy::math::Ray3d;
use bevy::prelude::*;

/// Builds a world-space ray from a normalized viewport position.
///
/// `pos` is `[0,1]²` from the top-left (spec M7-1); `Camera::viewport_to_world`
/// wants viewport pixels, and `logical_viewport_size` is what "viewport
/// pixels" means for this camera's own target.
///
/// **Step 1 finding (verified against the pinned `bevy_camera-0.19.0` and
/// `bevy_render-0.19.0` sources, not assumed):** `Camera::viewport_to_world`
/// (`bevy_camera/src/camera.rs:647`) calls `viewport_to_ndc`
/// (`bevy_camera/src/camera.rs:799`), which divides the incoming position by
/// `logical_viewport_rect()`'s size — i.e. by `logical_viewport_size()`
/// (`bevy_camera/src/camera.rs:479`). That in turn is `to_logical` of the
/// physical size, and `Camera::to_logical` (`bevy_camera/src/camera.rs:434`)
/// divides by `computed.target_info.scale_factor`. That field is populated by
/// `bevy_render`'s `camera_system`
/// (`bevy_render/src/camera.rs:394` calling `get_render_target_info` at
/// line 268), and for `NormalizedRenderTarget::TextureView` specifically
/// (`bevy_render/src/camera.rs:294-300`) the `scale_factor` is *hardcoded to
/// `1.0`* — unlike the `Window` and `Image` branches, which read a real DPI
/// scale factor. This crate's viewport always renders to
/// `RenderTarget::TextureView` (see `headless.rs`'s `retarget_cameras`), so
/// for every camera this function is ever called with,
/// `logical_viewport_size() == physical_viewport_size()` (as floats) and
/// `pos * camera.logical_viewport_size()` is an exact pixel position, not an
/// approximation. The brief's given implementation is therefore correct as
/// written and needed no fix.
///
/// `None` covers a camera with no viewport size yet (the first frame, or a
/// zero-sized target) and a degenerate projection. Both are routine, not
/// errors.
pub fn viewport_ray(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    pos: Vec2,
) -> Option<Ray3d> {
    let size = camera.logical_viewport_size()?;
    camera.viewport_to_world(camera_transform, pos * size).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera at +Z looking down -Z, with a known viewport size.
    ///
    /// Built through a real headless `App` (the same scaffolding as
    /// `headless.rs`'s own `bevy_render_output_reaches_the_viewport_texture`
    /// test) rather than `Camera::default()`, because `viewport_to_world`
    /// reads `Camera::computed.clip_from_view` and
    /// `computed.target_info.scale_factor`, both of which only Bevy's own
    /// `bevy_render::camera::camera_system` fills in — a bare, unrun `Camera`
    /// has `computed: ComputedCameraValues::default()`, which yields `None`
    /// out of `logical_viewport_size()`. Chosen over the brief's
    /// input-assertion fallback because it is not actually heavier here:
    /// `camera_system` runs in ordinary `PreUpdate`/`PostUpdate` scheduling
    /// before any GPU work, so, unlike `headless.rs`'s pixel-readback test,
    /// this needs no wait for asynchronous pipeline compilation — a handful
    /// of `app.update()` calls populate `computed` deterministically. Getting
    /// the real geometric fixture here also verifies the actual ray math
    /// (the point of Task 11), not just that `viewport_ray` forwards its
    /// arguments unchanged.
    fn test_camera() -> (Camera, GlobalTransform) {
        let gpu = sway_gpu::GpuContext::new(None);
        let size = UVec2::new(4, 4);
        let viewport = sway_gpu::ViewportTexture::new(&gpu.device, size.x, size.y);
        let mut app = crate::headless::build_app(&gpu, &viewport, size);

        let transform = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        let entity = app.world_mut().spawn((Camera3d::default(), transform)).id();

        app.finish();
        app.cleanup();
        for _ in 0..5 {
            app.update();
        }

        let world = app.world_mut();
        let camera = world.get::<Camera>(entity).expect("camera component").clone();
        let global_transform = *world
            .get::<GlobalTransform>(entity)
            .expect("transform propagation ran");
        assert!(
            camera.logical_viewport_size().is_some(),
            "fixture camera should have a populated viewport size by now",
        );
        (camera, global_transform)
    }

    #[test]
    fn the_centre_of_the_viewport_casts_down_the_camera_forward_axis() {
        let (camera, transform) = test_camera();
        let ray = viewport_ray(&camera, &transform, Vec2::splat(0.5)).expect("a ray");
        let forward = transform.forward().as_vec3();
        assert!(
            ray.direction.as_vec3().dot(forward) > 0.999,
            "centre ray {:?} should point along {forward:?}",
            ray.direction,
        );
    }

    #[test]
    fn the_left_and_right_edges_cast_to_opposite_sides() {
        let (camera, transform) = test_camera();
        let left = viewport_ray(&camera, &transform, Vec2::new(0.0, 0.5)).expect("a ray");
        let right = viewport_ray(&camera, &transform, Vec2::new(1.0, 0.5)).expect("a ray");
        let right_axis = transform.right().as_vec3();
        assert!(left.direction.as_vec3().dot(right_axis) < 0.0);
        assert!(right.direction.as_vec3().dot(right_axis) > 0.0);
    }

    #[test]
    fn a_camera_with_no_viewport_size_yields_no_ray_rather_than_a_panic() {
        let camera = Camera::default();
        let ray = viewport_ray(&camera, &GlobalTransform::default(), Vec2::splat(0.5));
        assert!(ray.is_none());
    }
}
