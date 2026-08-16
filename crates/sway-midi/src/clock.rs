use crate::{ClockEstimator, MidiMessage, PULSES_PER_QUARTER};

pub struct PulseClock {
    estimator: ClockEstimator,
    pulse_index: u64,
    t_last: Option<f64>,
    last_clock_t: Option<f64>,
    playing: bool,
    frozen_ppq: f64,
    beats_per_bar: u32,
}

impl PulseClock {
    pub fn new() -> Self {
        Self {
            estimator: ClockEstimator::new(),
            pulse_index: 0,
            t_last: None,
            last_clock_t: None,
            playing: false,
            frozen_ppq: 0.0,
            beats_per_bar: 4,
        }
    }

    pub fn push(&mut self, t: f64, message: MidiMessage) {
        if !t.is_finite() {
            return;
        }

        if message == MidiMessage::Clock {
            if self.last_clock_t.is_some_and(|last| t <= last) {
                return;
            }
            self.last_clock_t = Some(t);
        }

        match message {
            MidiMessage::Clock if self.playing => {
                self.estimator.push_pulse(t);
                self.pulse_index += 1;
                self.t_last = Some(t);
                self.frozen_ppq = self.pulse_index as f64 / f64::from(PULSES_PER_QUARTER);
            }
            MidiMessage::Clock => self.estimator.push_pulse(t),
            MidiMessage::Start => {
                self.playing = true;
                self.pulse_index = 0;
                self.t_last = Some(t);
                self.frozen_ppq = 0.0;
            }
            MidiMessage::Continue => self.playing = true,
            MidiMessage::Stop => {
                self.frozen_ppq = self.ppq(t);
                self.playing = false;
            }
            MidiMessage::SongPosition { sixteenths } => {
                self.pulse_index = u64::from(sixteenths) * 6;
                self.t_last = Some(t);
                self.frozen_ppq = f64::from(sixteenths) / 4.0;
            }
            _ => {}
        }
    }

    pub fn ppq(&self, t: f64) -> f64 {
        if !self.playing {
            return self.frozen_ppq;
        }
        let Some(t_last) = self.t_last else {
            return self.frozen_ppq;
        };
        let spp = self.estimator.secs_per_pulse().unwrap_or(0.5 / 24.0);
        let frac = if spp > 0.0 { (t - t_last) / spp } else { 0.0 };
        let frac = frac.clamp(0.0, 1.0 - f64::EPSILON);
        (self.pulse_index as f64 + frac) / f64::from(PULSES_PER_QUARTER)
    }

    pub fn bpm(&self) -> f64 {
        self.estimator.bpm().unwrap_or(120.0)
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn locked(&self, t: f64) -> bool {
        let Some(last) = self.last_clock_t else {
            return false;
        };
        let spp = self.estimator.secs_per_pulse().unwrap_or(0.5 / 24.0);
        t - last <= spp
    }

    pub fn beats_per_bar(&self) -> u32 {
        self.beats_per_bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MidiMessage;

    const SPP_120: f64 = 0.5 / 24.0;

    fn play(clock: &mut PulseClock, n: usize, spp: f64, start: f64) -> f64 {
        clock.push(start, MidiMessage::Start);
        let mut t = start;
        for _ in 0..n {
            t += spp;
            clock.push(t, MidiMessage::Clock);
        }
        t
    }

    #[test]
    fn twenty_four_clocks_advance_ppq_by_one_at_pulse_instants() {
        let mut c = PulseClock::new();
        c.push(0.0, MidiMessage::Start);
        let mut t = 0.0;
        for i in 1..=24 {
            t += SPP_120;
            c.push(t, MidiMessage::Clock);
            assert!((c.ppq(t) - i as f64 / 24.0).abs() < 1e-12);
        }
    }

    #[test]
    fn interpolation_stays_inside_the_current_pulse() {
        let mut c = PulseClock::new();
        c.push(0.0, MidiMessage::Start);
        c.push(SPP_120, MidiMessage::Clock);
        let mid = SPP_120 * 1.5;
        let ppq = c.ppq(mid);
        assert!(ppq > 1.0 / 24.0);
        assert!(ppq < 2.0 / 24.0);
    }

    #[test]
    fn one_second_of_silence_does_not_advance_ppq() {
        let mut c = PulseClock::new();
        c.push(0.0, MidiMessage::Start);
        let mut t = 0.0;
        for _ in 0..24 {
            t += SPP_120;
            c.push(t, MidiMessage::Clock);
        }
        let held = c.ppq(t);
        assert!((c.ppq(t + 1.0) - held).abs() < 1.0 / 24.0 + 1e-9);
        assert!(c.ppq(t + 1.0) < held + 0.05);
    }

    #[test]
    fn start_zeros_ppq() {
        let mut c = PulseClock::new();
        let t = play(&mut c, 12, SPP_120, 0.0);
        assert!(c.ppq(t) > 0.0);

        c.push(t + SPP_120, MidiMessage::Start);

        assert_eq!(c.ppq(t + SPP_120), 0.0);
    }

    #[test]
    fn spp_eight_sixteenths_is_two_beats() {
        let mut c = PulseClock::new();
        c.push(0.0, MidiMessage::SongPosition { sixteenths: 8 });
        assert!((c.ppq(0.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn stop_then_continue_resumes_the_same_ppq() {
        let mut c = PulseClock::new();
        let t = play(&mut c, 12, SPP_120, 0.0);
        c.push(t, MidiMessage::Stop);
        let stopped = c.ppq(t);
        assert!(!c.playing());
        assert_eq!(c.ppq(t + 1.0), stopped);

        c.push(t, MidiMessage::Continue);
        assert!(c.playing());
        assert_eq!(c.ppq(t), stopped);
        c.push(t + SPP_120, MidiMessage::Clock);
        assert!((c.ppq(t + SPP_120) - (stopped + 1.0 / 24.0)).abs() < 1e-12);
    }

    #[test]
    fn clocks_while_stopped_update_bpm_only() {
        let mut c = PulseClock::new();
        let mut t = play(&mut c, 12, SPP_120, 0.0);
        c.push(t, MidiMessage::Stop);
        let stopped = c.ppq(t);

        for _ in 0..24 {
            t += SPP_120;
            c.push(t, MidiMessage::Clock);
        }

        assert!(!c.playing());
        assert_eq!(c.ppq(t), stopped);
        assert!((c.bpm() - 120.0).abs() < 1e-9);
    }

    #[test]
    fn duplicate_timestamps_do_not_increment_index() {
        let mut c = PulseClock::new();
        c.push(0.0, MidiMessage::Start);
        c.push(SPP_120, MidiMessage::Clock);
        let ppq = c.ppq(SPP_120);

        c.push(SPP_120, MidiMessage::Clock);

        assert_eq!(c.ppq(SPP_120), ppq);
    }

    #[test]
    fn a_three_pulse_gap_does_not_skip_count() {
        let mut c = PulseClock::new();
        let mut t = play(&mut c, 24, SPP_120, 0.0);
        let before = c.ppq(t);
        assert!(c.locked(t));

        t += 3.0 * SPP_120;
        c.push(t, MidiMessage::Clock);

        assert!((c.ppq(t) - (before + 1.0 / 24.0)).abs() < 1e-12);
    }

    #[test]
    fn beats_per_bar_defaults_to_four() {
        assert_eq!(PulseClock::new().beats_per_bar(), 4);
    }
}
