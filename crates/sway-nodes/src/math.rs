//! Math, Remap, Switch, Select — pure continuous / latch nodes (spec §8).

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_graph::{
    ContinuousIdx, Event, EventIdx, NodeType, PortView, TickCtx, register_event_port,
};

use crate::NoteMsg;

// --- Math -----------------------------------------------------------------

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    #[default]
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

#[derive(Reflect, Component, Default)]
pub struct MathParams {
    pub op: MathOp,
    pub a: f32,
    pub b: f32,
}

#[derive(Reflect, Default)]
pub struct MathOutputs {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct MathState;

pub struct Math;

impl Math {
    pub const OP: u16 = 0;
    pub const A: u16 = 1;
    pub const B: u16 = 2;
    pub const OUT_VALUE: u16 = 3;
}

impl NodeType for Math {
    type Params = MathParams;
    type Outputs = MathOutputs;
    type State = MathState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("op", Self::OP),
        ("a", Self::A),
        ("b", Self::B),
        ("value", Self::OUT_VALUE),
    ];

    fn register(app: &mut App) {
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<MathOp>();
    }

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let op: MathOp = ports.read(ContinuousIdx(Self::OP as u32));
        let a: f32 = ports.read(ContinuousIdx(Self::A as u32));
        let b: f32 = ports.read(ContinuousIdx(Self::B as u32));
        let value = match op {
            MathOp::Add => a + b,
            MathOp::Sub => a - b,
            MathOp::Mul => a * b,
            MathOp::Div => {
                if b == 0.0 {
                    0.0
                } else {
                    a / b
                }
            }
            MathOp::Min => a.min(b),
            MathOp::Max => a.max(b),
        };
        ports.write(ContinuousIdx(Self::OUT_VALUE as u32), value);
    }
}

// --- Remap ----------------------------------------------------------------

#[derive(Reflect, Component, Default)]
pub struct RemapParams {
    pub value: f32,
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub clamp: bool,
}

#[derive(Reflect, Default)]
pub struct RemapOutputs {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct RemapState;

pub struct Remap;

impl Remap {
    pub const VALUE: u16 = 0;
    pub const IN_MIN: u16 = 1;
    pub const IN_MAX: u16 = 2;
    pub const OUT_MIN: u16 = 3;
    pub const OUT_MAX: u16 = 4;
    pub const CLAMP: u16 = 5;
    pub const OUT_VALUE: u16 = 6;
}

impl NodeType for Remap {
    type Params = RemapParams;
    type Outputs = RemapOutputs;
    type State = RemapState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("value", Self::VALUE),
        ("in_min", Self::IN_MIN),
        ("in_max", Self::IN_MAX),
        ("out_min", Self::OUT_MIN),
        ("out_max", Self::OUT_MAX),
        ("clamp", Self::CLAMP),
        ("value", Self::OUT_VALUE),
    ];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let mut value: f32 = ports.read(ContinuousIdx(Self::VALUE as u32));
        let in_min: f32 = ports.read(ContinuousIdx(Self::IN_MIN as u32));
        let in_max: f32 = ports.read(ContinuousIdx(Self::IN_MAX as u32));
        let out_min: f32 = ports.read(ContinuousIdx(Self::OUT_MIN as u32));
        let out_max: f32 = ports.read(ContinuousIdx(Self::OUT_MAX as u32));
        let clamp: bool = ports.read(ContinuousIdx(Self::CLAMP as u32));

        if clamp {
            let lo = in_min.min(in_max);
            let hi = in_min.max(in_max);
            value = value.clamp(lo, hi);
        }

        let out = if in_min == in_max {
            out_min
        } else {
            let t = (value - in_min) / (in_max - in_min);
            out_min + t * (out_max - out_min)
        };
        ports.write(ContinuousIdx(Self::OUT_VALUE as u32), out);
    }
}

// --- Switch ---------------------------------------------------------------

#[derive(Reflect, Component, Default)]
pub struct SwitchParams {
    pub select: bool,
    pub a: f32,
    pub b: f32,
}

#[derive(Reflect, Default)]
pub struct SwitchOutputs {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct SwitchState;

pub struct Switch;

impl Switch {
    pub const SELECT: u16 = 0;
    pub const A: u16 = 1;
    pub const B: u16 = 2;
    pub const OUT_VALUE: u16 = 3;
}

impl NodeType for Switch {
    type Params = SwitchParams;
    type Outputs = SwitchOutputs;
    type State = SwitchState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("select", Self::SELECT),
        ("a", Self::A),
        ("b", Self::B),
        ("value", Self::OUT_VALUE),
    ];

    fn register(_app: &mut App) {}

    fn tick(_world: &mut World, _node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let select: bool = ports.read(ContinuousIdx(Self::SELECT as u32));
        let a: f32 = ports.read(ContinuousIdx(Self::A as u32));
        let b: f32 = ports.read(ContinuousIdx(Self::B as u32));
        ports.write(
            ContinuousIdx(Self::OUT_VALUE as u32),
            if select { a } else { b },
        );
    }
}

// --- Select ---------------------------------------------------------------

#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteField {
    #[default]
    Note,
    Velocity,
}

#[derive(Reflect, Component, Default)]
pub struct SelectParams {
    pub trigger: Event<NoteMsg>,
    pub field: NoteField,
}

#[derive(Reflect, Default)]
pub struct SelectOutputs {
    pub value: f32,
}

#[derive(Component, Default)]
pub struct SelectState {
    pub held: f32,
}

pub struct Select;

impl Select {
    pub const FIELD: u16 = 0;
    pub const OUT_VALUE: u16 = 1;
    pub const TRIGGER: u16 = 0;
}

impl NodeType for Select {
    type Params = SelectParams;
    type Outputs = SelectOutputs;
    type State = SelectState;

    const PORT_ORDINALS: &'static [(&'static str, u16)] = &[
        ("field", Self::FIELD),
        ("value", Self::OUT_VALUE),
        ("trigger", Self::TRIGGER),
    ];

    fn register(app: &mut App) {
        register_event_port::<NoteMsg>(app);
        app.world_mut()
            .resource_mut::<bevy_ecs::reflect::AppTypeRegistry>()
            .write()
            .register::<NoteField>();
    }

    fn tick(world: &mut World, node: Entity, ports: &mut PortView, _ctx: &TickCtx) {
        let field: NoteField = ports.read(ContinuousIdx(Self::FIELD as u32));
        let last = ports
            .events::<NoteMsg>(EventIdx(Self::TRIGGER as u32))
            .last()
            .map(|ev| ev.value.clone());

        if let Some(msg) = last {
            let held = match field {
                NoteField::Note => msg.note as f32,
                NoteField::Velocity => msg.velocity as f32 / 127.0,
            };
            world
                .get_mut::<SelectState>(node)
                .expect("SelectState")
                .held = held;
        }

        let held = world.get::<SelectState>(node).expect("SelectState").held;
        ports.write(ContinuousIdx(Self::OUT_VALUE as u32), held);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;
    use sway_graph::{NodeType, Occurrence, PortArena, PortView, TickCtx};

    use super::*;

    fn context() -> TickCtx {
        TickCtx {
            dt: 1.0 / 120.0,
            tick_start: 0.0,
            tick_index: 0,
        }
    }

    fn math_value(op: MathOp, a: f32, b: f32) -> f32 {
        let mut world = World::new();
        let node = world.spawn(MathState).id();
        let mut arena = PortArena::new(4, 0);
        arena.continuous[Math::OP as usize] = Box::new(op);
        arena.continuous[Math::A as usize] = Box::new(a);
        arena.continuous[Math::B as usize] = Box::new(b);
        let mut ports = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 3]);
        Math::tick(&mut world, node, &mut ports, &context());
        *arena.continuous[Math::OUT_VALUE as usize]
            .try_downcast_ref::<f32>()
            .unwrap()
    }

    #[test]
    fn math_supports_every_operation_and_zero_division() {
        for (op, expected) in [
            (MathOp::Add, 8.0),
            (MathOp::Sub, 4.0),
            (MathOp::Mul, 12.0),
            (MathOp::Div, 3.0),
            (MathOp::Min, 2.0),
            (MathOp::Max, 6.0),
        ] {
            assert_eq!(math_value(op, 6.0, 2.0), expected);
        }
        assert_eq!(math_value(MathOp::Div, 6.0, 0.0), 0.0);
    }

    fn remap_value(value: f32, clamp: bool) -> f32 {
        let mut world = World::new();
        let node = world.spawn(RemapState).id();
        let mut arena = PortArena::new(7, 0);
        for (index, value) in [
            (Remap::VALUE, value),
            (Remap::IN_MIN, 0.0),
            (Remap::IN_MAX, 10.0),
            (Remap::OUT_MIN, -1.0),
            (Remap::OUT_MAX, 1.0),
        ] {
            arena.continuous[index as usize] = Box::new(value);
        }
        arena.continuous[Remap::CLAMP as usize] = Box::new(clamp);
        let mut ports = PortView::new(&mut arena, 0, 0, 7, 0, &[false; 6]);
        Remap::tick(&mut world, node, &mut ports, &context());
        *arena.continuous[Remap::OUT_VALUE as usize]
            .try_downcast_ref::<f32>()
            .unwrap()
    }

    #[test]
    fn remap_can_extrapolate_or_clamp() {
        assert_eq!(remap_value(15.0, false), 2.0);
        assert_eq!(remap_value(15.0, true), 1.0);
    }

    #[test]
    fn remap_degenerate_input_range_returns_out_min() {
        let mut world = World::new();
        let node = world.spawn(RemapState).id();
        let mut arena = PortArena::new(7, 0);
        for (index, value) in [
            (Remap::VALUE, 4.0_f32),
            (Remap::IN_MIN, 2.0_f32),
            (Remap::IN_MAX, 2.0_f32),
            (Remap::OUT_MIN, 7.0_f32),
            (Remap::OUT_MAX, 9.0_f32),
        ] {
            arena.continuous[index as usize] = Box::new(value);
        }
        arena.continuous[Remap::CLAMP as usize] = Box::new(false);
        let mut ports = PortView::new(&mut arena, 0, 0, 7, 0, &[false; 6]);
        Remap::tick(&mut world, node, &mut ports, &context());
        assert_eq!(
            arena.continuous[Remap::OUT_VALUE as usize].try_downcast_ref::<f32>(),
            Some(&7.0)
        );
    }

    fn switch_value(select: bool) -> f32 {
        let mut world = World::new();
        let node = world.spawn(SwitchState).id();
        let mut arena = PortArena::new(4, 0);
        arena.continuous[Switch::SELECT as usize] = Box::new(select);
        arena.continuous[Switch::A as usize] = Box::new(3.0_f32);
        arena.continuous[Switch::B as usize] = Box::new(9.0_f32);
        let mut ports = PortView::new(&mut arena, 0, 0, 4, 0, &[false; 3]);
        Switch::tick(&mut world, node, &mut ports, &context());
        *arena.continuous[Switch::OUT_VALUE as usize]
            .try_downcast_ref::<f32>()
            .unwrap()
    }

    #[test]
    fn switch_selects_a_when_true_and_b_when_false() {
        assert_eq!(switch_value(true), 3.0);
        assert_eq!(switch_value(false), 9.0);
    }

    #[test]
    fn select_latches_the_last_event_and_holds_without_events() {
        let mut world = World::new();
        let node = world.spawn(SelectState::default()).id();
        let mut arena = PortArena::new(2, 1);
        arena.continuous[Select::FIELD as usize] = Box::new(NoteField::Note);
        arena.events[Select::TRIGGER as usize] = vec![
            Occurrence {
                offset: 0.001,
                value: Box::new(NoteMsg {
                    note: 60,
                    velocity: 20,
                }),
            },
            Occurrence {
                offset: 0.006,
                value: Box::new(NoteMsg {
                    note: 72,
                    velocity: 100,
                }),
            },
        ];
        {
            let mut ports = PortView::new(&mut arena, 0, 0, 2, 1, &[false]);
            Select::tick(&mut world, node, &mut ports, &context());
        }
        assert_eq!(
            arena.continuous[Select::OUT_VALUE as usize].try_downcast_ref::<f32>(),
            Some(&72.0)
        );

        arena.events[Select::TRIGGER as usize].clear();
        let mut ports = PortView::new(&mut arena, 0, 0, 2, 1, &[false]);
        Select::tick(&mut world, node, &mut ports, &context());
        assert_eq!(
            arena.continuous[Select::OUT_VALUE as usize].try_downcast_ref::<f32>(),
            Some(&72.0)
        );
    }

    #[test]
    fn select_normalizes_velocity() {
        let mut world = World::new();
        let node = world.spawn(SelectState::default()).id();
        let mut arena = PortArena::new(2, 1);
        arena.continuous[Select::FIELD as usize] = Box::new(NoteField::Velocity);
        arena.events[Select::TRIGGER as usize] = vec![Occurrence {
            offset: 0.001,
            value: Box::new(NoteMsg {
                note: 60,
                velocity: 127,
            }),
        }];
        let mut ports = PortView::new(&mut arena, 0, 0, 2, 1, &[false]);
        Select::tick(&mut world, node, &mut ports, &context());
        assert_eq!(
            arena.continuous[Select::OUT_VALUE as usize].try_downcast_ref::<f32>(),
            Some(&1.0)
        );
    }
}
