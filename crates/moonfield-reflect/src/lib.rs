//! Mini reflection for moonfield — just enough for the editor inspector.
//!
//! **Not** bevy_reflect: no `DynamicStruct`, no type registry, no
//! serialization, no change detection. A type implementing [`Reflect`]
//! (usually via `#[derive(Reflect)]` from `moonfield-reflect-derive`)
//! enumerates its named fields and exposes them as `&dyn Reflect` /
//! `&mut dyn Reflect` for dynamic read/write; leaf types (numbers, glam
//! vectors, …) have no fields and are edited through [`Any`](std::any::Any)
//! downcasts by the consumer.
//!
//! Dependency note: this crate depends on `glam` directly (not on
//! `moonfield-math`) so that `moonfield-math` can itself derive `Reflect`
//! (e.g. for `Transform`) without a dependency cycle. If the workspace ever
//! swaps glam out, this crate's leaf impls are the one place to update.

// Allow the derive's `::moonfield_reflect` paths to resolve inside this
// crate's own unit tests.
extern crate self as moonfield_reflect;

use std::any::Any;

pub use moonfield_reflect_derive::Reflect;

/// Static metadata for one named field of a reflected struct.
#[derive(Debug, Clone, Copy)]
pub struct FieldInfo {
    /// The field's name as written in the struct.
    pub name: &'static str,
    /// The field's type name (`std::any::type_name`); a fn pointer because
    /// `type_name` is not callable in const contexts on stable Rust.
    pub type_name: fn() -> &'static str,
}

impl FieldInfo {
    /// The field's type name.
    pub fn type_name(&self) -> &'static str {
        (self.type_name)()
    }
}

impl PartialEq for FieldInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.type_name() == other.type_name()
    }
}

impl Eq for FieldInfo {}

/// Const-constructible accessor for a type's name; used by the derive.
#[doc(hidden)]
pub fn type_name_of<T: 'static>() -> &'static str {
    std::any::type_name::<T>()
}

/// Mini reflection: named-field enumeration and dynamic field access.
///
/// Struct types get their impl from `#[derive(Reflect)]`. Leaf types (no
/// fields — `field_infos` returns an empty slice) ship hand-written impls
/// below and are meant to be recognized by consumers through
/// [`as_any_mut`](Reflect::as_any_mut) downcasts.
pub trait Reflect: 'static {
    /// The type's name (`std::any::type_name`).
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Static metadata of the struct's named fields, in declaration order.
    /// Empty for leaf types.
    fn field_infos(&self) -> &'static [FieldInfo] {
        &[]
    }

    /// Read a field by name as a dynamic value.
    fn field(&self, name: &str) -> Option<&dyn Reflect> {
        let _ = name;
        None
    }

    /// Get a field by name as a mutable dynamic value.
    fn field_mut(&mut self, name: &str) -> Option<&mut dyn Reflect> {
        let _ = name;
        None
    }

    /// Downcast support for leaf widgets.
    fn as_any(&self) -> &dyn Any;

    /// Downcast support for leaf widgets.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Leaf `Reflect` impl for a type with no fields.
macro_rules! impl_reflect_leaf {
    ($($t:ty),* $(,)?) => {
        $(
            impl Reflect for $t {
                fn as_any(&self) -> &dyn Any {
                    self
                }

                fn as_any_mut(&mut self) -> &mut dyn Any {
                    self
                }
            }
        )*
    };
}

impl_reflect_leaf!(bool, f32, f64, u32, i32, usize, String);
impl_reflect_leaf!(glam::Vec2, glam::Vec3, glam::Vec4, glam::Quat);
impl_reflect_leaf!([f32; 3], [f32; 4]);

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Reflect)]
    struct Inner {
        value: f32,
        label: String,
    }

    #[derive(Reflect)]
    struct Outer {
        inner: Inner,
        position: glam::Vec3,
        count: u32,
        #[reflect(ignore)]
        not_reflected: NotReflect,
    }

    /// Field type that is not Reflect; excluded via `#[reflect(ignore)]`.
    struct NotReflect;

    fn sample() -> Outer {
        Outer {
            inner: Inner {
                value: 1.5,
                label: "hello".into(),
            },
            position: glam::Vec3::new(1.0, 2.0, 3.0),
            count: 7,
            not_reflected: NotReflect,
        }
    }

    #[test]
    fn test_field_enumeration() {
        let outer = sample();
        let _ = &outer.not_reflected; // field exists, just not reflected
        let infos = outer.field_infos();
        let names: Vec<&str> = infos.iter().map(|f| f.name).collect();
        assert_eq!(names, ["inner", "position", "count"]);
        assert_eq!(infos[1].type_name(), std::any::type_name::<glam::Vec3>());
    }

    #[test]
    fn test_field_read() {
        let outer = sample();
        let position = outer.field("position").unwrap();
        let vec = position.as_any().downcast_ref::<glam::Vec3>().unwrap();
        assert_eq!(*vec, glam::Vec3::new(1.0, 2.0, 3.0));
        assert!(outer.field("not_reflected").is_none());
        assert!(outer.field("nope").is_none());
    }

    #[test]
    fn test_field_write_roundtrip() {
        let mut outer = sample();
        *outer
            .field_mut("count")
            .unwrap()
            .as_any_mut()
            .downcast_mut::<u32>()
            .unwrap() = 42;
        assert_eq!(outer.count, 42);
    }

    #[test]
    fn test_nested_struct_access() {
        let mut outer = sample();
        let inner = outer.field_mut("inner").unwrap();
        // The nested struct is itself Reflect: its fields enumerate.
        let inner_names: Vec<&str> = inner.field_infos().iter().map(|f| f.name).collect();
        assert_eq!(inner_names, ["value", "label"]);
        *inner
            .field_mut("value")
            .unwrap()
            .as_any_mut()
            .downcast_mut::<f32>()
            .unwrap() = 9.0;
        assert_eq!(outer.inner.value, 9.0);
    }

    #[test]
    fn test_leaf_types_have_no_fields() {
        let v = glam::Vec3::ONE;
        assert!(v.field_infos().is_empty());
        assert!(v.field("x").is_none());
        assert_eq!(v.type_name(), std::any::type_name::<glam::Vec3>());
    }
}
