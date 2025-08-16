use bytemuck::Pod;

pub mod allocator;
pub mod logging;
pub mod math;
pub mod type_traits;

// to avoid dangling pointer, the returned slice must have the same lifetime
// as the original array. Since the slice references the same memory address
pub fn array_as_u8_slice<'a, T: Sized + Pod>(v: &'a [T]) -> &'a [u8] {
    unsafe {
        std::slice::from_raw_parts(
            v.as_ptr() as *const u8,
            std::mem::size_of_val(v),
        )
    }
}

pub fn array_as_u8_slice_mut<'a, T: Sized + Pod>(
    v: &'a mut [T],
) -> &'a mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut(
            v.as_ptr() as *mut u8,
            std::mem::size_of_val(v),
        )
    }
}

/// This macro can downcasting trait object
#[macro_export]
macro_rules! any_ext_for {
    ($base:ident=>$ext:ident) => {
        /// add any trait for base
        pub trait $ext: $base + std::any::Any {
            fn as_any(&self) -> &dyn std::any::Any;
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
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
