use crate::{Component, archetype::ComponentMeta};
use std::any::{TypeId, type_name};
use std::fmt;
use std::mem;
use std::ptr::NonNull;

/// Error indicating that an entity did not have a required component
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MissingComponent(&'static str);

impl MissingComponent {
    pub fn new<T: Component>() -> Self {
        Self(type_name::<T>())
    }
}

impl fmt::Display for MissingComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing {} component", self.0)
    }
}

impl core::error::Error for MissingComponent {}

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

    /// Move each component value out of the bundle, invoking `f` with a
    /// pointer to it and its [`ComponentMeta`].
    ///
    /// # Safety
    ///
    /// Every pointer handed to `f` points to a valid, initialized value of the
    /// type described by the accompanying [`ComponentMeta`], and the callback
    /// must not outlive that value. Each component is yielded exactly once;
    /// the bundle must not be used afterward.
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

    /// Rebuild the bundle by reading each component out of the storage
    /// provided by `f`.
    ///
    /// # Safety
    ///
    /// The pointer returned by `f` for a given [`ComponentMeta`] must point to
    /// a valid, initialized value of the described type (or be null, in which
    /// case [`MissingComponent`] is returned and the pointer is never
    /// dereferenced). Each value is read out exactly once.
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

/// A dynamically typed collection of cloneable components
///
/// # Safety
///
/// The callback passed to [`Self::put_with_clone`] must be invoked with a
/// pointer to a valid, initialized component value described by the
/// accompanying [`ComponentMeta`], plus an appropriate [`DynamicClone`].
pub unsafe trait DynamicBundleClone: DynamicBundle {
    /// Allow a callback to move all components out of the bundle, cloning each
    /// one with a type-erased [`DynamicClone`].
    ///
    /// # Safety
    ///
    /// See the trait-level contract.
    unsafe fn put_with_clone(self, f: impl FnMut(*mut u8, ComponentMeta, DynamicClone));
}

macro_rules! tuple_impl {
    ($($name:ident),*) => {
        unsafe impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            fn has<T: Component>(&self) -> bool {
                false $(|| TypeId::of::<$name>() == TypeId::of::<T>())*
            }

            fn key(&self) -> Option<TypeId> {
                Some(TypeId::of::<Self>())
            }

            fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
                Self::with_static_ids(f)
            }

            fn component_meta(&self) -> Vec<ComponentMeta> {
                Self::with_static_component_meta(|info| info.to_vec())
            }

            #[allow(unused_variables, unused_mut)]
            unsafe fn put(self, mut f: impl FnMut(*mut u8, ComponentMeta)) {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = self;
                $(
                    f(
                        (&mut $name as *mut $name).cast::<u8>(),
                        ComponentMeta::of::<$name>()
                    );
                    mem::forget($name);
                )*
            }
        }

        unsafe impl<$($name: Component + Clone),*> DynamicBundleClone for ($($name,)*) {
            // Compiler false positive warnings
            #[allow(unused_variables, unused_mut)]
            unsafe fn put_with_clone(
                self,
                mut f: impl FnMut(*mut u8, ComponentMeta, DynamicClone),
            ) {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = self;
                $(
                    f(
                        (&mut $name as *mut $name).cast::<u8>(),
                        ComponentMeta::of::<$name>(),
                        DynamicClone::new::<$name>()
                    );
                    mem::forget($name);
                )*
            }
        }

        #[allow(clippy::zero_repeat_side_effects)]
        unsafe impl<$($name: Component),*> Bundle for ($($name,)*) {
            fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T {
                const N: usize = count!($($name),*);
                let mut xs: [(usize, TypeId); N] =
                    [$((mem::align_of::<$name>(), TypeId::of::<$name>())),*];
                xs.sort_unstable_by(|x, y| x.0.cmp(&y.0).reverse().then(x.1.cmp(&y.1)));
                let mut ids = [TypeId::of::<()>(); N];
                for (slot, &(_, id)) in ids.iter_mut().zip(xs.iter()) {
                    *slot = id;
                }
                f(&ids)
            }

            fn with_static_component_meta<T>(f: impl FnOnce(&[ComponentMeta]) -> T) -> T {
                const N: usize = count!($($name),*);
                let mut xs: [ComponentMeta; N] = [$(ComponentMeta::of::<$name>()),*];
                xs.sort_unstable();
                f(&xs)
            }

            #[allow(unused_variables, unused_mut)]
            // The 0-tuple expansion reads nothing, so the unsafe block inside
            // is flagged as needless; larger tuples need it for `read()`.
            #[allow(unused_unsafe, clippy::unused_unit)]
            unsafe fn get(
                mut f: impl FnMut(ComponentMeta) -> Option<NonNull<u8>>,
            ) -> Result<Self, MissingComponent> {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = ($(
                    f(ComponentMeta::of::<$name>())
                        .ok_or_else(MissingComponent::new::<$name>)?
                        .as_ptr()
                        .cast::<$name>(),)*
                );
                // SAFETY: every pointer came from `f`, which hands out valid
                // pointers to live components of that type.
                Ok(unsafe { ($($name.read(),)*) })
            }
        }
    };
}

macro_rules! count {
    () => { 0 };
    ($x:ident $(, $rest:ident)*) => { 1 + count!($($rest),*) };
}

smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
