use core::fmt::Debug;
use std::any::Any;

/// Base trait for all RHI objects, allows downcasting via [`Any`].
///
/// This trait provides a common interface for all graphics API objects
/// including devices, resources, pipelines, encoders, and other RHI constructs.
pub trait DynObject: Any + Debug + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

macro_rules! impl_dyn_object {
    ($($type:ty),*) => {
        $(
            impl crate::DynObject for $type {
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
pub(crate) use impl_dyn_object;

pub trait DynSurface: DynObject + Debug {}
pub trait DynAdapter: DynObject + Debug {}
pub trait DynDevice: DynObject + Debug {}

pub trait DynResource: DynObject + Debug {}
pub trait DynBuffer: DynResource + Debug {}
pub trait DynTexture: DynResource + Debug {}
pub trait DynTextureView: DynResource + Debug {}
pub trait DynSurfaceTexture:
    DynResource + core::borrow::Borrow<dyn DynTexture> + Debug {
}
pub trait DynSampler: DynResource + Debug {}
pub trait DynAccelerationStructure: DynResource + Debug {}

pub trait DynShaderProgram: DynObject + Debug {}
pub trait DynShaderObject: DynObject + Debug {}
pub trait DynShaderTable: DynObject + Debug {}
pub trait DynPipeline: DynObject + Debug {}
pub trait DynRenderPipeline: DynPipeline + Debug {}
pub trait DynComputePipeline: DynPipeline + Debug {}
pub trait DynRayTracingPipeline: DynPipeline + Debug {}

pub trait DynCommandBuffer: DynObject + Debug {}
pub trait DynCommandEncoder: DynObject + Debug {}
pub trait DynPassEncoder: DynObject + Debug {}
pub trait RenderPassEncoder: DynPassEncoder + Debug {}
pub trait ComputePassEncoder: DynPassEncoder + Debug {}
pub trait DynRayTracingPassEncoder: DynPassEncoder + Debug {}
pub trait DynCommandQueue: DynResource + Debug {}

pub trait DynInputLayout: DynObject + Debug {}
pub trait DynFence: DynObject + Debug {}
pub trait DynQueryPool: DynObject + Debug {}
pub trait DynPersistentCache: DynObject + Debug {}
pub trait DynHeap: DynObject + Debug {}
