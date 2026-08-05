//! Pure material construction retained for the future asset-flow slice.

use bevy::prelude::*;

pub fn standard_material(
    base_color: Color,
    emissive: Color,
    metallic: f32,
    perceptual_roughness: f32,
) -> StandardMaterial {
    StandardMaterial {
        base_color,
        emissive: emissive.into(),
        metallic,
        perceptual_roughness,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_parameters_are_preserved() {
        let material = standard_material(Color::WHITE, Color::BLACK, 0.25, 0.75);
        assert_eq!(material.base_color, Color::WHITE);
        assert_eq!(material.metallic, 0.25);
        assert_eq!(material.perceptual_roughness, 0.75);
    }
}
