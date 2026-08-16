use core::fmt;

use bevy_ecs::resource::Resource;
use bevy_reflect::Reflect;

/// The current MIDI transport playhead, published once per fixed tick.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub ppq: f64,
    pub bpm: f64,
    pub playing: bool,
    pub locked: bool,
    pub beats_per_bar: u32,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            ppq: 0.0,
            bpm: 120.0,
            playing: false,
            locked: false,
            beats_per_bar: 4,
        }
    }
}

/// Musical position: bars, beats and sixteenths, counted from one.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct MusicalTime {
    pub bar: u32,
    pub beat: u32,
    pub sixteenth: u32,
    /// How far through the bar, in `0.0..1.0`.
    pub bar_phase: f32,
}

impl MusicalTime {
    /// Converts a PPQ position into bars, beats and sixteenths.
    pub fn from_ppq(ppq: f64, beats_per_bar: u32) -> Self {
        let per_bar = beats_per_bar.max(1) as f64;
        let ppq = ppq.max(0.0);
        let in_bar = ppq % per_bar;
        Self {
            bar: ((ppq / per_bar) as u32).saturating_add(1),
            beat: (in_bar as u32).saturating_add(1),
            sixteenth: ((in_bar.fract() * 4.0) as u32).saturating_add(1),
            bar_phase: (in_bar / per_bar) as f32,
        }
    }
}

impl fmt::Display for MusicalTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03}.{}.{}", self.bar, self.beat, self.sixteenth)
    }
}

#[cfg(test)]
mod tests {
    use super::{MusicalTime, Transport};

    #[test]
    fn the_default_transport_is_stopped_at_120_bpm_in_four_four() {
        let transport = Transport::default();
        assert!(!transport.playing);
        assert_eq!(transport.bpm, 120.0);
        assert_eq!(transport.beats_per_bar, 4);
        assert_eq!(transport.ppq, 0.0);
        assert!(!transport.locked);
    }

    #[test]
    fn musical_time_counts_bars_beats_and_sixteenths_from_one() {
        assert_eq!(
            MusicalTime::from_ppq(0.0, 4),
            MusicalTime {
                bar: 1,
                beat: 1,
                sixteenth: 1,
                bar_phase: 0.0,
            }
        );
        let at = MusicalTime::from_ppq(17.5, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (5, 2, 3));
    }

    #[test]
    fn musical_time_honours_a_three_four_bar() {
        let at = MusicalTime::from_ppq(7.0, 3);
        assert_eq!((at.bar, at.beat), (3, 2));
    }

    #[test]
    fn musical_time_displays_as_bar_dot_beat_dot_sixteenth() {
        assert_eq!(MusicalTime::from_ppq(16.25, 4).to_string(), "005.1.2");
    }

    #[test]
    fn a_negative_position_clamps_to_the_start_rather_than_wrapping() {
        let at = MusicalTime::from_ppq(-3.0, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (1, 1, 1));
    }

    #[test]
    fn large_ppq_values_do_not_panic() {
        let _ = MusicalTime::from_ppq(1e20, 4);
        let _ = MusicalTime::from_ppq(f64::MAX, 4);
        let _ = MusicalTime::from_ppq(1e20, 3);
    }
}
