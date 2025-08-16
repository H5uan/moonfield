pub mod backend;
pub mod buffer;

mod descriptors;
pub mod error;
pub mod frame_buffer;
pub mod geometry_buffer;
mod traits;
mod types;

#[cfg(feature = "metal")]
pub mod metal;

#[cfg(feature = "vulkan")]
pub mod vulkan;

pub use crate::descriptors::*;
pub use crate::traits::*;
pub use crate::types::*;

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
