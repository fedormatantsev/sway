//! `Time<Transport>` — beat time as a Bevy clock (parent §2.7).
//!
//! `Time<T>` is generic over a clock, so the transport *is* one: its elapsed
//! time is measured in beats and its advance per tick is whatever the phase
//! estimator says. A tempo-synced node takes `Res<Time<Transport>>` and is
//! otherwise an ordinary node; stop is a clock that stops advancing.
//!
//! Nothing here knows about MIDI. `sway-graph` must not learn what a pulse is
//! (parent §2), and the editor — which may not depend on `sway-nodes` — reads
//! this type directly.
//!
//! **The clock never rewinds.** `Time::advance_by` takes a `Duration`, so a
//! Start cannot reset it. Musical position is `elapsed - origin_beats`, and
//! repositioning moves the origin.

use core::fmt;

use bevy_reflect::Reflect;
use bevy_time::Time;

/// Whether beat time is advancing.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    #[default]
    Stopped,
    Playing,
}

/// The transport clock's context.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub state: TransportState,
    /// Best current estimate of one beat's duration, in seconds. Survives a
    /// clock dropout: freewheeling advances at exactly this rate.
    pub secs_per_beat: f64,
    /// Beats in a bar. MIDI clock carries no time signature, so this is
    /// authored — and it lives here rather than on a node so the readout and
    /// every transport-aware node agree about where a bar starts.
    pub beats_per_bar: u32,
    /// The clock's elapsed beats at musical position zero.
    pub origin_beats: f64,
    /// Whether the phase estimator currently has a lock. Purely informational
    /// — freewheeling and locked both advance beat time.
    pub locked: bool,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            state: TransportState::Stopped,
            secs_per_beat: 0.5, // 120 BPM
            beats_per_bar: 4,
            origin_beats: 0.0,
            locked: false,
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
    /// Converts a beat position into bars, beats and sixteenths.
    ///
    /// A negative position clamps to the start: `reposition` can briefly
    /// leave the origin ahead of elapsed, and bar 0 on the readout is worse
    /// than a clamp.
    pub fn from_beats(beats: f64, beats_per_bar: u32) -> Self {
        let per_bar = beats_per_bar.max(1) as f64;
        let beats = beats.max(0.0);
        let in_bar = beats % per_bar;
        Self {
            bar: ((beats / per_bar) as u32).saturating_add(1),
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

/// Beat-time accessors on `Time<Transport>`.
pub trait TransportTime {
    fn transport(&self) -> &Transport;
    fn transport_mut(&mut self) -> &mut Transport;
    /// Musical position, in beats since the last reposition. Never negative.
    fn beats(&self) -> f64;
    /// The clock's own elapsed beats, before the origin is subtracted. This
    /// is monotone, and it is what a beat-boundary search works in.
    fn beats_total(&self) -> f64;
    fn bpm(&self) -> f64;
    fn state(&self) -> TransportState;
    fn is_playing(&self) -> bool;
    fn position(&self) -> MusicalTime;
    /// Moves musical position zero so that `beats()` reads `beats`.
    fn reposition(&mut self, beats: f64);
}

impl TransportTime for Time<Transport> {
    fn transport(&self) -> &Transport {
        self.context()
    }

    fn transport_mut(&mut self) -> &mut Transport {
        self.context_mut()
    }

    fn beats(&self) -> f64 {
        (self.beats_total() - self.context().origin_beats).max(0.0)
    }

    fn beats_total(&self) -> f64 {
        self.elapsed_secs_f64()
    }

    fn bpm(&self) -> f64 {
        let spb = self.context().secs_per_beat;
        if spb > 0.0 { 60.0 / spb } else { 0.0 }
    }

    fn state(&self) -> TransportState {
        self.context().state
    }

    fn is_playing(&self) -> bool {
        self.state() == TransportState::Playing
    }

    fn position(&self) -> MusicalTime {
        MusicalTime::from_beats(self.beats(), self.context().beats_per_bar)
    }

    fn reposition(&mut self, beats: f64) {
        let origin = self.beats_total() - beats;
        self.context_mut().origin_beats = origin;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    fn clock() -> Time<Transport> {
        Time::new_with(Transport::default())
    }

    #[test]
    fn the_default_transport_is_stopped_at_120_bpm_in_four_four() {
        let time = clock();
        assert_eq!(time.state(), TransportState::Stopped);
        assert!((time.bpm() - 120.0).abs() < 1e-9);
        assert_eq!(time.transport().beats_per_bar, 4);
        assert_eq!(time.beats(), 0.0);
    }

    #[test]
    fn position_is_elapsed_minus_the_origin() {
        let mut time = clock();
        time.advance_by(Duration::from_secs_f64(9.0));
        time.reposition(0.0);
        assert!((time.beats() - 0.0).abs() < 1e-9);
        time.advance_by(Duration::from_secs_f64(2.5));
        assert!((time.beats() - 2.5).abs() < 1e-9);
        assert!((time.beats_total() - 11.5).abs() < 1e-9, "the clock itself never rewinds");
    }

    #[test]
    fn a_reposition_can_move_the_origin_forward_or_back() {
        let mut time = clock();
        time.advance_by(Duration::from_secs_f64(16.0));
        time.reposition(4.0);
        assert!((time.beats() - 4.0).abs() < 1e-9);
        time.reposition(0.0);
        assert!((time.beats() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn musical_time_counts_bars_beats_and_sixteenths_from_one() {
        // Beat 0 is bar 1, beat 1, sixteenth 1 — musicians count from one and
        // the readout has to match what the sequencer shows.
        assert_eq!(
            MusicalTime::from_beats(0.0, 4),
            MusicalTime { bar: 1, beat: 1, sixteenth: 1, bar_phase: 0.0 }
        );
        let at = MusicalTime::from_beats(17.5, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (5, 2, 3));
    }

    #[test]
    fn musical_time_honours_a_three_four_bar() {
        let at = MusicalTime::from_beats(7.0, 3);
        assert_eq!((at.bar, at.beat), (3, 2), "beat 7 is bar 3 beat 2 in 3/4");
    }

    #[test]
    fn musical_time_displays_as_bar_dot_beat_dot_sixteenth() {
        assert_eq!(MusicalTime::from_beats(16.25, 4).to_string(), "005.1.2");
    }

    #[test]
    fn a_negative_position_clamps_to_the_start_rather_than_wrapping() {
        // `reposition` can leave the origin ahead of elapsed for one tick.
        // Bar 0 or a negative sixteenth on the readout is worse than a clamp.
        let at = MusicalTime::from_beats(-3.0, 4);
        assert_eq!((at.bar, at.beat, at.sixteenth), (1, 1, 1));
    }

    #[test]
    fn bpm_and_secs_per_beat_are_each_other_inverses() {
        let mut time = clock();
        time.context_mut().secs_per_beat = 60.0 / 137.0;
        assert!((time.bpm() - 137.0).abs() < 1e-9);
    }

    #[test]
    fn the_graph_plugin_inserts_the_transport_clock() {
        let mut app = bevy_app::App::new();
        app.add_plugins(crate::tick::GraphPlugin);
        assert!(app.world().get_resource::<Time<Transport>>().is_some());
    }

    #[test]
    fn large_beat_values_do_not_panic() {
        // Very large beat values should saturate to u32::MAX, not panic.
        // This verifies that saturating_add prevents overflow panics.
        let _ = MusicalTime::from_beats(1e20, 4);
        let _ = MusicalTime::from_beats(f64::MAX, 4);
        let _ = MusicalTime::from_beats(1e20, 3);
        // If we get here without panicking, the test passes.
    }
}
