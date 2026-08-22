//! `Timer`: elapsed time in the `time` inlet's units, reset by Trigger.

use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use sway_events::{EventArena, EventHandle};
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::Trigger;

/// [`Timer`]'s inlets. `trigger` is variadic so several sources merge.
#[derive(Reflect, Default, Debug, Clone)]
pub struct TimerIn {
    pub time: f32,
    pub trigger: Vec<EventHandle<Trigger>>,
}

/// Origin latched against the time inlet (design D4).
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct TimerState {
    pub origin: f32,
    pub primed: bool,
}

/// [`Timer`]'s outlets.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub struct TimerOut {
    pub out: f32,
}

/// Accumulates elapsed time and resets to zero on any Trigger occurrence.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind)]
pub struct Timer {
    pub inlets: TimerIn,
    pub state: TimerState,
    pub outlets: TimerOut,
}

fn any_trigger(world: &World, handles: &[EventHandle<Trigger>]) -> bool {
    let Some(arena) = world.get_non_send::<EventArena>() else {
        return false;
    };
    handles
        .iter()
        .any(|handle| arena.read(*handle).is_some_and(|batch| !batch.is_empty()))
}

impl NodeKind for Timer {
    fn evaluate(&mut self, world: &World) {
        let time = self.inlets.time;
        let fired = any_trigger(world, &self.inlets.trigger);
        if !self.state.primed || fired {
            self.state.origin = time;
            self.state.primed = true;
        }
        self.outlets.out = (time - self.state.origin).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use bevy_ecs::world::World;
    use bevy_reflect::TypeRegistry;
    use sway_events::{EventArena, EventHandle, register_event_handle};
    use sway_graph::graph::registry::register_node_kind;
    use sway_graph::graph::testing::{self, read_field, set_field, tick_once};
    use sway_graph::graph::{Graph, Node, NodeKind, Part, Port, ReflectNodeKind};

    use super::*;
    use crate::Trigger;

    /// Test-only producer: publishes `count` Triggers onto an ordinary handle
    /// outlet so Timer's variadic inlet is filled by propagate, not set_field
    /// (a `Vec` inlet is truncated to the edge count each tick).
    #[derive(Reflect, Default, Debug)]
    struct ClickIn {
        pub count: f32,
    }

    #[derive(Reflect, Default, Debug)]
    struct ClickOut {
        pub out: EventHandle<Trigger>,
    }

    #[derive(Reflect, Default, Debug)]
    #[reflect(NodeKind)]
    struct Click {
        pub inlets: ClickIn,
        pub state: (),
        pub outlets: ClickOut,
    }

    impl NodeKind for Click {
        fn evaluate(&mut self, world: &World) {
            let n = self.inlets.count.max(0.0) as u32;
            self.outlets.out = match world.get_non_send::<EventArena>() {
                Some(arena) => arena.publish((0..n).map(|_| Trigger)),
                None => EventHandle::EMPTY,
            };
        }
    }

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::new();
        register_node_kind::<Timer>(&mut registry);
        register_node_kind::<Click>(&mut registry);
        register_event_handle::<Trigger>(&mut registry);
        registry.register::<Trigger>();
        registry
    }

    fn world_with_arena() -> World {
        let mut world = testing::trace_world(registry());
        world.insert_non_send(EventArena::default());
        world
    }

    #[test]
    fn time_since_start_with_no_trigger_equals_the_inlet() {
        let world = world_with_arena();
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Timer::default()));

        set_field(&mut graph, node, "time", &0.0_f32);
        tick_once(&mut graph, &world);
        set_field(&mut graph, node, "time", &4.0_f32);
        tick_once(&mut graph, &world);

        assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "out"), 4.0);
    }

    #[test]
    fn a_trigger_zeros_elapsed_time_and_further_time_advances() {
        let world = world_with_arena();
        let mut graph = Graph::default();
        let click = graph.insert(Node::of(Click::default()));
        let node = graph.insert(Node::of(Timer::default()));
        graph
            .connect(Port::new(click, "out"), Port::new(node, "trigger"), 0)
            .expect("legal");

        set_field(&mut graph, node, "time", &0.0_f32);
        tick_once(&mut graph, &world);
        set_field(&mut graph, node, "time", &2.0_f32);
        tick_once(&mut graph, &world);
        assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "out"), 2.0);

        set_field(&mut graph, click, "count", &1.0_f32);
        tick_once(&mut graph, &world);
        assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "out"), 0.0);

        set_field(&mut graph, click, "count", &0.0_f32);
        set_field(&mut graph, node, "time", &5.0_f32);
        tick_once(&mut graph, &world);
        assert_eq!(
            read_field::<f32>(&graph, node, Part::Outlets, "out"),
            3.0,
            "elapsed advances from the time of the reset"
        );
    }

    #[test]
    fn either_of_two_trigger_handles_resets() {
        let world = world_with_arena();
        let mut graph = Graph::default();
        let silent = graph.insert(Node::of(Click::default()));
        let loud = graph.insert(Node::of(Click {
            inlets: ClickIn { count: 1.0 },
            ..Default::default()
        }));
        let node = graph.insert(Node::of(Timer::default()));
        graph
            .connect(Port::new(silent, "out"), Port::new(node, "trigger"), 0)
            .expect("legal");
        graph
            .connect(Port::new(loud, "out"), Port::new(node, "trigger"), 1)
            .expect("legal");

        set_field(&mut graph, node, "time", &0.0_f32);
        tick_once(&mut graph, &world);
        set_field(&mut graph, node, "time", &3.0_f32);
        // loud still publishes this tick, so the timer resets rather than
        // accumulating to 3.
        tick_once(&mut graph, &world);

        assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "out"), 0.0);
    }

    #[test]
    fn no_arena_still_evaluates_and_accumulates() {
        let world = testing::trace_world(registry());
        let mut graph = Graph::default();
        let node = graph.insert(Node::of(Timer::default()));

        set_field(&mut graph, node, "time", &0.0_f32);
        tick_once(&mut graph, &world);
        set_field(&mut graph, node, "time", &4.0_f32);
        tick_once(&mut graph, &world);

        assert_eq!(read_field::<f32>(&graph, node, Part::Outlets, "out"), 4.0);
    }

    #[test]
    fn identical_inlets_and_state_give_identical_outlets() {
        let world = world_with_arena();
        let mut first = Graph::default();
        let mut second = Graph::default();
        let a = first.insert(Node::of(Timer {
            inlets: TimerIn {
                time: 1.5,
                trigger: Vec::new(),
            },
            ..Default::default()
        }));
        let b = second.insert(Node::of(Timer {
            inlets: TimerIn {
                time: 1.5,
                trigger: Vec::new(),
            },
            ..Default::default()
        }));

        tick_once(&mut first, &world);
        tick_once(&mut second, &world);

        assert_eq!(
            read_field::<f32>(&first, a, Part::Outlets, "out"),
            read_field::<f32>(&second, b, Part::Outlets, "out"),
        );
    }
}
