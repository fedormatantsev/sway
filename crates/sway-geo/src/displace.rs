//! Pure element-wise displacement along normals.

use std::sync::Arc;

use bevy_math::Vec3;

use crate::geometry::{Attribute, Geometry};

pub fn displace(input: &Geometry, amount: f32, frequency: f32) -> Option<Geometry> {
    let positions = input.get("P")?.as_vec3()?;
    let normals = input.get("N").and_then(|attribute| attribute.as_vec3());
    let displaced = positions
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let normal = normals.map(|values| values[index]).unwrap_or(Vec3::Y);
            let factor = (position.x * frequency).sin() * (position.z * frequency).sin();
            *position + normal * (amount * factor)
        })
        .collect();
    let mut output = input.clone();
    output.set("P", Attribute::Vec3(Arc::new(displaced)));
    Some(output)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{GridParams, grid};

    #[test]
    fn unchanged_attributes_are_shared_and_positions_are_rebuilt() {
        let input = grid(GridParams {
            rows: 3,
            cols: 3,
            width: 2.0,
            height: 2.0,
        });
        let output = displace(&input, 0.5, 1.0).unwrap();
        let (Some(Attribute::Vec3(input_normals)), Some(Attribute::Vec3(output_normals))) =
            (input.get("N"), output.get("N"))
        else {
            panic!("normal attributes");
        };
        assert!(Arc::ptr_eq(input_normals, output_normals));
        let input_positions = input.get("P").unwrap().as_vec3().unwrap();
        let output_positions = output.get("P").unwrap().as_vec3().unwrap();
        assert!(!Arc::ptr_eq(input_positions, output_positions));
    }
}
