//! Pure signal transforms retained for their wire-model return.

use bevy_reflect::Reflect;

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    #[default]
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

pub fn math_value(op: MathOp, a: f32, b: f32) -> f32 {
    match op {
        MathOp::Add => a + b,
        MathOp::Sub => a - b,
        MathOp::Mul => a * b,
        MathOp::Div if b == 0.0 => 0.0,
        MathOp::Div => a / b,
        MathOp::Min => a.min(b),
        MathOp::Max => a.max(b),
    }
}

pub fn remap_value(
    mut value: f32,
    in_min: f32,
    in_max: f32,
    out_min: f32,
    out_max: f32,
    clamp: bool,
) -> f32 {
    if clamp {
        value = value.clamp(in_min.min(in_max), in_min.max(in_max));
    }
    if in_min == in_max {
        out_min
    } else {
        out_min + (value - in_min) / (in_max - in_min) * (out_max - out_min)
    }
}

pub fn switch_value(select: bool, a: f32, b: f32) -> f32 {
    if select { a } else { b }
}

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteField {
    #[default]
    Note,
    Velocity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_supports_every_operation_and_zero_division() {
        for (op, expected) in [
            (MathOp::Add, 8.0),
            (MathOp::Sub, 4.0),
            (MathOp::Mul, 12.0),
            (MathOp::Div, 3.0),
            (MathOp::Min, 2.0),
            (MathOp::Max, 6.0),
        ] {
            assert_eq!(math_value(op, 6.0, 2.0), expected);
        }
        assert_eq!(math_value(MathOp::Div, 6.0, 0.0), 0.0);
    }

    #[test]
    fn remap_can_extrapolate_clamp_and_handle_degenerate_ranges() {
        assert_eq!(remap_value(15.0, 0.0, 10.0, -1.0, 1.0, false), 2.0);
        assert_eq!(remap_value(15.0, 0.0, 10.0, -1.0, 1.0, true), 1.0);
        assert_eq!(remap_value(4.0, 2.0, 2.0, 7.0, 9.0, false), 7.0);
    }
}
