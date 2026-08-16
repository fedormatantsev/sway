//! Pure transport-aware signal and boundary logic.

use bevy_reflect::Reflect;

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
pub struct BeatTriggerState;

#[derive(Debug, Clone, PartialEq)]
pub struct BeatPulse {
    pub offset: f32,
    pub value: Beat,
}

fn beat_from_ppq(ppq: f64, beats_per_bar: u32) -> Beat {
    let per_bar = f64::from(beats_per_bar.max(1));
    let ppq = ppq.max(0.0);
    let in_bar = ppq % per_bar;
    Beat {
        bar: ((ppq / per_bar) as u32).saturating_add(1),
        beat: (in_bar as u32).saturating_add(1),
        sixteenth: ((in_bar.fract() * 4.0) as u32).saturating_add(1),
    }
}

pub fn beat_pulses(
    state: &mut BeatTriggerState,
    division: Division,
    playing: bool,
    beats_per_bar: u32,
    prev_ppq: Option<f64>,
    ppq: f64,
    dt: f32,
) -> Vec<BeatPulse> {
    let relocated = prev_ppq.is_some_and(|prev| ppq < prev || ppq - prev > 1.0);
    if !playing || relocated {
        *state = BeatTriggerState;
        return Vec::new();
    }
    let start = prev_ppq.unwrap_or(ppq);
    let end = ppq;
    let advanced = end - start;
    let _ = state;
    if advanced <= 0.0 {
        return Vec::new();
    }
    let step = division.beats(beats_per_bar);
    let first = (start / step).floor() as i64 + 1;
    let last = (end / step).floor() as i64;
    (first..=last.min(first + MAX_PULSES_PER_TICK as i64 - 1))
        .map(|index| {
            let boundary = index as f64 * step;
            let offset = (dt as f64 * (boundary - start) / advanced).clamp(0.0, dt as f64) as f32;
            let at = beat_from_ppq(boundary, beats_per_bar);
            BeatPulse { offset, value: at }
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
    fn a_stopped_or_relocated_playhead_emits_nothing() {
        let mut state = BeatTriggerState::default();
        assert!(
            beat_pulses(
                &mut state,
                Division::Beat,
                false,
                4,
                Some(0.0),
                0.5,
                1.0 / 120.0
            )
            .is_empty()
        );
        assert!(
            beat_pulses(
                &mut state,
                Division::Beat,
                true,
                4,
                Some(2.0),
                1.0,
                1.0 / 120.0
            )
            .is_empty()
        );
        assert!(
            beat_pulses(
                &mut state,
                Division::Beat,
                true,
                4,
                Some(0.0),
                2.5,
                1.0 / 120.0
            )
            .is_empty()
        );
    }

    #[test]
    fn a_one_beat_advance_emits_the_crossed_boundary() {
        let mut state = BeatTriggerState::default();
        let pulses = beat_pulses(
            &mut state,
            Division::Beat,
            true,
            4,
            Some(0.25),
            1.25,
            1.0 / 120.0,
        );
        assert_eq!(pulses.len(), 1);
        assert_eq!(
            pulses[0].value,
            Beat {
                bar: 1,
                beat: 2,
                sixteenth: 1,
            }
        );
    }

    #[test]
    fn musical_time_matches_inlined_ppq_arithmetic() {
        assert_eq!(
            beat_from_ppq(0.0, 4),
            Beat {
                bar: 1,
                beat: 1,
                sixteenth: 1,
            }
        );
        assert_eq!(
            beat_from_ppq(17.5, 4),
            Beat {
                bar: 5,
                beat: 2,
                sixteenth: 3,
            }
        );
        assert_eq!(
            beat_from_ppq(7.0, 3),
            Beat {
                bar: 3,
                beat: 2,
                sixteenth: 1
            }
        );
    }

    #[test]
    fn tempo_sync_is_a_function_of_beat_position() {
        let a = sync_lfo_value(0.5, 2.0, Waveform::Sine, 0.0, 1.0);
        let b = sync_lfo_value(0.5, 2.0, Waveform::Sine, 0.0, 1.0);
        assert_eq!(a, b);
    }
}
