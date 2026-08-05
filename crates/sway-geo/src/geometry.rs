//! `Geometry` — a named, planar attribute table. Design §5.
//!
//! Planar rather than interleaved, as in Houdini and USD, which is also the
//! layout the GPU wants when M5 moves these buffers onto it. One component
//! holding a map rather than one component per attribute, because an author
//! can create `@myattr` at runtime and component types cannot be registered
//! then (parent §2.10).

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::component::Component;
use bevy_math::{Vec2, Vec3, Vec4};
use bevy_reflect::TypePath;

/// One planar attribute column. `Arc` so an operator that rewrites `P` and
/// passes `N` through copies neither.
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    F32(Arc<Vec<f32>>),
    Vec2(Arc<Vec<Vec2>>),
    Vec3(Arc<Vec<Vec3>>),
    Vec4(Arc<Vec<Vec4>>),
    U32(Arc<Vec<u32>>),
}

impl Attribute {
    pub fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::Vec2(v) => v.len(),
            Self::Vec3(v) => v.len(),
            Self::Vec4(v) => v.len(),
            Self::U32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_f32(&self) -> Option<&Arc<Vec<f32>>> {
        match self {
            Self::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_vec2(&self) -> Option<&Arc<Vec<Vec2>>> {
        match self {
            Self::Vec2(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<&Arc<Vec<Vec3>>> {
        match self {
            Self::Vec3(v) => Some(v),
            _ => None,
        }
    }
}

/// Derives `TypePath` and not `Reflect`: reflecting `Arc<Vec<Vec3>>` would be
/// work with no consumer.
#[derive(Component, Clone, Debug, Default, TypePath)]
pub struct Geometry {
    attrs: BTreeMap<String, Attribute>,
    point_count: usize,
    indices: Option<Arc<Vec<u32>>>,
}

impl Geometry {
    pub fn new(point_count: usize) -> Self {
        Self {
            attrs: BTreeMap::new(),
            point_count,
            indices: None,
        }
    }

    pub fn point_count(&self) -> usize {
        self.point_count
    }

    /// Panics if `attr`'s length disagrees with `point_count`. A mismatched
    /// column is a cook bug, and the panic names it here rather than letting
    /// it surface as an out-of-bounds index during mesh upload.
    pub fn set(&mut self, name: impl Into<String>, attr: Attribute) {
        let name = name.into();
        assert_eq!(
            attr.len(),
            self.point_count,
            "attribute `{name}` has {} elements but this Geometry has {} points",
            attr.len(),
            self.point_count
        );
        self.attrs.insert(name, attr);
    }

    pub fn get(&self, name: &str) -> Option<&Attribute> {
        self.attrs.get(name)
    }

    pub fn attr_names(&self) -> impl Iterator<Item = &str> {
        self.attrs.keys().map(|k| k.as_str())
    }

    pub fn indices(&self) -> Option<&Arc<Vec<u32>>> {
        self.indices.as_ref()
    }

    pub fn set_indices(&mut self, indices: Option<Arc<Vec<u32>>>) {
        self.indices = indices;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_of(count: usize) -> Geometry {
        let mut g = Geometry::new(count);
        g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; count])));
        g.set("N", Attribute::Vec3(Arc::new(vec![Vec3::Y; count])));
        g
    }

    #[test]
    fn cloning_shares_attribute_buffers_rather_than_copying_them() {
        // Design §5: "passing an unchanged attribute through an operator is a
        // refcount bump rather than a copy". This is the property that claim
        // rests on, so it is asserted rather than described.
        let a = grid_of(4);
        let b = a.clone();

        let (Some(Attribute::Vec3(pa)), Some(Attribute::Vec3(pb))) = (a.get("P"), b.get("P"))
        else {
            panic!("P must be a Vec3 attribute");
        };
        assert!(Arc::ptr_eq(pa, pb), "clone must share, not copy");
    }

    #[test]
    fn attribute_names_iterate_in_deterministic_order() {
        // BTreeMap, not HashMap: cook output is asserted directly and mesh
        // upload walks this map, so iteration order is observable (§5).
        let mut g = Geometry::new(2);
        g.set("uv", Attribute::Vec2(Arc::new(vec![Vec2::ZERO; 2])));
        g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; 2])));
        g.set("Cd", Attribute::Vec4(Arc::new(vec![Vec4::ONE; 2])));

        assert_eq!(g.attr_names().collect::<Vec<_>>(), vec!["Cd", "P", "uv"]);
    }

    #[test]
    fn an_attribute_of_the_wrong_length_is_rejected() {
        // A mismatched attribute is a cook bug that would otherwise surface
        // as an out-of-bounds index deep in mesh upload.
        let mut g = Geometry::new(4);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.set("P", Attribute::Vec3(Arc::new(vec![Vec3::ZERO; 3])));
        }));
        assert!(result.is_err(), "length mismatch must not be accepted");
    }

    #[test]
    fn indices_round_trip_and_default_to_none() {
        let mut g = Geometry::new(3);
        assert!(g.indices().is_none());
        g.set_indices(Some(Arc::new(vec![0, 1, 2])));
        assert_eq!(
            g.indices().map(|i| i.as_slice()),
            Some([0u32, 1, 2].as_slice())
        );
    }
}
