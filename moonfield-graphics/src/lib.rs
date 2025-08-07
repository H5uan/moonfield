use core::error::Error;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineError {}

pub trait Device {
    fn create_buffer(&self);
    fn destroy_buffer(&self);
    /// map gpu buffer to cpu
    fn map_buffer(&self);
    fn unmap_buffer(&self);

    fn create_shader_module(&self);
    fn destroy_shader_module(&self);

    fn create_pipeline_layout(&self);
    fn destroy_pipeline_layout(&self);

    fn create_render_pipeline(&self);
    fn destroy_render_pipeline(&self);

    fn create_pipeline_cache(&self);
    fn destroy_pipeline_cache(&self);

    fn create_surface(&self);
}
