//! Node-kind registration.
//!
//! A node kind contributes three things to the type registry:
//!
//! - [`ReflectNodeKind`] — the reflected [`NodeKind`] trait, which the tick
//!   uses to call `evaluate` on a `&mut dyn Reflect`.
//! - [`NodeParts`] — the reflected type of each of the three parts, so the
//!   editor, the document and the legality rule can walk a kind's schema
//!   without an instance in hand.
//! - `ReflectDefault` — how [`GraphCommand::Create`](crate::graph::GraphCommand)
//!   builds a fresh value.

use core::any::TypeId;

use bevy_app::App;
use bevy_ecs::reflect::AppTypeRegistry;
use bevy_ecs::world::World;
use bevy_reflect::std_traits::ReflectDefault;
use bevy_reflect::{
    FromReflect, FromType, GetTypeRegistration, Reflect, TypeInfo, TypePath, TypeRegistry, Typed,
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

/// The reflected type of one part of a node kind.
#[derive(Clone, Copy, Debug)]
pub struct PartType {
    /// The part's concrete `TypeId`. `TypeId::of::<()>()` for an empty part.
    pub type_id: TypeId,
    /// The part's reflected type path. `"()"` for an empty part.
    pub type_path: &'static str,
    /// The part's static `TypeInfo`, when it has one.
    pub info: Option<&'static TypeInfo>,
}

impl PartType {
    /// Whether this part is the empty `()` part.
    pub fn is_empty(&self) -> bool {
        self.type_id == TypeId::of::<()>()
    }
}

/// The reflected type of each of a node kind's three parts.
///
/// Registered as type data, so it is resolvable from the type registry:
///
/// ```ignore
/// let parts = registry.get_type_data::<NodeParts>(type_id)?;
/// let inlets = parts.part(Part::Inlets);
/// ```
#[derive(Clone, Debug)]
pub struct NodeParts {
    inlets: PartType,
    state: PartType,
    outlets: PartType,
}

impl NodeParts {
    /// The reflected type of one part.
    pub fn part(&self, part: Part) -> PartType {
        match part {
            Part::Inlets => self.inlets,
            Part::State => self.state,
            Part::Outlets => self.outlets,
        }
    }

    /// The reflected type of the `inlets` part.
    pub fn inlets(&self) -> PartType {
        self.inlets
    }

    /// The reflected type of the `state` part.
    pub fn state(&self) -> PartType {
        self.state
    }

    /// The reflected type of the `outlets` part.
    pub fn outlets(&self) -> PartType {
        self.outlets
    }
}

fn part_type_of(info: &'static TypeInfo, name: &str) -> Option<PartType> {
    let TypeInfo::Struct(struct_info) = info else {
        return None;
    };
    let field = struct_info.field(name)?;
    Some(PartType {
        type_id: field.type_id(),
        type_path: field.type_path(),
        info: field.type_info(),
    })
}

impl<T: Typed> FromType<T> for NodeParts {
    fn from_type() -> Self {
        let info = T::type_info();
        let part = |name: &str| {
            part_type_of(info, name).unwrap_or_else(|| {
                panic!(
                    "`{}` is not a node kind: design D3 requires a struct with \
                     exactly the fields `inlets`, `state` and `outlets` (use \
                     `()` for an empty part); `{name}` is missing",
                    info.type_path()
                )
            })
        };
        Self {
            inlets: part("inlets"),
            state: part("state"),
            outlets: part("outlets"),
        }
    }
}

/// Registers a node kind with a bare [`TypeRegistry`].
///
/// Panics if `T` is not a struct with exactly the three part fields — a D3
/// violation is a programming error, not a runtime condition.
pub fn register_node_kind<T>(registry: &mut TypeRegistry)
where
    T: NodeKind + Reflect + Typed + TypePath + FromReflect + GetTypeRegistration + Default,
{
    registry.register::<T>();
    registry.register_type_data::<T, ReflectNodeKind>();
    registry.register_type_data::<T, ReflectDefault>();
    registry.register_type_data::<T, NodeParts>();
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
    fn a_node_kind_registers_its_three_part_types() {
        let registry = test_registry();
        let parts = registry
            .get_type_data::<NodeParts>(TypeId::of::<Counter>())
            .expect("NodeParts is registered type data");

        assert_eq!(parts.inlets().type_path, "sway_graph::graph::testing::Step");
        assert_eq!(
            parts.state().type_path,
            "sway_graph::graph::testing::Accumulator"
        );
        assert_eq!(
            parts.outlets().type_path,
            "sway_graph::graph::testing::Total"
        );
        assert!(!parts.inlets().is_empty());
    }

    #[test]
    fn an_empty_part_registers_as_the_unit_type() {
        let registry = test_registry();
        let parts = registry
            .get_type_data::<NodeParts>(TypeId::of::<Sink>())
            .expect("NodeParts is registered type data");
        assert!(parts.state().is_empty());
        assert_eq!(parts.state().type_path, "()");
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
