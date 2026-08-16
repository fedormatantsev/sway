//! Wires into the scene transform. Roadmap D5: these take `Vec3`, not floats.

use std::any::TypeId;

use bevy::prelude::*;
use bevy_ecs::change_detection::Mut;
use bevy_reflect::{PartialReflect, Reflect};
use sway_graph::{ReflectWire, Wire};

use crate::field_wire::field_wire;
use crate::outputs::Vec3Out;

field_wire!(
    /// Drives `Transform.translation` whole. There is no per-axis wire: an
    /// offset that used to live in the authored `Transform` now lives in the
    /// `Vec3` node feeding this (roadmap D5).
    TranslationFrom / DrivesTranslation,
    Vec3Out => Transform,
    "translation"
);

field_wire!(
    /// Drives `Transform.scale` whole.
    ScaleFrom / DrivesScale,
    Vec3Out => Transform,
    "scale"
);

/// Drives `Transform.rotation` from euler angles in **degrees**, XYZ order.
/// The quaternion is built before the comparison, so an unchanged triple
/// leaves `Transform` clean.
#[derive(Component, Reflect, Clone, Copy)]
#[relationship(relationship_target = DrivesRotation)]
#[reflect(Component, Wire)]
pub struct RotationFrom(#[entities] pub Entity);

#[derive(Component)]
#[relationship_target(relationship = RotationFrom)]
pub struct DrivesRotation(Vec<Entity>);

impl From<Entity> for RotationFrom {
    fn from(entity: Entity) -> Self {
        Self(entity)
    }
}

impl Wire for RotationFrom {
    fn producer(&self) -> Entity {
        self.0
    }

    fn source_type(&self) -> TypeId {
        TypeId::of::<Vec3Out>()
    }

    fn target_type(&self) -> TypeId {
        TypeId::of::<Transform>()
    }

    fn source_path(&self) -> &'static str {
        "0"
    }

    fn target_path(&self) -> &'static str {
        "rotation"
    }

    fn propagate(&self, outlet: &dyn PartialReflect, mut inlet: Mut<dyn Reflect>) {
        let Some(euler) = (match outlet.reflect_ref() {
            bevy_reflect::ReflectRef::TupleStruct(tuple) => tuple
                .field(0)
                .and_then(|field| field.try_downcast_ref::<Vec3>().copied()),
            _ => None,
        }) else {
            return;
        };
        let quat = Quat::from_euler(
            EulerRot::XYZ,
            euler.x.to_radians(),
            euler.y.to_radians(),
            euler.z.to_radians(),
        );
        let Some(current) = (*inlet).downcast_ref::<Transform>() else {
            return;
        };
        if current.rotation == quat {
            return;
        }
        let Some(transform) = inlet.downcast_mut::<Transform>() else {
            return;
        };
        transform.rotation = quat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outputs::Vec3Out;
    use crate::wire_testing::{assert_writes_only_on_change, propagate_wire};
    use sway_graph::register_wire_type;

    fn spatial_app() -> App {
        let mut app = App::new();
        app.register_type::<Vec3Out>();
        app.register_type::<Transform>();
        register_wire_type::<TranslationFrom>(&mut app);
        register_wire_type::<ScaleFrom>(&mut app);
        register_wire_type::<RotationFrom>(&mut app);
        app
    }

    #[test]
    fn translation_and_scale_write_the_whole_vector() {
        let mut app = spatial_app();
        let src = app
            .world_mut()
            .spawn(Vec3Out(Vec3::new(1.0, 2.0, 3.0)))
            .id();
        let dst = app.world_mut().spawn(Transform::default()).id();

        propagate_wire::<TranslationFrom>(app.world_mut(), src, dst);
        propagate_wire::<ScaleFrom>(app.world_mut(), src, dst);

        let transform = app.world().get::<Transform>(dst).copied().expect("present");
        assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(transform.scale, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn rotation_reads_euler_degrees() {
        // Degrees because that is what an author types. The wire converts, so
        // nothing downstream ever sees a degree.
        let mut app = spatial_app();
        let src = app
            .world_mut()
            .spawn(Vec3Out(Vec3::new(0.0, 90.0, 0.0)))
            .id();
        let dst = app.world_mut().spawn(Transform::default()).id();

        propagate_wire::<RotationFrom>(app.world_mut(), src, dst);

        let rotation = app.world().get::<Transform>(dst).expect("present").rotation;
        let turned = rotation * Vec3::Z;
        assert!(
            (turned - Vec3::X).length() < 1e-5,
            "90 degrees about Y must take +Z to +X, got {turned:?}"
        );
    }

    #[test]
    fn the_transform_wires_never_write_an_equal_value() {
        assert_writes_only_on_change::<TranslationFrom, _, _>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        assert_writes_only_on_change::<ScaleFrom, _, _>(
            Vec3Out(Vec3::ONE),
            Vec3Out(Vec3::X),
            Transform::default(),
        );
        // The quaternion is compared, not the euler triple it came from.
        assert_writes_only_on_change::<RotationFrom, _, _>(
            Vec3Out(Vec3::new(0.0, 90.0, 0.0)),
            Vec3Out(Vec3::new(0.0, 45.0, 0.0)),
            Transform::default(),
        );
    }
}
