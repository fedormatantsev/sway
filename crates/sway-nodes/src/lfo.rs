//! LFO — absolute-time periodic oscillator (spec §6, §8).

use core::f32::consts::TAU;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::{ContinuousIdx, NodeType, PortView, TickCtx};

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
}

#[derive(Reflect, Component, Default)]
pub struct LfoParams {
    pub hz: f32,
    pub shape: Waveform,
    pub phase: f32,
    pub amplitude: f32,
}

#[derive(Reflect, Default)]
pub struct LfoOutputs {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct LfoState;

pub struct LFO;

impl LFO {
    pub const HZ: u16 = 0;
    pub const SHAPE: u16 = 1;
    pub const PHASE: u16 = 2;
    pub const AMPLITUDE: u16 = 3;
    pub const OUT_VALUE: u16 = 4;
}

impl NodeType for LFO {
    type Params = LfoParams;
    type Outputs = LfoOutputs;
    type State = LfoState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("hz", Self::HZ),
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

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, ctx: &TickCtx) {
        let hz: f32 = ports.read(ContinuousIdx(Self::HZ as u32));
        let shape: Waveform = ports.read(ContinuousIdx(Self::SHAPE as u32));
        let phase: f32 = ports.read(ContinuousIdx(Self::PHASE as u32));
        let amplitude: f32 = ports.read(ContinuousIdx(Self::AMPLITUDE as u32));

        // Absolute time — never accumulate phase across ticks (spec §6).
        let p = (ctx.tick_start * hz as f64 + phase as f64).rem_euclid(1.0) as f32;

        let wave = match shape {
            Waveform::Sine => (p * TAU).sin(),
            Waveform::Triangle => 4.0 * (p - 0.5).abs() - 1.0,
            Waveform::Saw => 2.0 * p - 1.0,
            Waveform::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        };
        ports.write(ContinuousIdx(Self::OUT_VALUE as u32), wave * amplitude);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;
    use sway_graph::{NodeType, PortArena, PortView, TickCtx};

    use super::*;

    const DT: f32 = 1.0 / 120.0;

    fn run_lfo(ticks: usize, dropped: core::ops::Range<usize>) -> f32 {
        let mut world = World::new();
        let node = world.spawn(LfoState).id();
        let mut arena = PortArena::new(5, 0);
        arena.continuous[LFO::HZ as usize] = Box::new(2.25_f32);
        arena.continuous[LFO::SHAPE as usize] = Box::new(Waveform::Triangle);
        arena.continuous[LFO::PHASE as usize] = Box::new(0.17_f32);
        arena.continuous[LFO::AMPLITUDE as usize] = Box::new(0.8_f32);

        for tick in 0..ticks {
            if dropped.contains(&tick) {
                continue;
            }
            let mut ports = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 4]);
            LFO::tick(
                &mut world,
                node,
                &mut ports,
                &TickCtx {
                    dt: DT,
                    tick_start: tick as f64 * DT as f64,
                    tick_index: tick as u64,
                },
            );
        }

        *arena.continuous[LFO::OUT_VALUE as usize]
            .try_downcast_ref::<f32>()
            .expect("LFO value is f32")
    }

    fn run_lfo_continuously(ticks: usize) -> f32 {
        run_lfo(ticks, 0..0)
    }

    fn run_lfo_with_dropped_ticks(ticks: usize, gap: core::ops::Range<usize>) -> f32 {
        run_lfo(ticks, gap)
    }

    #[test]
    fn the_lfo_is_a_function_of_absolute_time_not_an_accumulator() {
        let a = run_lfo_continuously(100);
        let b = run_lfo_with_dropped_ticks(100, 45..55);
        assert!((a - b).abs() < 1e-6, "accumulated phase: {a} vs {b}");
    }

    #[test]
    fn waveforms_are_bipolar_and_amplitude_scaled() {
        let mut world = World::new();
        let node = world.spawn(LfoState).id();
        for (shape, expected) in [
            (Waveform::Sine, 0.5),
            (Waveform::Triangle, 0.0),
            (Waveform::Saw, -0.25),
            (Waveform::Square, 0.5),
        ] {
            let mut arena = PortArena::new(5, 0);
            arena.continuous[LFO::HZ as usize] = Box::new(0.0_f32);
            arena.continuous[LFO::SHAPE as usize] = Box::new(shape);
            arena.continuous[LFO::PHASE as usize] = Box::new(0.25_f32);
            arena.continuous[LFO::AMPLITUDE as usize] = Box::new(0.5_f32);
            let mut ports = PortView::new(&mut arena, 0, 0, 5, 0, &[false; 4]);
            LFO::tick(
                &mut world,
                node,
                &mut ports,
                &TickCtx {
                    dt: DT,
                    tick_start: 0.0,
                    tick_index: 0,
                },
            );
            let actual = arena.continuous[LFO::OUT_VALUE as usize]
                .try_downcast_ref::<f32>()
                .copied()
                .unwrap();
            assert!((actual - expected).abs() < 1e-6, "{shape:?}: {actual}");
        }
    }
}
