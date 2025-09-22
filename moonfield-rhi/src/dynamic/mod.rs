use core::fmt::Debug;
use std::any::Any;

/// Base trait for all resources, allows downcasting via [`Any`].
pub trait DynResource: Any + Debug + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

macro_rules! impl_dyn_resource {
    ($($type:ty),*) => {
        $(
            impl crate::DynResource for $type {
                fn as_any(&self) -> &dyn ::core::any::Any {
                    self
                }

                fn as_any_mut(&mut self) -> &mut dyn ::core::any::Any {
                    self
                }
            }
        )*
    };
}
pub(crate) use impl_dyn_resource;

pub trait DynCommandBuffer: DynResource + Debug {}
pub trait DynBuffer: DynResource + Debug {}
pub trait DynTexture: DynResource + Debug {}
pub trait DynTextureView: DynResource + Debug {}
pub trait DynSurfaceTexture:
    DynResource + core::borrow::Borrow<dyn DynTexture> + Debug {
}
pub trait DyncQuerySet: DynResource + Debug {}
pub trait DynFence: DynResource + Debug {}
pub trait DynSampler: DynResource + Debug {}

pub trait DynRenderPipeline: DynResource + Debug {}
pub trait DynComputePipeline: DynResource + Debug {}
pub trait DynRayTracingPipeline: DynResource + Debug {}
pub trait DynPipelineCache: DynResource + Debug {}

pub trait DynAccelerationStructure: DynResource + Debug {}

pub trait DynShaderModule: DynResource + Debug {}
pub trait DynInputLayout: DynResource + Debug {}
