//! Pure oscillator logic retained from the previous graph integration.

use core::f32::consts::TAU;

use bevy_reflect::Reflect;

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
}

pub fn lfo_value(hz: f32, shape: Waveform, phase: f32, amplitude: f32, time: f64) -> f32 {
    let phase = (time * hz as f64 + phase as f64).rem_euclid(1.0) as f32;
    wave(shape, phase) * amplitude
}

pub(crate) fn wave(shape: Waveform, phase: f32) -> f32 {
    match shape {
        Waveform::Sine => (phase * TAU).sin(),
        Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        Waveform::Saw => 2.0 * phase - 1.0,
        Waveform::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_samples_do_not_change_absolute_time_output() {
        let direct = lfo_value(2.25, Waveform::Triangle, 0.17, 0.8, 99.0 / 120.0);
        let after_gap = lfo_value(2.25, Waveform::Triangle, 0.17, 0.8, 99.0 / 120.0);
        assert_eq!(direct, after_gap);
    }

    #[test]
    fn waveforms_are_bipolar_and_amplitude_scaled() {
        for (shape, expected) in [
            (Waveform::Sine, 0.5),
            (Waveform::Triangle, 0.0),
            (Waveform::Saw, -0.25),
            (Waveform::Square, 0.5),
        ] {
            assert!((lfo_value(0.0, shape, 0.25, 0.5, 0.0) - expected).abs() < 1e-6);
        }
    }
}
