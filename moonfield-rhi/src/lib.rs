pub mod backend;
pub mod buffer;

pub mod error;
pub mod frame_buffer;
pub mod geometry_buffer;
pub mod dynamic;

mod rhi_types;

#[cfg(feature = "metal")]
pub mod metal;

#[cfg(feature = "vulkan")]
pub mod vulkan;


pub use crate::rhi_types::*;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineError {}
