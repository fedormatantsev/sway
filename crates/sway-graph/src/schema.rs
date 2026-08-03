//! Deriving a node's field schema from its `Inlets` / `Outlets` types.
//!
//! The schema is derived from the types, never written beside them. A plain
//! field is a value slot; a field typed `Events<T>` is an event field whose
//! payload is `T`; a field typed `Product<T>` is a structural field whose
//! capability is `T`.

use core::any::TypeId;
use core::fmt;

use bevy_app::App;
use bevy_ecs::entity::Entity;
use bevy_reflect::structs::StructInfo;
use bevy_reflect::{
    FromReflect, FromType, GetTypeRegistration, PartialReflect, Reflect, TypeInfo, TypePath,
    TypeRegistry, Typed,
};

use crate::ports::{clear_events_of, Events, Occurrence, Product, Spatial};

/// Type data marking a type as an `Events<T>` slot value, carrying the payload
/// type and the fn that empties one in place.
#[derive(Clone)]
pub struct ReflectEventList {
    pub payload: TypeId,
    pub payload_path: &'static str,
    pub clear: fn(&mut dyn PartialReflect),
}

impl<T> FromType<Events<T>> for ReflectEventList
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    fn from_type() -> Self {
        Self {
            payload: TypeId::of::<T>(),
            payload_path: T::type_path(),
            clear: clear_events_of::<T>,
        }
    }
}

/// Reads and writes a `Product<T>`'s source through `dyn PartialReflect`, so
/// the engine can seed an outlet and a cook can read an inlet without knowing
/// the capability.
#[derive(Clone, Copy, Debug)]
pub struct ProductAccess {
    pub get: fn(&dyn PartialReflect) -> Option<Entity>,
    pub set: fn(&mut dyn PartialReflect, Option<Entity>),
}

/// Type data marking a type as a `Product<T>` slot value.
#[derive(Clone)]
pub struct ReflectProduct {
    pub capability: TypeId,
    pub capability_path: &'static str,
    pub access: ProductAccess,
}

impl<T: TypePath + Send + Sync + 'static> FromType<Product<T>> for ReflectProduct {
    fn from_type() -> Self {
        Self {
            capability: TypeId::of::<T>(),
            capability_path: T::type_path(),
            access: ProductAccess {
                get: |value| {
                    value
                        .try_downcast_ref::<Product<T>>()
                        .and_then(|product| product.source)
                },
                set: |value, source| {
                    if let Some(product) = value.try_downcast_mut::<Product<T>>() {
                        product.source = source;
                    }
                },
            },
        }
    }
}

/// Registers `Events<T>` and its `ReflectEventList` data. A node type with an
/// `Events<T>` field must call this in its `register`.
pub fn register_events<T>(app: &mut App)
where
    T: Reflect + TypePath + Typed + FromReflect + GetTypeRegistration,
{
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    let mut registry = registry.write();
    registry.register::<T>();
    registry.register::<Occurrence<T>>();
    registry.register::<Events<T>>();
    registry.register_type_data::<Events<T>, ReflectEventList>();
}

/// Registers `Product<T>` and its `ReflectProduct` data. A node type with a
/// `Product<T>` field must call this in its `register`.
pub fn register_product<T>(app: &mut App)
where
    T: TypePath + Send + Sync + 'static,
{
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    let mut registry = registry.write();
    registry.register::<Product<T>>();
    registry.register_type_data::<Product<T>, ReflectProduct>();
}

#[derive(Debug)]
pub enum SchemaError {
    NotAStruct { type_path: &'static str },
    UnregisteredEventsField {
        type_path: &'static str,
        field: &'static str,
    },
    UnregisteredProductField {
        type_path: &'static str,
        field: &'static str,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAStruct { type_path } => write!(
                f,
                "`{type_path}` must be a struct to derive a port schema from it"
            ),
            Self::UnregisteredEventsField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like an event list but its type is not \
                 registered as one — call `register_events::<Payload>(app)` in this node \
                 type's `register`"
            ),
            Self::UnregisteredProductField { type_path, field } => write!(
                f,
                "`{type_path}.{field}` looks like a product but its type is not \
                 registered as one — call `register_product::<Capability>(app)` in this \
                 node type's `register`"
            ),
        }
    }
}

impl core::error::Error for SchemaError {}

/// Casts `T`'s reflected [`TypeInfo`] to its [`StructInfo`], or reports the
/// one error `derive_fields` and its callers share: `T` must be a struct to
/// derive a schema from it. Callers go on to walk the result with
/// `field_len`/`field_at`, so a `&'static StructInfo` — not a `dyn`-erased
/// view — is exactly what each needs.
///
/// [`TypeInfo`]: bevy_reflect::TypeInfo
pub(crate) fn struct_info<T: Typed>() -> Result<&'static StructInfo, SchemaError> {
    let info = T::type_info();
    info.as_struct().map_err(|_| SchemaError::NotAStruct {
        type_path: info.type_path(),
    })
}

/// What a field's slots carry. Derived from the field's type — for a `Vec`
/// field, from its element type.
#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    /// A plain reflect value: the slot holds it directly.
    Value,
    /// `Events<T>`: the slot holds this tick's occurrences, and is emptied
    /// before every tick through `clear`.
    Events {
        payload: TypeId,
        clear: fn(&mut dyn PartialReflect),
    },
    /// `Product<T>`: the slot holds the producing entity. `spatial` is true
    /// for the one capability the engine acts on (design §3).
    Product {
        capability: TypeId,
        spatial: bool,
        access: ProductAccess,
    },
}

/// One field of an `Inlets` or `Outlets` struct.
#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub name: &'static str,
    /// Index of the field in the reflect struct — what `Struct::field_at`
    /// takes. Equal to the field ordinal within its own struct.
    pub field_index: usize,
    pub kind: FieldKind,
    /// The type one *slot* holds: the element type for a `Vec` field, the
    /// field's own type otherwise. Edge validation compares these directly.
    pub slot_type: TypeId,
    pub slot_type_path: &'static str,
    /// A `Vec<_>` field, whose slot count comes from the instance.
    pub variadic: bool,
}

/// Derives one struct's fields. The struct is a node's `Inlets` or `Outlets`;
/// direction comes from which of the two it is, never from the field.
pub fn derive_fields<T: Typed>(registry: &TypeRegistry) -> Result<Vec<FieldSpec>, SchemaError> {
    let s = struct_info::<T>()?;

    let mut fields = Vec::with_capacity(s.field_len());
    for i in 0..s.field_len() {
        let field = s.field_at(i).expect("index below field_len");

        // A Vec field is variadic and its slots hold the element type.
        // Anything else holds the field's own type.
        let (slot_type, slot_type_path, variadic) = match field.type_info() {
            Some(TypeInfo::List(list)) => {
                let item = list.item_ty();
                (item.id(), item.path(), true)
            }
            _ => (field.type_id(), field.type_path(), false),
        };

        let kind = if let Some(events) = registry.get_type_data::<ReflectEventList>(slot_type) {
            FieldKind::Events { payload: events.payload, clear: events.clear }
        } else if let Some(product) = registry.get_type_data::<ReflectProduct>(slot_type) {
            FieldKind::Product {
                capability: product.capability,
                spatial: product.capability == TypeId::of::<Spatial>(),
                access: product.access,
            }
        } else {
            // A marker field whose type data is missing would otherwise
            // become a value port of a type no edge can usefully drive —
            // and, for Events, one that is never cleared. Catch it by path.
            if is_events_marker_path(slot_type_path) {
                return Err(SchemaError::UnregisteredEventsField {
                    type_path: s.type_path(),
                    field: field.name(),
                });
            }
            if is_product_marker_path(slot_type_path) {
                return Err(SchemaError::UnregisteredProductField {
                    type_path: s.type_path(),
                    field: field.name(),
                });
            }
            FieldKind::Value
        };

        fields.push(FieldSpec {
            name: field.name(),
            field_index: i,
            kind,
            slot_type,
            slot_type_path,
            variadic,
        });
    }
    Ok(fields)
}

/// Recognises `sway_graph::ports::Events<..>` by path. The authoritative test
/// is the `ReflectEventList` type data; this is the diagnostic for its absence.
fn is_events_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Events<")
}

/// The same, for `Product<..>`.
fn is_product_marker_path(path: &str) -> bool {
    path.starts_with("sway_graph::ports::Product<")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{Events, Product, Spatial};
    use bevy_reflect::{Reflect, TypeRegistry};

    #[derive(Reflect, Default, Debug, Clone, PartialEq)]
    struct NoteMsg {
        note: u8,
        velocity: u8,
    }

    #[derive(Reflect)]
    struct Geometry;

    #[test]
    fn a_non_struct_type_is_rejected() {
        let mut r = TypeRegistry::new();
        r.register::<f32>();
        let err = derive_fields::<f32>(&r).unwrap_err();
        assert!(err.to_string().contains("struct"), "{err}");
    }

    // --- derive_fields ------------------------------------------------

    #[derive(Reflect, Default)]
    struct SampleInlets {
        children: Vec<Product<Spatial>>,
        geo: Product<Geometry>,
        triggers: Vec<Events<NoteMsg>>,
        gain: f32,
        terms: Vec<f32>,
    }

    fn fields_registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register_type_data::<Events<NoteMsg>, ReflectEventList>();
        r.register::<Product<Spatial>>();
        r.register_type_data::<Product<Spatial>, ReflectProduct>();
        r.register::<Product<Geometry>>();
        r.register_type_data::<Product<Geometry>, ReflectProduct>();
        r
    }

    #[test]
    fn each_field_kind_is_derived_from_its_type() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        assert_eq!(
            fields.iter().map(|f| f.name).collect::<Vec<_>>(),
            vec!["children", "geo", "triggers", "gain", "terms"]
        );
        assert!(matches!(fields[0].kind, FieldKind::Product { .. }));
        assert!(matches!(fields[1].kind, FieldKind::Product { .. }));
        assert!(matches!(fields[2].kind, FieldKind::Events { .. }));
        assert!(matches!(fields[3].kind, FieldKind::Value));
        assert!(matches!(fields[4].kind, FieldKind::Value));
    }

    #[test]
    fn a_vec_field_is_variadic_and_reports_its_element_type() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        // The slot type is the ELEMENT type for a Vec field. Edge validation
        // compares slot types, and an edge connects to one element.
        assert!(fields[0].variadic, "children is Vec<_>");
        assert_eq!(fields[0].slot_type, core::any::TypeId::of::<Product<Spatial>>());
        assert!(!fields[1].variadic, "geo is a bare Product<_>");
        assert_eq!(fields[1].slot_type, core::any::TypeId::of::<Product<Geometry>>());
        assert!(fields[4].variadic, "terms is Vec<f32>");
        assert_eq!(fields[4].slot_type, core::any::TypeId::of::<f32>());
    }

    #[test]
    fn a_product_field_carries_its_capability_and_its_spatial_flag() {
        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");

        let FieldKind::Product { capability, spatial, .. } = fields[0].kind else {
            panic!("children must be a Product field");
        };
        assert_eq!(capability, core::any::TypeId::of::<Spatial>());
        assert!(spatial, "Spatial is the one capability the engine acts on");

        let FieldKind::Product { capability, spatial, .. } = fields[1].kind else {
            panic!("geo must be a Product field");
        };
        assert_eq!(capability, core::any::TypeId::of::<Geometry>());
        assert!(!spatial);
    }

    #[test]
    fn product_access_reads_and_writes_a_source_without_knowing_the_capability() {
        // This is what lets the runner seed a Product outlet and the cook
        // read a Product inlet through `dyn PartialReflect` alone.
        use bevy_ecs::entity::Entity;
        use bevy_reflect::PartialReflect;

        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");
        let FieldKind::Product { access, .. } = fields[1].kind else {
            panic!("geo must be a Product field");
        };

        let mut slot: Box<dyn PartialReflect> = Box::new(Product::<Geometry>::default());
        assert_eq!((access.get)(&*slot), None);

        let entity = Entity::from_raw_u32(11).unwrap();
        (access.set)(&mut *slot, Some(entity));
        assert_eq!((access.get)(&*slot), Some(entity));
    }

    #[test]
    fn an_events_field_carries_its_clear_fn() {
        use bevy_reflect::PartialReflect;

        let fields = derive_fields::<SampleInlets>(&fields_registry()).expect("fields");
        let FieldKind::Events { clear, .. } = fields[2].kind else {
            panic!("triggers must be an Events field");
        };

        let mut events = Events::<NoteMsg>::default();
        events.occurrences.push(Occurrence {
            offset: 0.0,
            value: NoteMsg { note: 60, velocity: 100 },
        });
        let mut slot: Box<dyn PartialReflect> = Box::new(events);

        clear(&mut *slot);

        assert!(slot
            .try_downcast_ref::<Events<NoteMsg>>()
            .expect("still Events")
            .occurrences
            .is_empty());
    }

    #[test]
    fn an_unregistered_events_field_is_an_error_not_a_value_port() {
        // The failure this prevents: a node author adds an Events<T> field,
        // forgets register_events, and it silently becomes a value port that
        // is never cleared -- so an occurrence fires forever.
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register::<Product<Spatial>>();
        r.register_type_data::<Product<Spatial>, ReflectProduct>();
        r.register::<Product<Geometry>>();
        r.register_type_data::<Product<Geometry>, ReflectProduct>();

        let msg = derive_fields::<SampleInlets>(&r).unwrap_err().to_string();
        assert!(msg.contains("triggers"), "must name the field: {msg}");
        assert!(msg.contains("register_events"), "must say the fix: {msg}");
    }

    #[test]
    fn an_unregistered_product_field_is_an_error_not_a_value_port() {
        let mut r = TypeRegistry::new();
        r.register::<NoteMsg>();
        r.register::<SampleInlets>();
        r.register::<Events<NoteMsg>>();
        r.register_type_data::<Events<NoteMsg>, ReflectEventList>();
        r.register::<Product<Spatial>>();
        r.register::<Product<Geometry>>();

        let msg = derive_fields::<SampleInlets>(&r).unwrap_err().to_string();
        assert!(msg.contains("children"), "must name the field: {msg}");
        assert!(msg.contains("register_product"), "must say the fix: {msg}");
    }
}
