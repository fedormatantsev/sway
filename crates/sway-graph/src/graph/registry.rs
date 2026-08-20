//! Node-kind registration.
//!
//! A node kind contributes three things to the type registry:
//!
//! - [`ReflectNodeKind`] — the reflected [`NodeKind`] trait, which the tick
//!   uses to call `evaluate` on a `&mut dyn Reflect`.
//! - `ReflectDefault` — how [`Graph::create`](crate::graph::Graph::create)
//!   builds a fresh value.
//!
//! The reflected type of each of the three parts is *not* registered: it is
//! already on the kind's own `TypeInfo`, and [`part_type`] reads it from there.

use core::any::TypeId;

use bevy_app::App;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{
    FromReflect, GetTypeRegistration, Reflect, TypeInfo, TypePath, TypeRegistry, Typed,
    reflect_trait,
};

use crate::graph::node::Part;

/// A node kind's evaluation.
///
/// `&mut self` reaches all three parts at once — inlets as they stand this
/// tick, state to read and write in place, outlets to write in place.
/// `&World` is read-only access to state outside the graph, which is how a
/// clock or a MIDI transport node reads its source without `sway-graph`
/// naming that source's type (design D4).
///
/// The `World` handed to `evaluate` **does not contain the `Graph`**: the tick
/// runs inside `World::resource_scope`, which takes the resource out for the
/// duration. A node therefore cannot read another node's outlets behind the
/// edge list's back, and cannot re-enter the tick.
///
/// *Node-authoring rule (design D4).* Golden traces are reproducible only by
/// discipline: world reads must stay confined to resources the trace controls.
/// Reading entity state or `Time<Real>` breaks reproducibility.
#[reflect_trait]
pub trait NodeKind {
    /// Runs this node for one tick.
    fn evaluate(&mut self, world: &World);
}

/// The reflected type of one of a node kind's three parts.
///
/// Read straight off the registration's own `TypeInfo` rather than from type
/// data of its own: `TypeInfo::Struct::field` already answers this, and one
/// registered item per node kind restating it is one more thing to keep in
/// step. `None` when the type is not registered, is not a struct, or has no
/// field by that name — which [`register_node_kind`] refuses to register.
///
/// ```ignore
/// let inlets = part_type(registry, type_id, Part::Inlets)?;
/// ```
pub fn part_type(
    registry: &TypeRegistry,
    type_id: TypeId,
    part: Part,
) -> Option<&'static TypeInfo> {
    let TypeInfo::Struct(struct_info) = registry.get(type_id)?.type_info() else {
        return None;
    };
    struct_info.field(part.as_str())?.type_info()
}

/// Whether a part is the empty `()` part.
pub fn is_empty_part(info: &TypeInfo) -> bool {
    info.type_id() == TypeId::of::<()>()
}

/// Registers a node kind with a bare [`TypeRegistry`].
///
/// Panics if `T` is not a struct with exactly the three part fields — a D3
/// violation is a programming error, not a runtime condition, and this is
/// where a caller expects to be told they got it wrong.
pub fn register_node_kind<T>(registry: &mut TypeRegistry)
where
    T: NodeKind + Reflect + Typed + TypePath + FromReflect + GetTypeRegistration + Default,
{
    assert_node_kind_shape::<T>();
    registry.register::<T>();
    registry.register_type_data::<T, ReflectNodeKind>();
    registry.register_type_data::<T, ReflectDefault>();
}

/// Design D3: a node kind is one struct with exactly the fields `inlets`,
/// `state` and `outlets`, an absent part being `()`.
fn assert_node_kind_shape<T: Typed + TypePath>() {
    let info = T::type_info();
    let TypeInfo::Struct(struct_info) = info else {
        panic!(
            "`{}` is not a node kind: design D3 requires a struct with exactly \
             the fields `inlets`, `state` and `outlets` (use `()` for an empty \
             part)",
            info.type_path()
        );
    };
    for part in Part::ALL {
        assert!(
            struct_info.field(part.as_str()).is_some(),
            "`{}` is not a node kind: design D3 requires a struct with exactly \
             the fields `inlets`, `state` and `outlets` (use `()` for an empty \
             part); `{part}` is missing",
            info.type_path()
        );
    }
}

/// `App`-side sugar for [`register_node_kind`].
pub trait RegisterNodeKind {
    /// Registers `T` as a node kind.
    fn register_node_kind<T>(&mut self) -> &mut Self
    where
        T: NodeKind + Reflect + Typed + TypePath + FromReflect + GetTypeRegistration + Default;
}

impl RegisterNodeKind for App {
    fn register_node_kind<T>(&mut self) -> &mut Self
    where
        T: NodeKind + Reflect + Typed + TypePath + FromReflect + GetTypeRegistration + Default,
    {
        let type_registry = self
            .world_mut()
            .get_resource_or_init::<AppTypeRegistry>()
            .clone();
        register_node_kind::<T>(&mut type_registry.write());
        self
    }
}

/// Every registered node kind's type path, sorted, for the editor's palette.
pub fn registered_node_kinds(registry: &TypeRegistry) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = registry
        .iter_with_data::<ReflectNodeKind>()
        .map(|(registration, _)| registration.type_info().type_path())
        .collect();
    kinds.sort_unstable();
    kinds
}

/// Looks a node kind up by type path, returning its `TypeId` only if it really
/// is a registered node kind.
pub fn node_kind_type_id(registry: &TypeRegistry, type_path: &str) -> Option<TypeId> {
    let registration = registry.get_with_type_path(type_path)?;
    registration.data::<ReflectNodeKind>()?;
    Some(registration.type_id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::testing::{Counter, Sink, test_registry};

    #[test]
    fn each_of_a_node_kinds_three_part_types_is_readable() {
        let registry = test_registry();
        let path = |part| {
            part_type(&registry, TypeId::of::<Counter>(), part)
                .expect("a registered node kind has all three parts")
                .type_path()
        };

        assert_eq!(path(Part::Inlets), "sway_graph::graph::testing::Step");
        assert_eq!(
            path(Part::State),
            "sway_graph::graph::testing::Accumulator"
        );
        assert_eq!(path(Part::Outlets), "sway_graph::graph::testing::Total");
        assert!(!is_empty_part(
            part_type(&registry, TypeId::of::<Counter>(), Part::Inlets).unwrap()
        ));
    }

    #[test]
    fn an_empty_part_reads_back_as_the_unit_type() {
        let registry = test_registry();
        let state = part_type(&registry, TypeId::of::<Sink>(), Part::State)
            .expect("an absent part is `()`, not a missing field");
        assert!(is_empty_part(state));
        assert_eq!(state.type_path(), "()");
    }

    #[test]
    fn an_unregistered_type_has_no_part_types() {
        #[derive(Reflect, Default)]
        struct NotRegistered {
            inlets: (),
            state: (),
            outlets: (),
        }
        let registry = test_registry();
        assert!(part_type(&registry, TypeId::of::<NotRegistered>(), Part::Inlets).is_none());
    }

    #[test]
    #[should_panic(expected = "is not a node kind")]
    fn registering_a_type_without_the_three_parts_is_refused_at_the_call_site() {
        #[derive(Reflect, Default)]
        struct Wrong {
            inlets: (),
            outlets: (),
        }
        impl NodeKind for Wrong {
            fn evaluate(&mut self, _world: &World) {}
        }
        let mut registry = TypeRegistry::new();
        register_node_kind::<Wrong>(&mut registry);
    }

    #[test]
    fn a_registered_kind_is_resolvable_by_type_path() {
        let registry = test_registry();
        assert_eq!(
            node_kind_type_id(&registry, "sway_graph::graph::testing::Counter"),
            Some(TypeId::of::<Counter>())
        );
        assert_eq!(node_kind_type_id(&registry, "not::A::Kind"), None);
        // `Step` is registered (it is a part type) but is not a node kind.
        assert_eq!(
            node_kind_type_id(&registry, "sway_graph::graph::testing::Step"),
            None
        );
    }

    #[test]
    fn the_palette_lists_every_registered_kind() {
        let kinds = registered_node_kinds(&test_registry());
        assert!(kinds.contains(&"sway_graph::graph::testing::Counter"));
        assert!(kinds.contains(&"sway_graph::graph::testing::Sink"));
        assert!(kinds.windows(2).all(|w| w[0] <= w[1]), "sorted");
    }
}
