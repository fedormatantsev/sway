//! Pure transport-aware signal and boundary logic.

use bevy_reflect::Reflect;
use sway_graph::MusicalTime;

use crate::lfo::{Waveform, wave};

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    #[default]
    Beat,
    Bar,
    Eighth,
    Sixteenth,
}

impl Division {
    pub fn beats(self, beats_per_bar: u32) -> f64 {
        match self {
            Self::Bar => beats_per_bar.max(1) as f64,
            Self::Beat => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
        }
    }
}

#[derive(Reflect, Default, Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub bar: u32,
    pub beat: u32,
    pub sixteenth: u32,
}

pub const MAX_PULSES_PER_TICK: usize = 64;

#[derive(Default, Debug, Clone)]
pub struct BeatTriggerState {
    prev_end: Option<f64>,
    prev_origin: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BeatPulse {
    pub offset: f32,
    pub value: Beat,
}

#[allow(clippy::too_many_arguments)]
pub fn beat_pulses(
    state: &mut BeatTriggerState,
    division: Division,
    playing: bool,
    beats_per_bar: u32,
    end: f64,
    advanced: f64,
    origin: f64,
    dt: f32,
) -> Vec<BeatPulse> {
    if !playing || advanced <= 0.0 {
        state.prev_end = None;
        state.prev_origin = None;
        return Vec::new();
    }
    let start = match (state.prev_end, state.prev_origin) {
        (Some(previous_end), Some(previous_origin)) if previous_origin == origin => previous_end,
        _ => (end - advanced).max(0.0),
    };
    state.prev_end = Some(end);
    state.prev_origin = Some(origin);
    let step = division.beats(beats_per_bar);
    let first = (start / step).floor() as i64 + 1;
    let last = (end / step).floor() as i64;
    (first..=last.min(first + MAX_PULSES_PER_TICK as i64 - 1))
        .map(|index| {
            let boundary = index as f64 * step;
            let offset = (dt as f64 * (boundary - start) / advanced).clamp(0.0, dt as f64) as f32;
            let at = MusicalTime::from_beats(boundary, beats_per_bar);
            BeatPulse {
                offset,
                value: Beat {
                    bar: at.bar,
                    beat: at.beat,
                    sixteenth: at.sixteenth,
                },
            }
        })
        .collect()
}

pub fn sync_lfo_value(beats: f64, period: f32, shape: Waveform, phase: f32, amplitude: f32) -> f32 {
    let phase = if period > 0.0 {
        (beats / period as f64 + phase as f64).rem_euclid(1.0) as f32
    } else {
        phase.rem_euclid(1.0)
    };
    wave(shape, phase) * amplitude
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_boundaries_are_half_open_and_capped() {
        let mut state = BeatTriggerState::default();
        let pulses = beat_pulses(
            &mut state,
            Division::Sixteenth,
            true,
            4,
            1000.0,
            1000.0,
            0.0,
            1.0 / 120.0,
        );
        assert_eq!(pulses.len(), MAX_PULSES_PER_TICK);
    }

    #[test]
    fn tempo_sync_is_a_function_of_beat_position() {
        let a = sync_lfo_value(0.5, 2.0, Waveform::Sine, 0.0, 1.0);
        let b = sync_lfo_value(0.5, 2.0, Waveform::Sine, 0.0, 1.0);
        assert_eq!(a, b);
    }
}
