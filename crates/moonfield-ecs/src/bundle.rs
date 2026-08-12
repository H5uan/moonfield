use crate::{archetype::ComponentMeta, Component};
use std::any::TypeId;
use std::ptr::NonNull;

/// Error indicating that an entity did not have a required component
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MissingComponent(&'static str);

/// A type-erased collection of components that can be inserted into an ECS
/// archetype.
///
/// # Safety
///
/// Implementors must keep the component identities and layouts returned by
/// [`Self::with_ids`] and [`Self::component_meta`] consistent with the values
/// yielded by [`Self::put`]. For every callback invocation, the
/// [`ComponentMeta`] must describe the pointed-to initialized component value:
/// the pointer must be valid and correctly aligned for that component, and
/// the callback must not outlive the value. Implementations must also invoke
/// the callback exactly once for each component represented by the bundle.
pub unsafe trait DynamicBundle {
    fn key(&self) -> Option<TypeId> {
        None
    }

    fn has<T: Component>(&self) -> bool {
        self.with_ids(|types| types.contains(&TypeId::of::<T>()))
    }

    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T;

    fn component_meta(&self) -> Vec<ComponentMeta>;

    unsafe fn put(self, f: impl FnMut(*mut u8, ComponentMeta));
}

/// A statically described [`DynamicBundle`].
///
/// # Safety
///
/// In addition to the [`DynamicBundle`] contract, implementors must keep
/// [`Self::with_static_ids`] and [`Self::with_static_component_meta`] in the
/// same order and identity as the components returned by [`Self::get`]. The
/// callback passed to `get` may return null for a missing component; the
/// implementation must then return [`MissingComponent`] and must not
/// dereference that pointer or otherwise invoke undefined behavior.
pub unsafe trait Bundle: DynamicBundle {
    fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T;

    fn with_static_component_meta<T>(f: impl FnOnce(&[ComponentMeta]) -> T) -> T;

    unsafe fn get(
        f: impl FnMut(ComponentMeta) -> Option<NonNull<u8>>,
    ) -> Result<Self, MissingComponent>
    where
        Self: Sized;
}

#[derive(Copy, Clone)]
/// Type-erased [`Clone`] implementation
pub struct DynamicClone {
    pub(crate) func: unsafe fn(*const u8, &mut dyn FnMut(*mut u8, ComponentMeta)),
}

impl DynamicClone {
    pub fn new<T: Component + Clone>() -> Self {
        Self {
            func: |src, f| {
                let mut tmp: T = unsafe { (*src.cast::<T>()).clone() };
                f((&mut tmp as *mut T).cast(), ComponentMeta::of::<T>());
                core::mem::forget(tmp);
            },
        }
    }
}
