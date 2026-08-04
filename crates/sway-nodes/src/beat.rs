//! Transport-aware nodes (parent §5, M3): a beat time base, a tempo-synced
//! oscillator, and a beat-quantised trigger.
//!
//! `TransportTime` and `SyncLfo` are pure functions of the beat position
//! `advance_transport` maintains. `BeatTrigger` also remembers its preceding
//! endpoint so adjacent event windows abut exactly; it discards that endpoint
//! when a transport reposition changes the origin. Thus a dropped tick, tempo
//! change, or reposition still produces output from the correct beat window.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use bevy_time::Time;
use sway_graph::{
    Events, MusicalTime, NodeType, PortView, TickCtx, Transport, TransportTime, register_events,
};

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

/// How often a [`BeatTrigger`] fires.
///
/// An enum-valued behaviour param, in the same family as `LFO.shape` and
/// `Math.op` — not a type selector. It changes a number, not which node this
/// is (parent §2.4).
///
/// `Beat` is the first variant and carries `#[default]`: firing once per beat
/// is what an author expects from a node called `BeatTrigger`, and the
/// workspace-wide rule that a default is the first variant listed needs no
/// exception here.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    #[default]
    Beat,
    Bar,
    Eighth,
    Sixteenth,
}

impl Division {
    /// This division's length, in beats.
    pub fn beats(self, beats_per_bar: u32) -> f64 {
        match self {
            Self::Bar => beats_per_bar.max(1) as f64,
            Self::Beat => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
        }
    }
}

/// What a [`BeatTrigger`] emits: the musical position of the boundary it
/// fired on.
#[derive(Reflect, Default, Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub bar: u32,
    pub beat: u32,
    pub sixteenth: u32,
}

/// Ceiling on occurrences per tick. A tick that somehow spans a thousand
/// beats — a stalled app resuming, a reposition far ahead — must not flood
/// every downstream event list; the tick is infallible and this is what makes
/// it so here.
pub const MAX_PULSES_PER_TICK: usize = 64;

#[derive(Reflect, Component, Default)]
pub struct BeatTriggerInlets {
    pub division: Division,
}

#[derive(Reflect, Default)]
pub struct BeatTriggerOutlets {
    pub pulse: Events<Beat>,
}

#[derive(Component, Default)]
pub struct BeatTriggerState {
    /// This node's own previous tick's `end` (in beats), reused as the start
    /// of the next boundary search.
    ///
    /// `end - advanced` looks like it should reconstruct the same value, but
    /// `end` and `advanced` come from two independent `Duration -> f64`
    /// conversions inside `Time<Transport>` (`elapsed_secs_f64()` and
    /// `delta_secs_f64()`), which round separately and are not guaranteed to
    /// agree to the bit. A boundary sitting near a tick seam can land on the
    /// wrong side of that reconstructed `start` and get double-counted (or
    /// skipped). Carrying the previous tick's own `end` forward instead keeps
    /// consecutive windows exactly abutting: this tick's `start` is the exact
    /// same value last tick reported as `end`, no recomputation involved.
    prev_end: Option<f64>,
    /// Transport origin paired with `prev_end`. Reposition changes this after
    /// an advance, making the prior endpoint a stale search start.
    prev_origin: Option<f64>,
}

pub struct BeatTrigger;

impl BeatTrigger {
    pub const DIVISION: u16 = 0;
    pub const OUT_PULSE: u16 = 1;
}

impl NodeType for BeatTrigger {
    type Inlets = BeatTriggerInlets;
    type Outlets = BeatTriggerOutlets;
    type State = BeatTriggerState;

    const ORDINALS: &'static [(&'static str, u16)] =
        &[("division", Self::DIVISION), ("pulse", Self::OUT_PULSE)];

    fn register(app: &mut App) {
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<Division>();
        register_events::<Beat>(app);
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, ctx: &TickCtx) {
        let division: Division = ports.read(Self::DIVISION);

        let (playing, beats_per_bar, end, advanced, origin) = {
            let time = world.resource::<Time<Transport>>();
            (
                time.is_playing(),
                time.transport().beats_per_bar,
                time.beats(),
                time.delta_secs_f64(),
                time.transport().origin_beats,
            )
        };

        let mut state = world.get_mut::<BeatTriggerState>(node).expect("BeatTriggerState");
        if !playing || advanced <= 0.0 {
            // Nothing advanced this tick, so there is no window to continue.
            // The next tick that does advance has no continuous predecessor
            // to resume from, so it falls back to reconstructing its own
            // start (first tick after Play).
            state.prev_end = None;
            state.prev_origin = None;
            return;
        }

        // Reuse the previous tick's own `end` as this tick's `start` when
        // the transport origin is unchanged. Start and Song Position apply
        // after advancing the clock, so their new origin makes that endpoint
        // stale; reconstruct this tick's small window instead.
        let start = match (state.prev_end, state.prev_origin) {
            (Some(previous_end), Some(previous_origin)) if previous_origin == origin => {
                previous_end
            }
            _ => (end - advanced).max(0.0),
        };
        state.prev_end = Some(end);
        state.prev_origin = Some(origin);

        let step = division.beats(beats_per_bar);

        // Every multiple of `step` in `(start, end]`. Half-open at the start,
        // so a boundary is never emitted twice across two ticks.
        let first = (start / step).floor() as i64 + 1;
        let last = (end / step).floor() as i64;
        for index in first..=last.min(first + MAX_PULSES_PER_TICK as i64 - 1) {
            let boundary = index as f64 * step;
            // Invert this tick's own advance to place the crossing inside
            // the window. Linear within a tick, which is exact for a steady
            // tempo and within a tick's worth of error otherwise.
            let offset = (ctx.dt as f64 * (boundary - start) / advanced)
                .clamp(0.0, ctx.dt as f64) as f32;
            let at = MusicalTime::from_beats(boundary, beats_per_bar);
            ports.emit(
                Self::OUT_PULSE,
                offset,
                Beat { bar: at.bar, beat: at.beat, sixteenth: at.sixteenth },
            );
        }
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
        sway_graph::register_node_type::<BeatTrigger>(&mut app);
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

    fn spawn_beat_trigger(app: &mut App, division: Division) -> Entity {
        let node_type = node_type_id::<BeatTrigger>(app);
        app.world_mut()
            .spawn((
                GraphNode { id: NodeId(2), node_type },
                BeatTriggerInlets { division },
                BeatTriggerState::default(),
            ))
            .id()
    }

    fn pulses(app: &App, node: Entity) -> Vec<sway_graph::Occurrence<Beat>> {
        let compiled = app.world().resource::<CompiledGraph>();
        let plan = compiled.plans.iter().find(|p| p.entity == node).expect("compiled");
        let slot = plan.base + plan.field_offsets[BeatTrigger::OUT_PULSE as usize];
        app.world().resource::<PortArena>().values[slot]
            .try_downcast_ref::<sway_graph::Events<Beat>>()
            .expect("pulse is Events<Beat>")
            .occurrences
            .clone()
    }

    /// Runs `ticks` ticks with the transport advancing `beats_per_tick`, and
    /// returns every occurrence seen, tick by tick.
    fn collect(app: &mut App, node: Entity, ticks: usize, beats_per_tick: f64) -> Vec<Beat> {
        let mut seen = Vec::new();
        for _ in 0..ticks {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(beats_per_tick));
            }
            app.update();
            seen.extend(pulses(app, node).into_iter().map(|o| o.value));
        }
        seen
    }

    #[test]
    fn a_beat_division_fires_once_per_beat() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        // Four beats' worth, at a tenth of a beat per tick.
        let fired = collect(&mut app, node, 40, 0.1);

        assert_eq!(fired.len(), 4, "four beats, four pulses: {fired:?}");
    }

    #[test]
    fn a_bar_division_fires_once_per_bar() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Bar);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 80, 0.1); // eight beats = two bars
        assert_eq!(fired.len(), 2);
        assert_eq!(fired[1].bar, 3, "the second pulse opens bar 3");
    }

    #[test]
    fn a_sixteenth_division_fires_four_times_per_beat() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 20, 0.1); // two beats
        assert_eq!(fired.len(), 8);
    }

    #[test]
    fn a_pulse_carries_the_musical_position_of_its_boundary() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let fired = collect(&mut app, node, 40, 0.1);
        assert_eq!(
            (fired[0].bar, fired[0].beat, fired[0].sixteenth),
            (1, 2, 1),
            "the first crossing after position 0 is beat 2"
        );
    }

    #[test]
    fn a_pulse_offset_lands_inside_the_tick_window() {
        // Sub-tick timestamps are the whole point of an event port: an
        // envelope downstream starts at the correct phase (parent §2.4).
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        let dt = (1.0 / TICK_HZ) as f32;
        for _ in 0..40 {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(0.1));
            }
            app.update();
            for occurrence in pulses(&app, node) {
                assert!(
                    (0.0..=dt).contains(&occurrence.offset),
                    "offset {} outside [0, {dt}]",
                    occurrence.offset
                );
            }
        }
    }

    #[test]
    fn a_boundary_landing_mid_tick_is_not_placed_at_zero() {
        // Half a beat per tick starting from 0.25 beats puts every boundary
        // squarely inside a window; an implementation that stamped 0.0 would
        // pass every count-based test above and still be wrong.
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);
        app.world_mut().resource_mut::<Time<Transport>>().reposition(0.25);

        let mut offsets = Vec::new();
        for _ in 0..8 {
            {
                let mut time = app.world_mut().resource_mut::<Time<Transport>>();
                time.advance_by(core::time::Duration::from_secs_f64(0.5));
            }
            app.update();
            offsets.extend(pulses(&app, node).into_iter().map(|o| o.offset));
        }

        assert!(!offsets.is_empty());
        assert!(
            offsets.iter().any(|&o| o > 1e-6),
            "every offset was zero — the boundary was not located inside the window"
        );
    }

    #[test]
    fn song_position_repositions_while_playing_only_search_the_current_tick() {
        // `advance_transport` advances first and applies Start/Song Position
        // afterwards. The jump must discard the previous search endpoint:
        // these one-tenth-beat windows cross no integer boundary, so neither
        // direction may synthesize historical Beat pulses.
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Beat);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.advance_by(core::time::Duration::from_secs_f64(0.1));
        }
        app.update();
        assert!(pulses(&app, node).is_empty());

        // Forward Song Position: the actual tick window is (100.0, 100.1].
        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.advance_by(core::time::Duration::from_secs_f64(0.1));
            time.reposition(100.1);
        }
        app.update();
        assert!(
            pulses(&app, node).is_empty(),
            "a forward reposition must not replay historical boundaries"
        );

        // Backward Song Position: the actual tick window is (0.9, 1.0], so
        // it contains precisely the beat-one boundary.
        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.advance_by(core::time::Duration::from_secs_f64(0.1));
            time.reposition(1.0);
        }
        app.update();
        let backward = pulses(&app, node);
        assert_eq!(backward.len(), 1, "only the current window may be searched");
        assert_eq!(
            (backward[0].value.bar, backward[0].value.beat, backward[0].value.sixteenth),
            (1, 2, 1),
            "the backward reposition must emit the beat-one boundary"
        );
    }

    #[test]
    fn a_stopped_transport_fires_nothing() {
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        // No play_at: the transport is stopped and never advances.

        for _ in 0..40 {
            app.update();
            assert!(pulses(&app, node).is_empty());
        }
    }

    #[test]
    fn a_long_freeze_does_not_flood_a_single_tick() {
        // A stalled app resuming, or a reposition far ahead, must not emit
        // thousands of occurrences in one tick.
        let mut app = beat_app();
        let node = spawn_beat_trigger(&mut app, Division::Sixteenth);
        compile_graph(&mut app);
        play_at(&mut app, 120.0);

        {
            let mut time = app.world_mut().resource_mut::<Time<Transport>>();
            time.advance_by(core::time::Duration::from_secs_f64(1000.0));
        }
        app.update();

        assert!(
            pulses(&app, node).len() <= MAX_PULSES_PER_TICK,
            "a thousand beats in one tick produced {} occurrences",
            pulses(&app, node).len()
        );
    }
}
