//! # Moonfield Core
//!
//! Core utilities and foundational components for the Moonfield graphics engine.
//!
//! This crate provides essential functionality used throughout the Moonfield engine:
//!
//! - **Allocator**: Memory pool management with handle-based resource tracking
//! - **Logging**: Unified tracing-based logging system
//! - **Math**: Mathematical utilities and types
//! - **Type Traits**: Component system traits and utilities
//!
//! ## Usage
//!
//! ```rust
//! use moonfield_core::{allocator::Handle, logging::init_auto_logging};
//!
//! // Define a resource type
//! struct MyResource {
//!     data: String,
//! }
//!
//! // Initialize logging
//! init_auto_logging().expect("Failed to initialize logging");
//!
//! // Use handle-based resource management
//! let handle: Handle<MyResource> = Handle::new(1, 1);
//! ```
//!
//! ## Safety
//!
//! This crate contains unsafe code for performance-critical operations.
//! All unsafe blocks are carefully documented and tested.

use bytemuck::Pod;

pub mod allocator;
pub mod ecs;
pub mod logging;
pub mod math;
pub mod type_traits;

/// Converts a slice of `Pod` types to a slice of bytes.
///
/// This function is used to safely convert typed data to raw bytes for GPU operations.
/// The returned slice maintains the same lifetime as the original array to prevent
/// dangling pointers.
///
/// # Safety
///
/// - `T` must implement `Pod` trait (plain old data)
/// - The returned slice references the same memory as the input
///
/// # Arguments
///
/// * `v` - A slice of `Pod` types to convert
///
/// # Returns
///
/// A slice of bytes representing the input data
///
/// # Examples
///
/// ```rust
/// use moonfield_core::array_as_u8_slice;
///
/// let vertices = vec![1.0f32, 2.0f32, 3.0f32];
/// let bytes = array_as_u8_slice(&vertices);
/// // bytes can now be safely sent to GPU
/// ```
pub fn array_as_u8_slice<T: Sized + Pod>(v: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            v.as_ptr() as *const u8,
            std::mem::size_of_val(v),
        )
    }
}

/// Converts a mutable slice of `Pod` types to a mutable slice of bytes.
///
/// This function is used to safely convert typed data to raw bytes for GPU operations.
/// The returned slice maintains the same lifetime as the original array to prevent
/// dangling pointers.
///
/// # Safety
///
/// - `T` must implement `Pod` trait (plain old data)
/// - The returned slice references the same memory as the input
///
/// # Arguments
///
/// * `v` - A mutable slice of `Pod` types to convert
///
/// # Returns
///
/// A mutable slice of bytes representing the input data
///
/// # Examples
///
/// ```rust
/// use moonfield_core::array_as_u8_slice_mut;
///
/// let mut vertices = vec![1.0f32, 2.0f32, 3.0f32];
/// let bytes = array_as_u8_slice_mut(&mut vertices);
/// // bytes can now be safely sent to GPU
/// ```
pub fn array_as_u8_slice_mut<T: Sized + Pod>(v: &mut [T]) -> &mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut(
            v.as_ptr() as *mut u8,
            std::mem::size_of_val(v),
        )
    }
}

/// Macro for implementing type-safe downcasting for trait objects.
///
/// This macro generates a trait that extends a base trait with `Any` functionality,
/// allowing safe downcasting of trait objects to concrete types.
///
/// # Arguments
///
/// * `base` - The base trait to extend
/// * `ext` - The name of the new trait that extends the base trait
///
/// # Examples
///
/// ```rust
/// use moonfield_core::any_ext_for;
///
/// trait MyTrait {
///     fn do_something(&self);
/// }
///
/// any_ext_for!(MyTrait => MyTraitExt);
///
/// struct MyStruct;
/// impl MyTrait for MyStruct {
///     fn do_something(&self) {
///         println!("Hello from MyStruct!");
///     }
/// }
///
/// let obj: Box<dyn MyTraitExt> = Box::new(MyStruct);
/// if let Some(concrete) = obj.as_any().downcast_ref::<MyStruct>() {
///     concrete.do_something();
/// }
/// ```
#[macro_export]
macro_rules! any_ext_for {
    ($base:ident=>$ext:ident) => {
        /// Extension trait that adds `Any` functionality to the base trait.
        ///
        /// This trait allows safe downcasting of trait objects to concrete types.
        pub trait $ext: $base + std::any::Any {
            /// Get an immutable reference to the underlying `Any` trait object.
            fn as_any(&self) -> &dyn std::any::Any;

            /// Get a mutable reference to the underlying `Any` trait object.
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

            /// Consume the boxed trait object and return it as a boxed `Any`.
            fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
        }

        impl<T: $base + std::any::Any + 'static> $ext for T {
            #[inline(always)]
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            #[inline(always)]
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
            #[inline(always)]
            fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
                self
            }
        }
    };
}
