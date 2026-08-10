//! Wires into the scene transform. Roadmap D5: these take `Vec3`, not floats.

use bevy::prelude::*;

use crate::field_wire::field_wire;
use crate::outputs::Vec3Out;

field_wire!(
    /// Drives `Transform.translation` whole. There is no per-axis wire: an
    /// offset that used to live in the authored `Transform` now lives in the
    /// `Vec3` node feeding this (roadmap D5).
    TranslationFrom / DrivesTranslation,
    Vec3Out => Transform,
    "translation",
    |t| &mut t.translation,
    |s| s.0
);

field_wire!(
    /// Drives `Transform.rotation` from euler angles in **degrees**, XYZ order.
    /// The quaternion is built before the comparison, so an unchanged triple
    /// leaves `Transform` clean.
    RotationFrom / DrivesRotation,
    Vec3Out => Transform,
    "rotation",
    |t| &mut t.rotation,
    |s| Quat::from_euler(
        EulerRot::XYZ,
        s.0.x.to_radians(),
        s.0.y.to_radians(),
        s.0.z.to_radians()
    )
);

field_wire!(
    /// Drives `Transform.scale` whole.
    ScaleFrom / DrivesScale,
    Vec3Out => Transform,
    "scale",
    |t| &mut t.scale,
    |s| s.0
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outputs::Vec3Out;
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::propagate_of;

    #[test]
    fn translation_and_scale_write_the_whole_vector() {
        let mut world = World::new();
        let src = world.spawn(Vec3Out(Vec3::new(1.0, 2.0, 3.0))).id();
        let dst = world.spawn(Transform::default()).id();

        propagate_of::<TranslationFrom>(&mut world, src, dst);
        propagate_of::<ScaleFrom>(&mut world, src, dst);

        let transform = world.get::<Transform>(dst).copied().expect("present");
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn rotation_reads_euler_degrees() {
        // Degrees because that is what an author types. The wire converts, so
        // nothing downstream ever sees a degree.
        let mut world = World::new();
        let src = world.spawn(Vec3Out(Vec3::new(0.0, 90.0, 0.0))).id();
        let dst = world.spawn(Transform::default()).id();

        propagate_of::<RotationFrom>(&mut world, src, dst);

        let rotation = world.get::<Transform>(dst).expect("present").rotation;
        let turned = rotation * Vec3::Z;
        assert!(
            (turned - Vec3::X).length() < 1e-5,
            "90 degrees about Y must take +Z to +X, got {turned:?}"
        );
    }

    #[test]
    fn the_transform_wires_never_write_an_equal_value() {
        assert_writes_only_on_change::<TranslationFrom>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        assert_writes_only_on_change::<ScaleFrom>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        // The quaternion is compared, not the euler triple it came from.
        assert_writes_only_on_change::<RotationFrom>(
            Vec3Out(Vec3::new(0.0, 90.0, 0.0)),
            Vec3Out(Vec3::new(0.0, 45.0, 0.0)),
            Transform::default(),
        );
    }
}
