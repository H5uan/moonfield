pub mod backend;
pub mod buffer;
pub mod error;
pub mod frame_buffer;
pub mod geometry_buffer;

#[cfg(feature = "metal")]
pub mod metal_backend;

#[cfg(feature = "vulkan")]
pub mod vulkan_backend;

#[macro_export]
macro_rules! define_shared_wrapper {
    ($name:ident<$ty:ty>) => {
        #[derive(Clone)]
        #[doc(hidden)]
        pub struct $name(pub std::rc::Rc<$ty>);

        impl std::ops::Deref for $name {
            type Target = $ty;

            fn deref(&self) -> &Self::Target {
                self.0.deref()
            }
        }
    };
}
