//! `Envelope`: an ADSR envelope driven by a gate inlet.
//!
//! The gate is read once per tick as an ordinary boolean inlet — no sub-tick
//! offset. Routing individual MIDI events onto graph inlets is `sway-events`
//! territory, still out of scope; what that costs is exact sub-tick retrigger
//! timing, and what it buys is a node whose gate is an ordinary connection.
//!
//! Time arrives on an inlet, matching `Oscillator`. The node therefore reads
//! nothing outside the graph, the same inlets and state always produce the
//! same outlet, and the source of time is a connection the author can see and
//! change — in practice `MidiTime`. An envelope whose `time` inlet is
//! unconnected holds still rather than free-running, which is the visible
//! difference from the version that accumulated `Time<Fixed>` itself.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

/// The four ADSR parameters, as [`adsr_unscaled`] takes them.
#[derive(Debug, Clone, Copy)]
pub struct EnvelopeParams {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

/// The envelope's level at `now`, given when the gate went on and (if it has)
/// off. Unscaled: velocity is applied by the caller.
///
/// A pure function of its arguments, kept separate from the node so the shape
/// of the curve can be asserted without a graph.
pub fn adsr_unscaled(gate_on: f32, gate_off: Option<f32>, now: f32, params: EnvelopeParams) -> f32 {
    let attack = params.attack.max(0.0);
    let decay = params.decay.max(0.0);
    let release = params.release.max(0.0);
    let level_while_gated = |at: f32| {
        let elapsed = at - gate_on;
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
            let elapsed = now - off;
            if release == 0.0 || elapsed >= release {
                0.0
            } else {
                level_while_gated(off) * (1.0 - elapsed / release)
            }
        }
    }
}

/// [`Envelope`]'s inlets.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeIn {
    /// The time base the gate's timestamps are taken against. Connect a time
    /// source — `MidiTime`, in practice.
    pub time: f32,
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
            time: 0.0,
            gate: false,
            velocity: 1.0,
            attack: 0.05,
            decay: 0.08,
            sustain: 0.4,
            release: 0.1,
        }
    }
}

/// [`Envelope`]'s state: the gate's on/off timestamps, in the inlet's own
/// time base.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeState {
    pub gate_on: Option<f32>,
    pub gate_off: Option<f32>,
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
    fn evaluate(&mut self, _world: &World) {
        let now = self.inlets.time;

        if self.inlets.gate {
            if self.state.gate_on.is_none() {
                self.state.gate_on = Some(now);
                self.state.gate_off = None;
            }
        } else if self.state.gate_on.is_some() && self.state.gate_off.is_none() {
            self.state.gate_off = Some(now);
        }

        let params = EnvelopeParams {
            attack: self.inlets.attack,
            decay: self.inlets.decay,
            sustain: self.inlets.sustain,
            release: self.inlets.release,
        };
        self.outlets.out = self.state.gate_on.map_or(0.0, |on| {
            adsr_unscaled(on, self.state.gate_off, now, params) * self.inlets.velocity
        });
    }
}

#[cfg(test)]
mod tests {
    use bevy_reflect::TypeRegistry;
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing;
    use sway_graph::graph::{Graph, Node, Part};

    use super::*;

    const PARAMS: EnvelopeParams = EnvelopeParams {
        attack: 0.05,
        decay: 0.08,
        sustain: 0.4,
        release: 0.1,
    };

    fn envelope_graph(node: Envelope) -> (Graph, sway_graph::graph::NodeId, World) {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Envelope>(&mut registry);
        let world = testing::trace_world(registry);
        let mut graph = Graph::default();
        let id = graph.insert(Node::of(node));
        (graph, id, world)
    }

    #[test]
    fn an_unset_gate_stays_silent() {
        let (mut graph, node, world) = envelope_graph(Envelope::default());

        for tick in 0..5 {
            testing::set_field(&mut graph, node, "time", &(tick as f32 / 120.0));
            testing::tick_once(&mut graph, &world);
        }

        assert_eq!(
            testing::read_field::<f32>(&graph, node, Part::Outlets, "out"),
            0.0
        );
    }

    /// Raising the gate mid-run matches `adsr_unscaled` fed the same on/off
    /// timestamps against the inlet's time base.
    #[test]
    fn the_gate_reproduces_adsr_unscaled_against_the_time_inlet() {
        let (mut graph, node, world) = envelope_graph(Envelope::default());

        let dt = 1.0 / testing::TICK_HZ as f32;
        let mut gate_on: Option<f32> = None;
        let mut gate_off: Option<f32> = None;

        for tick in 0..60 {
            let gate = (10..40).contains(&tick);
            let now = tick as f32 * dt;
            testing::set_field(&mut graph, node, "gate", &gate);
            testing::set_field(&mut graph, node, "time", &now);

            testing::tick_once(&mut graph, &world);
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

    #[test]
    fn the_same_inlets_and_state_give_the_same_outlet() {
        // A pure function of its inlets and state: two nodes fed identical
        // values produce identical outlets, and nothing outside the graph is
        // read to get there.
        let held = Envelope {
            inlets: EnvelopeIn {
                time: 0.2,
                gate: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let (mut first, a, world) = envelope_graph(held);
        let (mut second, b, _) = envelope_graph(Envelope {
            inlets: EnvelopeIn {
                time: 0.2,
                gate: true,
                ..Default::default()
            },
            ..Default::default()
        });

        testing::tick_once(&mut first, &world);
        testing::tick_once(&mut second, &world);

        assert_eq!(
            testing::read_field::<f32>(&first, a, Part::Outlets, "out"),
            testing::read_field::<f32>(&second, b, Part::Outlets, "out"),
        );
    }

    #[test]
    fn an_unconnected_time_inlet_holds_the_envelope_still() {
        // The visible consequence of taking time as an inlet: with no time
        // source wired in, the envelope does not free-run.
        let (mut graph, node, world) = envelope_graph(Envelope {
            inlets: EnvelopeIn {
                gate: true,
                ..Default::default()
            },
            ..Default::default()
        });

        testing::tick_once(&mut graph, &world);
        let first = testing::read_field::<f32>(&graph, node, Part::Outlets, "out");
        for _ in 0..20 {
            testing::tick_once(&mut graph, &world);
        }

        assert_eq!(
            testing::read_field::<f32>(&graph, node, Part::Outlets, "out"),
            first,
            "time never advanced, so neither did the envelope"
        );
    }

    #[test]
    fn retiming_is_authored_rather_than_built_in() {
        // Feeding the same gate against a time base running twice as fast
        // reaches the same point on the curve in half the ticks — with no
        // change to the node kind.
        let (mut slow, slow_node, world) = envelope_graph(Envelope {
            inlets: EnvelopeIn {
                gate: true,
                ..Default::default()
            },
            ..Default::default()
        });
        let (mut fast, fast_node, _) = envelope_graph(Envelope {
            inlets: EnvelopeIn {
                gate: true,
                ..Default::default()
            },
            ..Default::default()
        });

        // Both time bases start at 0 and end at 0.040; the fast one gets
        // there in half the ticks.
        for tick in 0..9 {
            testing::set_field(&mut slow, slow_node, "time", &(tick as f32 * 0.005));
            testing::tick_once(&mut slow, &world);
        }
        for tick in 0..5 {
            testing::set_field(&mut fast, fast_node, "time", &(tick as f32 * 0.010));
            testing::tick_once(&mut fast, &world);
        }

        assert!(
            (testing::read_field::<f32>(&slow, slow_node, Part::Outlets, "out")
                - testing::read_field::<f32>(&fast, fast_node, Part::Outlets, "out"))
            .abs()
                < 1e-5,
        );
    }

    // --- the curve itself -------------------------------------------------

    #[test]
    fn an_earlier_gate_is_further_into_its_attack() {
        let early = adsr_unscaled(0.0001, None, 1.0 / 120.0, PARAMS);
        let late = adsr_unscaled(0.0080, None, 1.0 / 120.0, PARAMS);
        assert!(early > late);
    }

    #[test]
    fn the_curve_walks_attack_decay_sustain_and_release() {
        let params = EnvelopeParams {
            attack: 0.1,
            decay: 0.1,
            sustain: 0.5,
            release: 0.1,
        };
        assert_eq!(adsr_unscaled(0.0, None, 0.0, params), 0.0, "the very start");
        assert!(
            (adsr_unscaled(0.0, None, 0.05, params) - 0.5).abs() < 1e-6,
            "attack"
        );
        assert!(
            (adsr_unscaled(0.0, None, 0.1, params) - 1.0).abs() < 1e-6,
            "peak"
        );
        assert!(
            (adsr_unscaled(0.0, None, 0.15, params) - 0.75).abs() < 1e-6,
            "decay"
        );
        assert!(
            (adsr_unscaled(0.0, None, 1.0, params) - 0.5).abs() < 1e-6,
            "sustain"
        );
        assert!(
            (adsr_unscaled(0.0, Some(1.0), 1.05, params) - 0.25).abs() < 1e-6,
            "release"
        );
        assert_eq!(
            adsr_unscaled(0.0, Some(1.0), 1.1, params),
            0.0,
            "silent after"
        );
    }

    #[test]
    fn zero_length_stages_do_not_divide_by_zero() {
        let instant = EnvelopeParams {
            attack: 0.0,
            decay: 0.0,
            sustain: 0.7,
            release: 0.0,
        };
        assert_eq!(adsr_unscaled(0.0, None, 0.0, instant), 0.7);
        assert_eq!(adsr_unscaled(0.0, Some(0.0), 0.001, instant), 0.0);
    }
}
