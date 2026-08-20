//! `Envelope`, a new node kind wrapping the wire-model's pure
//! `crate::envelope::{adsr_unscaled, EnvelopeParams}` (previously exercised
//! only via `tests/traces.rs`, never wired into the old entity/component
//! graph as a `Behaviour`).
//!
//! The old `envelope_tick` drove gate transitions from discrete, sub-tick-
//! offset MIDI note events (`&[(f32, bool, NoteMsg)]`) — routing individual
//! MIDI events onto graph inlets is `sway-events` territory, explicitly out
//! of scope for this change (`design.md` Non-Goals). This node instead reads
//! an ordinary boolean `gate` inlet once per tick (no sub-tick offset), which
//! is the natural shape for a value-node inlet. `state.gate_on` /
//! `state.gate_off` are exactly the memory tasks 4.2 asks to move into
//! `state`, now keyed against a self-accumulated `state.now` — the node's own
//! running clock, built the same way `Lfo`'s `state.elapsed` is — rather than
//! the tick's wall time, so the node needs nothing beyond `Time<Fixed>`'s
//! `dt` from `&World`.
//!
//! **Open decision**: this trades exact sub-tick retrigger timing (what
//! `envelope-retrigger.out.ron` golden-traces) for a plain per-tick gate.
//! Feeding sub-tick MIDI timing back in is left to `sway-events`.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::{Fixed, Time};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::envelope::{EnvelopeParams, adsr_unscaled};

/// [`Envelope`]'s inlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeIn {
    /// Held true while the note sounds.
    pub gate: bool,
    /// Scales the whole envelope, standing in for the old per-note velocity.
    pub velocity: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Default for EnvelopeIn {
    fn default() -> Self {
        Self {
            gate: false,
            velocity: 1.0,
            attack: 0.05,
            decay: 0.08,
            sustain: 0.4,
            release: 0.1,
        }
    }
}

/// [`Envelope`]'s state: the gate's on/off timestamps against the node's own
/// running clock, and that clock itself.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeState {
    pub gate_on: Option<f64>,
    pub gate_off: Option<f64>,
    pub now: f64,
}

/// [`Envelope`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeOut {
    pub out: f32,
}

/// An ADSR envelope driven by a gate inlet.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Envelope {
    pub inlets: EnvelopeIn,
    pub state: EnvelopeState,
    pub outlets: EnvelopeOut,
}

impl NodeKind for Envelope {
    fn evaluate(&mut self, world: &World) {
        let dt = world
            .get_resource::<Time<Fixed>>()
            .map_or(0.0, |time| time.delta_secs());
        self.state.now += dt as f64;

        if self.inlets.gate {
            if self.state.gate_on.is_none() {
                self.state.gate_on = Some(self.state.now);
                self.state.gate_off = None;
            }
        } else if self.state.gate_on.is_some() && self.state.gate_off.is_none() {
            self.state.gate_off = Some(self.state.now);
        }

        let params = EnvelopeParams {
            attack: self.inlets.attack,
            decay: self.inlets.decay,
            sustain: self.inlets.sustain,
            release: self.inlets.release,
        };
        self.outlets.out = self.state.gate_on.map_or(0.0, |on| {
            adsr_unscaled(on, self.state.gate_off, self.state.now, params) * self.inlets.velocity
        });
    }
}

#[cfg(test)]
mod tests {
    
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::{Graph, Node, Part};

    use super::*;
    use sway_graph::graph::testing;

    const PARAMS: EnvelopeParams = EnvelopeParams {
        attack: 0.05,
        decay: 0.08,
        sustain: 0.4,
        release: 0.1,
    };

    #[test]
    fn an_unset_gate_stays_silent() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Envelope>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Envelope::default()));

        for _ in 0..5 {
            testing::tick_once(&mut graph, &world);
        }

        assert_eq!(testing::read_field::<f32>(&graph, node, Part::Outlets, "out"), 0.0);
    }

    /// Raising the gate mid-run matches `adsr_unscaled` fed the same on/off
    /// timestamps against the node's own accumulated clock — the same ADSR
    /// math `tests/traces.rs`'s envelope cases exercise, now reached through
    /// a gate inlet and a real graph tick instead of MIDI note-event offsets.
    #[test]
    fn the_gate_reproduces_adsr_unscaled_against_the_nodes_own_clock() {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Envelope>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Envelope {
                inlets: EnvelopeIn {
                    gate: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        ));

        let dt = 1.0 / testing::TICK_HZ;
        let mut now = 0.0_f64;
        let mut gate_on: Option<f64> = None;
        let mut gate_off: Option<f64> = None;

        for tick in 0..60 {
            let gate = (10..40).contains(&tick);
            testing::set_field(&mut graph, node, "gate", &gate);

            testing::tick_once(&mut graph, &world);
            now += dt;
            if gate {
                gate_on.get_or_insert(now);
            } else if gate_on.is_some() {
                gate_off.get_or_insert(now);
            }

            let expected = gate_on.map_or(0.0, |on| adsr_unscaled(on, gate_off, now, PARAMS));
            let actual = testing::read_field::<f32>(&graph, node, Part::Outlets, "out");
            assert!(
                (actual - expected).abs() < 1e-5,
                "tick {tick}: actual={actual} expected={expected}"
            );
        }
    }
}
