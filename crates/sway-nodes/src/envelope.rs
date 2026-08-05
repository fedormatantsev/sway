//! ADSR logic as a pure function of absolute gate times.

use crate::NoteMsg;

#[derive(Default, Debug, Clone)]
pub struct EnvelopeState {
    pub gate_on: Option<f64>,
    pub gate_off: Option<f64>,
    pub velocity: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct EnvelopeParams {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

pub fn envelope_tick(
    state: &mut EnvelopeState,
    events: &[(f32, bool, NoteMsg)],
    tick_start: f64,
    dt: f32,
    params: EnvelopeParams,
) -> f32 {
    let mut events = events.to_vec();
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (offset, gate_on, message) in events {
        let at = tick_start + offset as f64;
        if gate_on {
            state.gate_on = Some(at);
            state.gate_off = None;
            state.velocity = message.velocity as f32 / 127.0;
        } else {
            state.gate_off = Some(at);
        }
    }
    state.gate_on.map_or(0.0, |gate_on| {
        adsr_unscaled(gate_on, state.gate_off, tick_start + dt as f64, params) * state.velocity
    })
}

pub fn adsr_unscaled(gate_on: f64, gate_off: Option<f64>, now: f64, params: EnvelopeParams) -> f32 {
    let attack = params.attack.max(0.0);
    let decay = params.decay.max(0.0);
    let release = params.release.max(0.0);
    let level_while_gated = |at: f64| {
        let elapsed = (at - gate_on) as f32;
        if elapsed < 0.0 {
            0.0
        } else if attack == 0.0 {
            if decay == 0.0 {
                params.sustain
            } else if elapsed < decay {
                1.0 - (1.0 - params.sustain) * elapsed / decay
            } else {
                params.sustain
            }
        } else if elapsed < attack {
            elapsed / attack
        } else if decay == 0.0 {
            params.sustain
        } else if elapsed - attack < decay {
            let after_attack = elapsed - attack;
            1.0 - (1.0 - params.sustain) * (after_attack / decay)
        } else {
            params.sustain
        }
    };

    match gate_off {
        None => level_while_gated(now),
        Some(off) if now <= off => level_while_gated(now),
        Some(off) => {
            let elapsed = (now - off) as f32;
            if release == 0.0 || elapsed >= release {
                0.0
            } else {
                level_while_gated(off) * (1.0 - elapsed / release)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS: EnvelopeParams = EnvelopeParams {
        attack: 0.05,
        decay: 0.01,
        sustain: 0.4,
        release: 0.05,
    };

    #[test]
    fn an_earlier_note_is_further_into_its_attack() {
        let note = NoteMsg {
            note: 60,
            velocity: 127,
        };
        let early = envelope_tick(
            &mut EnvelopeState::default(),
            &[(0.0001, true, note.clone())],
            0.0,
            1.0 / 120.0,
            PARAMS,
        );
        let late = envelope_tick(
            &mut EnvelopeState::default(),
            &[(0.0080, true, note)],
            0.0,
            1.0 / 120.0,
            PARAMS,
        );
        assert!(early > late);
    }
}
