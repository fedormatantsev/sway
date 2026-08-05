//! Pure scene-value construction retained for future wire behaviours.

use bevy::prelude::*;

pub fn transform(
    translation: Vec3,
    rotation_x: f32,
    rotation_y: f32,
    rotation_z: f32,
    scale: Vec3,
) -> Transform {
    Transform {
        translation,
        rotation: Quat::from_euler(EulerRot::XYZ, rotation_x, rotation_y, rotation_z),
        scale,
    }
}

pub fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::srgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_values_preserve_authored_components() {
        let value = transform(Vec3::new(1.0, 2.0, 3.0), 0.0, 0.0, 0.0, Vec3::ONE);
        assert_eq!(value.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(rgb(1.0, 0.5, 0.0), Color::srgb(1.0, 0.5, 0.0));
    }
}
