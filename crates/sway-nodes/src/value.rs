//! Value nodes: literals and arithmetic that produce an outlet.

use bevy::prelude::*;
use bevy_ecs::change_detection::Mut;
use bevy_reflect::Reflect;
use sway_graph::{Behaviour, EditorPos, ReflectBehaviour, ReflectWire, TickCtx};

use crate::field_wire;
use crate::outputs::{FloatOut, Vec3Out, write_outlet};

/// A vector literal whose components are driveable (roadmap D5). Transform,
/// colour and tint inlets take `Vec3`, so something has to produce one; this
/// reads as a value in the graph rather than as a `Compose` operator, which is
/// how both TouchDesigner and Houdini present it.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq, Behaviour)]
#[require(Vec3Out, EditorPos)]
pub struct Vec3Value {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Behaviour for Vec3Value {
    fn state_type(&self) -> Option<std::any::TypeId> {
        None
    }

    fn outlet_type(&self) -> Option<std::any::TypeId> {
        Some(std::any::TypeId::of::<Vec3Out>())
    }

    fn evaluate(
        &self,
        _state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        _ctx: &TickCtx,
    ) {
        write_outlet(outlets, Vec3Out(Vec3::new(self.x, self.y, self.z)));
    }
}

field_wire!(
    /// Drives `Vec3.x`.
    Vec3XFrom / DrivesVec3X,
    FloatOut => Vec3Value,
    "x"
);

field_wire!(
    /// Drives `Vec3.y`.
    Vec3YFrom / DrivesVec3Y,
    FloatOut => Vec3Value,
    "y"
);

field_wire!(
    /// Drives `Vec3.z`.
    Vec3ZFrom / DrivesVec3Z,
    FloatOut => Vec3Value,
    "z"
);

use crate::math::{MathOp, math_value, remap_value};

/// Binary arithmetic. `b` is an authored field a wire may override, which is
/// why there is no `Const` node: "LFO x 2" is one `Math` with `b: 2.0` unwired.
#[derive(Component, Reflect, Default, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq, Behaviour)]
#[require(FloatOut, EditorPos)]
pub struct Math {
    pub op: MathOp,
    pub a: f32,
    pub b: f32,
}

impl Behaviour for Math {
    fn state_type(&self) -> Option<std::any::TypeId> {
        None
    }

    fn outlet_type(&self) -> Option<std::any::TypeId> {
        Some(std::any::TypeId::of::<FloatOut>())
    }

    fn evaluate(
        &self,
        _state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        _ctx: &TickCtx,
    ) {
        write_outlet(outlets, FloatOut(math_value(self.op, self.a, self.b)));
    }
}

field_wire!(
    /// Drives `Math.a`.
    MathAFrom / DrivesMathA,
    FloatOut => Math,
    "a"
);

field_wire!(
    /// Drives `Math.b`.
    MathBFrom / DrivesMathB,
    FloatOut => Math,
    "b"
);

/// Rescales `input` from one range to another. `input` is a field rather than
/// an implicit inlet so that `RemapInputFrom` has something to write, exactly
/// as `Math.a` does.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Component, Default, PartialEq, Behaviour)]
#[require(FloatOut, EditorPos)]
pub struct Remap {
    pub input: f32,
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub clamp: bool,
}

impl Default for Remap {
    fn default() -> Self {
        Self {
            input: 0.0,
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            clamp: false,
        }
    }
}

impl Behaviour for Remap {
    fn state_type(&self) -> Option<std::any::TypeId> {
        None
    }

    fn outlet_type(&self) -> Option<std::any::TypeId> {
        Some(std::any::TypeId::of::<FloatOut>())
    }

    fn evaluate(
        &self,
        _state: Option<Mut<dyn Reflect>>,
        outlets: Option<Mut<dyn Reflect>>,
        _ctx: &TickCtx,
    ) {
        write_outlet(
            outlets,
            FloatOut(remap_value(
                self.input,
                self.in_min,
                self.in_max,
                self.out_min,
                self.out_max,
                self.clamp,
            )),
        );
    }
}

field_wire!(
    /// Drives `Remap.input`.
    RemapInputFrom / DrivesRemapInput,
    FloatOut => Remap,
    "input"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::MathOp;
    use crate::outputs::{FloatOut, Vec3Out};
    use crate::wire_testing::assert_writes_only_on_change;
    use sway_graph::WiresPlugin;

    fn slice_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(Time::<Fixed>::from_hz(120.0))
            .insert_resource(bevy::time::TimeUpdateStrategy::FixedTimesteps(1))
            .add_plugins(WiresPlugin)
            .add_plugins(crate::WireNodesPlugin);
        app.update(); // frame 0 starts with an empty fixed-time accumulator
        app
    }

    #[test]
    fn a_vec3_node_publishes_its_three_fields() {
        let mut app = slice_app();
        let node = app
            .world_mut()
            .spawn(Vec3Value {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            })
            .id();

        app.update();

        assert_eq!(
            app.world().get::<Vec3Out>(node).map(|o| o.0),
            Some(Vec3::new(1.0, 2.0, 3.0)),
            "#[require] must have supplied Vec3Out, and the behaviour filled it"
        );
    }

    #[test]
    fn a_float_reaches_a_vec3_axis_in_one_tick() {
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(0.75)).id();
        let node = app.world_mut().spawn(Vec3Value::default()).id();
        app.world_mut().entity_mut(node).insert(Vec3YFrom(float));

        app.update();

        assert_eq!(
            app.world().get::<Vec3Out>(node).map(|o| o.0),
            Some(Vec3::new(0.0, 0.75, 0.0)),
            "the inlet must land before the behaviour runs, in ONE tick"
        );
    }

    #[test]
    fn the_vec3_inlets_never_write_an_equal_value() {
        assert_writes_only_on_change::<Vec3XFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
        assert_writes_only_on_change::<Vec3YFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
        assert_writes_only_on_change::<Vec3ZFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Vec3Value::default(),
        );
    }

    #[test]
    fn math_computes_from_its_authored_and_driven_inlets() {
        // "LFO x 2" is one Math with b left unwired — the reason there is no
        // Const node.
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(3.0)).id();
        let node = app
            .world_mut()
            .spawn(Math {
                op: MathOp::Mul,
                a: 0.0,
                b: 2.0,
            })
            .id();
        app.world_mut().entity_mut(node).insert(MathAFrom(float));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(node).map(|o| o.0), Some(6.0));
    }

    #[test]
    fn remap_rescales_its_driven_input() {
        let mut app = slice_app();
        let float = app.world_mut().spawn(FloatOut(0.5)).id();
        let node = app
            .world_mut()
            .spawn(Remap {
                input: 0.0,
                in_min: 0.0,
                in_max: 1.0,
                out_min: 0.0,
                out_max: 10.0,
                clamp: true,
            })
            .id();
        app.world_mut()
            .entity_mut(node)
            .insert(RemapInputFrom(float));

        app.update();

        assert_eq!(app.world().get::<FloatOut>(node).map(|o| o.0), Some(5.0));
    }

    #[test]
    fn the_math_and_remap_inlets_never_write_an_equal_value() {
        assert_writes_only_on_change::<MathAFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Math::default(),
        );
        assert_writes_only_on_change::<MathBFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Math::default(),
        );
        assert_writes_only_on_change::<RemapInputFrom, _, _>(
            FloatOut(1.0),
            FloatOut(2.0),
            Remap::default(),
        );
    }
}
