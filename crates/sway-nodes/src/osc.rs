//! The beat-locked LFO, as a behaviour. Spec §2.4.
//!
//! This is the third case in the behaviour table: its `amplitude` can be
//! driven by a wire, so it must compute between that propagation and the
//! propagation of its own output. A plain Bevy system runs at a fixed point
//! in the schedule and cannot.

use bevy::prelude::*;
use bevy_ecs::change_detection::DetectChangesMut;
use sway_graph::{EditorPos, TickCtx, Transport, TransportTime};

use crate::field_wire::field_wire;
use crate::lfo::{wave, Waveform};
use crate::outputs::FloatOut;

/// Period in beats, plus shape, phase offset in cycles, and amplitude.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(FloatOut, EditorPos)]
pub struct Lfo {
    pub beats: f32,
    pub shape: Waveform,
    pub phase: f32,
    pub amplitude: f32,
}

impl Default for Lfo {
    fn default() -> Self {
        Self { beats: 4.0, shape: Waveform::Sine, phase: 0.0, amplitude: 1.0 }
    }
}

/// Absolute beat position, never an accumulator — so a tempo change, a
/// reposition and a dropped tick all leave this correct. Carried over from
/// `SyncLfo::tick`.
pub fn lfo_behaviour(world: &mut World, entity: Entity, _ctx: &TickCtx) {
    let beats = world.resource::<Time<Transport>>().beats();
    let Some(lfo) = world.get::<Lfo>(entity).copied() else {
        return;
    };
    // An authored zero or negative period holds still rather than dividing:
    // the behaviour is infallible.
    let p = if lfo.beats > 0.0 {
        (beats / lfo.beats as f64 + lfo.phase as f64).rem_euclid(1.0) as f32
    } else {
        lfo.phase.rem_euclid(1.0)
    };
    let value = wave(lfo.shape, p) * lfo.amplitude;

    if let Some(mut out) = world.get_mut::<FloatOut>(entity) {
        out.set_if_neq(FloatOut(value));
    }
}

field_wire!(
    /// Drives `Lfo.amplitude`.
    AmplitudeFrom / DrivesAmplitude,
    FloatOut => Lfo,
    "amplitude",
    |t| &mut t.amplitude,
    |s| s.0
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::TranslationFrom;
    use crate::value::{Vec3Value, Vec3YFrom};
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::WiresPlugin;

    /// At beat 0 with `phase = 0.25`, a sine LFO's phase is exactly 0.25 and
    /// `wave` returns 1.0 — so every value below is exact, and `Time<Transport>`
    /// needs no advancing.
    fn slice_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy::time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(WiresPlugin)
            .add_plugins(crate::WireNodesPlugin);
        // Frame 0 starts with an empty fixed-time accumulator.
        app.update();
        app
    }

    fn lfo(phase: f32, amplitude: f32) -> Lfo {
        Lfo { beats: 4.0, shape: Waveform::Sine, phase, amplitude }
    }

    #[test]
    fn the_amplitude_wire_never_writes_an_equal_value() {
        assert_writes_only_on_change::<AmplitudeFrom>(
            FloatOut(1.0),
            FloatOut(2.0),
            Lfo::default(),
        );
    }

    #[test]
    fn a_modulated_lfo_reaches_a_transform_in_one_tick() {
        // The chain the design turns on, now one node longer (D5):
        //   Lfo A -> Lfo B.amplitude -> B computes -> Vec3.y -> Transform
        // A's output is 1.0 * 0.5 = 0.5, which becomes B's amplitude, so B's
        // output is 0.5, which becomes the vector's y. If the order were wrong,
        // B would still hold its authored amplitude of 0.0 and y would be 0.0.
        let mut app = slice_app();
        let a = app.world_mut().spawn(lfo(0.25, 0.5)).id();
        let b = app.world_mut().spawn(lfo(0.25, 0.0)).id();
        let vector = app.world_mut().spawn(Vec3Value::default()).id();
        let mesh = app.world_mut().spawn(Transform::default()).id();
        app.world_mut().entity_mut(b).insert(AmplitudeFrom(a));
        app.world_mut().entity_mut(vector).insert(Vec3YFrom(b));
        app.world_mut().entity_mut(mesh).insert(TranslationFrom(vector));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(a).map(|o| o.0), Some(0.5));
        assert_eq!(app.world().get::<Lfo>(b).map(|l| l.amplitude), Some(0.5));
        assert_eq!(
            app.world().get::<Transform>(mesh).map(|t| t.translation.y),
            Some(0.5),
            "the whole chain must land in ONE tick"
        );
    }

    #[test]
    fn one_producer_fans_out_to_two_consumers() {
        let mut app = slice_app();
        let a = app.world_mut().spawn(lfo(0.25, 0.5)).id();
        let vector = app.world_mut().spawn(Vec3Value::default()).id();
        app.world_mut().entity_mut(vector).insert(Vec3YFrom(a));
        let x = app.world_mut().spawn(Transform::default()).id();
        let y = app.world_mut().spawn(Transform::default()).id();
        app.world_mut().entity_mut(x).insert(TranslationFrom(vector));
        app.world_mut().entity_mut(y).insert(TranslationFrom(vector));

        app.update();

        assert_eq!(app.world().get::<Transform>(x).map(|t| t.translation.y), Some(0.5));
        assert_eq!(app.world().get::<Transform>(y).map(|t| t.translation.y), Some(0.5));
    }

    #[test]
    fn a_child_of_wire_parents_without_any_propagation() {
        // Spec §2.3: nothing inserts ChildOf but the author, and Bevy's hooks
        // maintain Children.
        let mut app = slice_app();
        let group = app.world_mut().spawn(Transform::default()).id();
        let child = app.world_mut().spawn(Transform::default()).id();
        app.world_mut().entity_mut(child).insert(ChildOf(group));

        app.update();

        assert_eq!(app.world().get::<ChildOf>(child).map(|c| c.0), Some(group));
        assert_eq!(
            app.world().get::<Children>(group).map(|c| c.len()),
            Some(1)
        );
    }
}
