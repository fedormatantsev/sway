//! The 24 ppqn tempo smoother: pulses in, seconds-per-pulse and BPM out.
//!
//! Pure. No Bevy, no world, no clock of its own — every time it is handed is
//! absolute seconds on whatever timeline the caller uses. `PulseClock` owns
//! the one instance and uses the fit only as `secs_per_pulse`.
//!
//! Raw pulse timing is too jittery to use directly, so this fits a line to
//! the last [`WINDOW_PULSES`] `(pulse index, arrival time)` pairs by least
//! squares. The slope is seconds per pulse. Musical position is *not*
//! interpolated from this line; [`crate::PulseClock`] counts pulses.

use std::collections::VecDeque;

/// MIDI clock resolution: 24 pulses per quarter note.
pub const PULSES_PER_QUARTER: u32 = 24;

/// How many pulses the fit spans — two beats. Long enough to average out
/// jitter, short enough that a tempo change settles in under a second.
pub const WINDOW_PULSES: usize = 48;

/// Below this many samples there is no lock: a fit over three noisy points
/// is worse than freewheeling at the last known tempo.
const MIN_SAMPLES: usize = 8;

/// A gap longer than this many pulse periods abandons the fit. One beat of
/// silence means the clock stopped, not that pulses were dropped, and
/// inferring 24+ missing indices from a stale period is guesswork.
const MAX_GAP_PULSES: f64 = PULSES_PER_QUARTER as f64;

/// Fits tempo and phase to a 24 ppqn pulse train.
#[derive(Debug, Default)]
pub struct ClockEstimator {
    /// `(pulse index, arrival time)`, oldest first, at most `WINDOW_PULSES`.
    samples: VecDeque<(f64, f64)>,
    /// Index to assign the next pulse, before gap inference.
    next_index: f64,
    last_time: Option<f64>,
    /// Current fit: `time = slope * index + intercept`. `None` until the fit
    /// is both long enough and non-degenerate.
    fit: Option<(f64, f64)>,
}

impl ClockEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Abandons the current fit.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.next_index = 0.0;
        self.last_time = None;
        self.fit = None;
    }

    /// Records one clock pulse, arriving at absolute time `t`.
    pub fn push_pulse(&mut self, t: f64) {
        if !t.is_finite() {
            return;
        }

        // A pulse that does not land strictly after the previous one carries no
        // new phase information the fit can use — it is indistinguishable from
        // "the same pulse observed again." This happens for real: a caller can
        // hand several same-frame MIDI events a shared "now" timestamp (see
        // `sway-app`'s `feed_midi`, which does exactly this for host_time == 0
        // messages), collapsing them onto one instant. Blindly assigning such a
        // pulse a fresh index would tell the fit "a pulse happened in zero
        // time," dragging the slope toward zero — and because the *next*
        // pulse's index is inferred from this fit, the error compounds pulse
        // over pulse rather than averaging out. Dropping it here is lossless:
        // tempo depends only on the fitted line, never on the index
        // bookkeeping, and the next pulse that *does* advance time recovers
        // whatever this one couldn't distinguish via the same elapsed-pulses-
        // across-a-gap inference already used for genuinely missed pulses.
        if self.last_time.is_some_and(|last| t <= last) {
            return;
        }

        // Infer how many pulse periods elapsed rather than counting pulses,
        // so a dropped pulse does not shear the fit.
        let index = match (self.last_time, self.fit) {
            (Some(last), Some((slope, _))) if slope > 0.0 => {
                let elapsed = ((t - last) / slope).round().max(1.0);
                if elapsed > MAX_GAP_PULSES {
                    self.reset();
                    0.0
                } else {
                    self.next_index + elapsed - 1.0
                }
            }
            _ => self.next_index,
        };

        self.samples.push_back((index, t));
        if self.samples.len() > WINDOW_PULSES {
            self.samples.pop_front();
        }
        self.next_index = index + 1.0;
        self.last_time = Some(t);
        self.refit();
    }

    fn refit(&mut self) {
        if self.samples.len() < MIN_SAMPLES {
            self.fit = None;
            return;
        }
        let n = self.samples.len() as f64;
        let mean_x = self.samples.iter().map(|&(x, _)| x).sum::<f64>() / n;
        let mean_y = self.samples.iter().map(|&(_, y)| y).sum::<f64>() / n;
        let mut covariance = 0.0;
        let mut variance = 0.0;
        for &(x, y) in &self.samples {
            covariance += (x - mean_x) * (y - mean_y);
            variance += (x - mean_x) * (x - mean_x);
        }
        // A degenerate fit (every pulse at one instant, or a non-positive
        // period) is refused rather than propagated: the tick is infallible
        // and a NaN period would reach `Duration::from_secs_f64`.
        let slope = covariance / variance;
        self.fit = (variance > 0.0 && slope.is_finite() && slope > 0.0)
            .then_some((slope, mean_y - slope * mean_x));
    }

    /// Whether there is a usable fit.
    pub fn is_locked(&self) -> bool {
        self.fit.is_some()
    }

    /// Estimated pulse period, in seconds.
    pub fn secs_per_pulse(&self) -> Option<f64> {
        self.fit.map(|(slope, _)| slope)
    }

    /// Estimated beat period, in seconds.
    pub fn secs_per_beat(&self) -> Option<f64> {
        self.secs_per_pulse()
            .map(|spp| spp * PULSES_PER_QUARTER as f64)
    }

    pub fn bpm(&self) -> Option<f64> {
        self.secs_per_beat().map(|spb| 60.0 / spb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPP_120: f64 = 0.5 / PULSES_PER_QUARTER as f64;

    /// A deterministic pseudo-random jitter source. No `rand` dependency:
    /// a golden-trace project cannot afford a non-reproducible test.
    struct Lcg(u64);
    impl Lcg {
        fn next_signed(&mut self, magnitude: f64) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let unit = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
            (unit * 2.0 - 1.0) * magnitude
        }
    }

    fn steady(estimator: &mut ClockEstimator, pulses: usize, spp: f64, start: f64) -> f64 {
        let mut t = start;
        for _ in 0..pulses {
            estimator.push_pulse(t);
            t += spp;
        }
        t
    }

    #[test]
    fn a_clean_train_recovers_its_tempo_exactly() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 48, SPP_120, 0.0);
        assert!((estimator.bpm().expect("locked") - 120.0).abs() < 1e-9);
    }

    #[test]
    fn fewer_than_the_minimum_pulses_is_not_a_lock() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 3, SPP_120, 0.0);
        assert!(!estimator.is_locked());
        assert_eq!(estimator.bpm(), None);
    }

    #[test]
    fn jitter_averages_out_across_the_window() {
        let mut estimator = ClockEstimator::new();
        let mut lcg = Lcg(0x5EED);
        let mut t = 0.0;
        for _ in 0..96 {
            // ±1ms of jitter, which is worse than USB MIDI in practice.
            estimator.push_pulse(t + lcg.next_signed(0.001));
            t += SPP_120;
        }
        let bpm = estimator.bpm().expect("locked");
        assert!((bpm - 120.0).abs() < 1.0, "jittered fit gave {bpm} BPM");
    }

    #[test]
    fn a_tempo_change_settles_within_one_window() {
        let mut estimator = ClockEstimator::new();
        let end = steady(&mut estimator, 48, SPP_120, 0.0);
        let spp_140 = (60.0 / 140.0) / PULSES_PER_QUARTER as f64;
        steady(&mut estimator, WINDOW_PULSES, spp_140, end);
        let bpm = estimator.bpm().expect("locked");
        assert!(
            (bpm - 140.0).abs() < 1.0,
            "after one full window: {bpm} BPM"
        );
    }

    #[test]
    fn a_few_dropped_pulses_do_not_shear_the_fit() {
        // The index is inferred from elapsed time, not counted. Without that,
        // three missing pulses would read as the tempo jumping.
        let mut estimator = ClockEstimator::new();
        let mut t = steady(&mut estimator, 48, SPP_120, 0.0);
        t += 3.0 * SPP_120; // three pulses lost in transit
        steady(&mut estimator, 24, SPP_120, t);
        let bpm = estimator.bpm().expect("locked");
        assert!(
            (bpm - 120.0).abs() < 0.5,
            "dropped pulses moved the tempo to {bpm}"
        );
    }

    #[test]
    fn a_gap_longer_than_a_beat_restarts_the_fit() {
        let mut estimator = ClockEstimator::new();
        let t = steady(&mut estimator, 48, SPP_120, 0.0);
        estimator.push_pulse(t + 3.0); // three seconds of silence
        assert!(!estimator.is_locked(), "one pulse is not a lock");
    }

    #[test]
    fn reset_drops_the_lock() {
        let mut estimator = ClockEstimator::new();
        steady(&mut estimator, 48, SPP_120, 0.0);
        estimator.reset();
        assert!(!estimator.is_locked());
        assert_eq!(estimator.bpm(), None);
    }

    #[test]
    fn two_pulses_at_the_same_instant_do_not_produce_a_nan() {
        // A duplicated timestamp gives a degenerate fit. The estimator must
        // refuse it rather than hand a NaN period to the tick.
        let mut estimator = ClockEstimator::new();
        for _ in 0..8 {
            estimator.push_pulse(1.0);
        }
        assert!(estimator.bpm().is_none_or(|bpm| bpm.is_finite()));
        assert!(estimator.secs_per_pulse().is_none_or(f64::is_finite));
    }

    #[test]
    fn same_instant_duplicate_pulses_do_not_corrupt_the_fit() {
        // This is what `feed_midi`'s host_time == 0 handling produces when more
        // than one real clock pulse is queued when a frame drains: every pulse
        // in that batch shares the identical mapped timestamp. Before the fix,
        // this fed the regression a "pulse happened in zero time" sample, and
        // because the next pulse's index was inferred from the (now wrong) fit,
        // the error compounded instead of averaging out.
        let mut estimator = ClockEstimator::new();
        let mut t = 0.0;
        for _ in 0..20 {
            // A correctly-spaced pulse...
            estimator.push_pulse(t);
            // ...followed by a same-instant duplicate, as a collapsed batch
            // would produce.
            estimator.push_pulse(t);
            estimator.push_pulse(t);
            t += SPP_120;
        }
        let bpm = estimator.bpm().expect("locked");
        assert!(
            (bpm - 120.0).abs() < 1.0,
            "duplicate pulses corrupted the fit: {bpm} BPM instead of ~120"
        );
    }

    #[test]
    fn duplicate_stragglers_are_dropped_silently_without_resetting_the_fit() {
        // A strictly-increasing train interleaved with occasional exact-
        // duplicate stragglers should still lock to the correct tempo, and
        // dropping the duplicates must be silent and cheap: it must not walk
        // the MAX_GAP_PULSES reset path (which only genuinely-long gaps
        // should trigger).
        let mut estimator = ClockEstimator::new();
        let mut t = 0.0;
        for i in 0..48 {
            estimator.push_pulse(t);
            if i % 5 == 0 {
                // A straggler: the same pulse observed again at the same
                // instant, as a collapsed batch would produce.
                estimator.push_pulse(t);
            }
            t += SPP_120;
        }
        assert!(
            estimator.is_locked(),
            "duplicates must never trigger a reset"
        );

        let bpm = estimator.bpm().expect("locked");
        assert!(
            (bpm - 120.0).abs() < 1.0,
            "stragglers moved the tempo to {bpm}"
        );

        // A few more duplicates after lock must still leave the fit locked.
        estimator.push_pulse(t - SPP_120);
        estimator.push_pulse(t - SPP_120);
        assert!(
            estimator.is_locked(),
            "duplicates after lock must not reset"
        );
    }
}
