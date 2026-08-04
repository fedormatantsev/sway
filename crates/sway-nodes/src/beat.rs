//! Transport-aware nodes (parent §5, M3): a beat time base, a tempo-synced
//! oscillator, and a beat-quantised trigger.
//!
//! All three are pure functions of the beat position `advance_transport`
//! maintains. That is parent §2.2's rule — derive from absolute time, never
//! accumulate — restated in beats: a dropped tick, a tempo change and a
//! reposition all leave the output correct, because nothing here remembers
//! where it was last tick.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::Time;
use sway_graph::{NodeType, PortView, TickCtx, Transport, TransportTime};

use crate::Waveform;
use crate::lfo::wave;

/// Beat time as ports: what bar, beat and sixteenth it is, and how fast.
///
/// No inlets. `beats_per_bar` belongs to `Transport` rather than to this node,
/// because the editor readout and every other transport-aware node have to
/// agree about where a bar starts (Task 4).
#[derive(Reflect, Component, Default)]
pub struct TransportTimeInlets {}

#[derive(Reflect, Default)]
pub struct TransportTimeOutlets {
    /// Musical position, in beats since the last reposition.
    pub beats: f32,
    /// Bar, beat and sixteenth, counted from one, as the sequencer shows them.
    pub bar: f32,
    pub beat: f32,
    pub sixteenth: f32,
    /// How far through the bar, `0.0..1.0`. The one output that is directly
    /// useful as a driver.
    pub bar_phase: f32,
    pub bpm: f32,
    /// 1.0 while playing, 0.0 while stopped.
    pub playing: f32,
}

#[derive(Component, Default)]
pub struct TransportTimeState;

pub struct TransportTimeNode;

impl TransportTimeNode {
    pub const OUT_BEATS: u16 = 0;
    pub const OUT_BAR: u16 = 1;
    pub const OUT_BEAT: u16 = 2;
    pub const OUT_SIXTEENTH: u16 = 3;
    pub const OUT_BAR_PHASE: u16 = 4;
    pub const OUT_BPM: u16 = 5;
    pub const OUT_PLAYING: u16 = 6;
}

impl NodeType for TransportTimeNode {
    type Inlets = TransportTimeInlets;
    type Outlets = TransportTimeOutlets;
    type State = TransportTimeState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("beats", Self::OUT_BEATS),
        ("bar", Self::OUT_BAR),
        ("beat", Self::OUT_BEAT),
        ("sixteenth", Self::OUT_SIXTEENTH),
        ("bar_phase", Self::OUT_BAR_PHASE),
        ("bpm", Self::OUT_BPM),
        ("playing", Self::OUT_PLAYING),
    ];

    fn register(_app: &mut App) {}

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let time = world.resource::<Time<Transport>>();
        let beats = time.beats();
        let at = time.position();
        let bpm = time.bpm();
        let playing = time.is_playing();

        ports.write(Self::OUT_BEATS, beats as f32);
        ports.write(Self::OUT_BAR, at.bar as f32);
        ports.write(Self::OUT_BEAT, at.beat as f32);
        ports.write(Self::OUT_SIXTEENTH, at.sixteenth as f32);
        ports.write(Self::OUT_BAR_PHASE, at.bar_phase);
        ports.write(Self::OUT_BPM, bpm as f32);
        ports.write(Self::OUT_PLAYING, if playing { 1.0f32 } else { 0.0 });
    }
}

/// An oscillator whose period is measured in beats rather than seconds.
///
/// A separate node type from `LFO`, not a mode param on it: a type-selector
/// param is a smell and this is the same argument (parent §2.4). The waveform
/// evaluation is shared, because the only real difference is where phase
/// comes from.
#[derive(Reflect, Component, Default)]
pub struct SyncLfoInlets {
    /// Period, in beats. One bar in 4/4 is 4.0.
    pub beats: f32,
    pub shape: Waveform,
    /// Phase offset, in cycles.
    pub phase: f32,
    pub amplitude: f32,
}

#[derive(Reflect, Default)]
pub struct SyncLfoOutlets {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct SyncLfoState;

pub struct SyncLfo;

impl SyncLfo {
    pub const BEATS: u16 = 0;
    pub const SHAPE: u16 = 1;
    pub const PHASE: u16 = 2;
    pub const AMPLITUDE: u16 = 3;
    pub const OUT_VALUE: u16 = 4;
}

impl NodeType for SyncLfo {
    type Inlets = SyncLfoInlets;
    type Outlets = SyncLfoOutlets;
    type State = SyncLfoState;

    const ORDINALS: &'static [(&'static str, u16)] = &[
        ("beats", Self::BEATS),
        ("shape", Self::SHAPE),
        ("phase", Self::PHASE),
        ("amplitude", Self::AMPLITUDE),
        ("value", Self::OUT_VALUE),
    ];

    fn register(app: &mut App) {
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<Waveform>();
    }

    fn tick(world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let period: f32 = ports.read(Self::BEATS);
        let shape: Waveform = ports.read(Self::SHAPE);
        let phase: f32 = ports.read(Self::PHASE);
        let amplitude: f32 = ports.read(Self::AMPLITUDE);

        // Absolute beat position, never an accumulator — so a tempo change,
        // a reposition and a dropped tick all leave this correct.
        let beats = world.resource::<Time<Transport>>().beats();
        // An authored zero or negative period holds still rather than
        // dividing: the tick is infallible.
        let p = if period > 0.0 {
            (beats / period as f64 + phase as f64).rem_euclid(1.0) as f32
        } else {
            phase.rem_euclid(1.0)
        };
        ports.write(Self::OUT_VALUE, wave(shape, p) * amplitude);
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use bevy_ecs::entity::Entity;
    use bevy_time::{Fixed, Time, TimePlugin, TimeUpdateStrategy};
    use sway_graph::{
        CompiledGraph, GraphNode, GraphPlugin, NodeId, NodeType, NodeTypeRegistry, PortArena,
        Transport, TransportState, TransportTime, compile,
    };

    use super::*;
    use crate::Waveform;

    const TICK_HZ: f64 = 120.0;

    /// Registers the transport node types **without** `SignalNodesPlugin`.
    ///
    /// That plugin also installs `advance_transport`, which would freewheel
    /// the clock underneath every assertion below — at 120 BPM and a 120 Hz
    /// tick that is an extra 1/60 beat per `app.update()`, which turns every
    /// exact count and every phase comparison into an approximation. These
    /// tests are about what the nodes *read*; what advances the clock has its
    /// own suite in `transport.rs`.
    fn beat_app() -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .insert_resource(TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(GraphPlugin);
        sway_graph::register_node_type::<TransportTimeNode>(&mut app);
        sway_graph::register_node_type::<SyncLfo>(&mut app);
        app.update();
        app
    }

    fn node_type_id<N: NodeType>(app: &App) -> sway_graph::NodeTypeId {
        app.world()
            .resource::<NodeTypeRegistry>()
            .id_of(core::any::type_name::<N>())
            .expect("node type registered by beat_app")
    }

    fn compile_graph(app: &mut App) {
        let compiled = compile(app.world_mut()).expect("compiles");
        let slots_len = compiled.slots_len;
        app.world_mut().resource_mut::<PortArena>().resize(slots_len);
        app.world_mut().insert_resource(compiled);
    }

    /// Puts the transport at an exact beat position and lets it run.
    fn play_at(app: &mut App, bpm: f64) {
        let mut time = app.world_mut().resource_mut::<Time<Transport>>();
        time.transport_mut().state = TransportState::Playing;
        time.transport_mut().secs_per_beat = 60.0 / bpm;
        time.reposition(0.0);
    }

    fn out(app: &App, node: Entity, ordinal: u16) -> f32 {
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("compiled");
        let slot = plan.base + plan.field_offsets[ordinal as usize];
        *app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<f32>()
            .expect("outlet is f32")
    }

    fn spawn_transport_time(app: &mut App) -> Entity {
        let node_type = node_type_id::<TransportTimeNode>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(0), node_type },
                TransportTimeInlets::default(),
                TransportTimeState,
            ))
            .id()
    }

    #[test]
    fn a_node_with_no_inlets_compiles_and_ticks() {
        // TransportTimeNode is the first node type in this engine with an
        // empty Inlets struct — beats_per_bar belongs to the clock, not to a
        // node. Field derivation, prefill and the arena layout all have to
        // survive zero inlet fields.
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.update();

        assert!(out(&app, node, TransportTimeNode::OUT_BPM) > 0.0);
    }

    #[test]
    fn transport_time_reports_bar_beat_and_sixteenth_from_one() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);
        // Beat 17.5 is bar 5, beat 2, sixteenth 3 in 4/4.
        app.world_mut().resource_mut::<Time<Transport>>().reposition(17.5);

        app.update();

        assert_eq!(out(&app, node, TransportTimeNode::OUT_BAR), 5.0);
        assert_eq!(out(&app, node, TransportTimeNode::OUT_BEAT), 2.0);
        assert_eq!(out(&app, node, TransportTimeNode::OUT_SIXTEENTH), 3.0);
    }

    #[test]
    fn transport_time_reports_whether_the_transport_is_playing() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);

        app.update();
        assert_eq!(out(&app, node, TransportTimeNode::OUT_PLAYING), 0.0);

        play_at(&mut app, 120.0);
        app.update();
        assert_eq!(out(&app, node, TransportTimeNode::OUT_PLAYING), 1.0);
    }

    #[test]
    fn transport_time_bar_phase_sweeps_zero_to_one_across_a_bar() {
        let mut app = beat_app();
        let node = spawn_transport_time(&mut app);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.world_mut().resource_mut::<Time<Transport>>().reposition(2.0);
        app.update();
        let half = out(&app, node, TransportTimeNode::OUT_BAR_PHASE);
        assert!((half - 0.5).abs() < 0.02, "two beats into a 4/4 bar is {half}");
    }

    fn spawn_sync_lfo(app: &mut App, beats: f32, shape: Waveform) -> Entity {
        let node_type = node_type_id::<SyncLfo>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(1), node_type },
                SyncLfoInlets { beats, shape, phase: 0.0, amplitude: 1.0 },
                SyncLfoState,
            ))
            .id()
    }

    #[test]
    fn a_sync_lfo_completes_one_cycle_per_period_in_beats() {
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 4.0, Waveform::Saw);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        // A saw over four beats: 0 beats is -1, 2 beats is 0, just under 4
        // beats is nearly +1.
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.0);
        app.update();
        assert!((out(&app, node, SyncLfo::OUT_VALUE) + 1.0).abs() < 0.02);

        app.world_mut().resource_mut::<Time<Transport>>().reposition(2.0);
        app.update();
        assert!(out(&app, node, SyncLfo::OUT_VALUE).abs() < 0.02);
    }

    #[test]
    fn a_sync_lfo_holds_its_phase_when_the_tempo_changes() {
        // The point of tempo sync: at a given beat position the output is the
        // same regardless of how fast the beats went by.
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 2.0, Waveform::Sine);
        compile_graph(&mut app);

        play_at(&mut app, 120.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.5);
        app.update();
        let at_120 = out(&app, node, SyncLfo::OUT_VALUE);

        play_at(&mut app, 174.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.5);
        app.update();
        let at_174 = out(&app, node, SyncLfo::OUT_VALUE);

        assert!((at_120 - at_174).abs() < 1e-5, "{at_120} vs {at_174}");
    }

    #[test]
    fn a_sync_lfo_with_a_zero_or_negative_period_holds_still_rather_than_dividing_by_zero() {
        // The tick is infallible: an authored 0 must not produce NaN.
        let mut app = beat_app();
        let node = spawn_sync_lfo(&mut app, 0.0, Waveform::Sine);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        app.update();

        assert!(out(&app, node, SyncLfo::OUT_VALUE).is_finite());
    }
}
