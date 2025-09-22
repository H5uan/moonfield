mod basic;
mod descriptors;
mod enums;
mod errors;

use std::fmt::Debug;

pub use basic::*;
pub use descriptors::*;
pub use enums::*;
pub use errors::*;

use crate::dynamic::{
    ComputePassEncoder, DynAccelerationStructure, DynAdapter, DynBuffer,
    DynCommandBuffer, DynCommandEncoder, DynCommandQueue, DynComputePipeline,
    DynDevice, DynFence, DynHeap, DynInputLayout, DynInstance, DynPassEncoder,
    DynPersistentCache, DynQueryPool, DynRayTracingPassEncoder,
    DynRayTracingPipeline, DynRenderPipeline, DynResource, DynSampler,
    DynShaderObject, DynShaderProgram, DynShaderTable, DynSurface,
    DynSurfaceTexture, DynTexture, DynTextureView, RenderPassEncoder,
};

pub type DeviceAddress = u64;
pub type Size = usize;
pub type Offset = usize;

pub const TIMEOUT_INFINITE: u64 = 0xFFFFFFFFFFFFFFFF;
pub const DEFAULT_ALIGNMENT: usize = 0xffffffff;
pub const REMAINING_TEXTURE_SIZE: u32 = 0xffffffff;
pub const ALL_LAYERS: u32 = 0xffffffff;
pub const ALL_MIPS: u32 = 0xffffffff;
pub const MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT: u32 = 2;

pub const ENTIRE_BUFFER: BufferRange = BufferRange { offset: 0, size: !0 };
pub const ENTIRE_TEXTURE: SubresourceRange = SubresourceRange {
    layer: 0,
    layer_count: ALL_LAYERS,
    mip: 0,
    mip_count: ALL_MIPS,
};
pub const ALL_SUBRESOURCES: SubresourceRange = SubresourceRange {
    layer: 0,
    layer_count: ALL_LAYERS,
    mip: 0,
    mip_count: ALL_MIPS,
};

pub type Label<'a> = Option<&'a str>;

pub trait Api: Clone + Debug + Sized + 'static {
    const VARIANT: Backend;

    type Instance: DynInstance;
    type Surface: DynSurface;
    type Adapter: DynAdapter;
    type Device: DynDevice;

    type Queue: DynCommandQueue;
    type CommandEncoder: DynCommandEncoder;
    type CommandBuffer: DynCommandBuffer;

    // Resource types
    type Resource: DynResource;
    type Buffer: DynBuffer;
    type Texture: DynTexture;
    type SurfaceTexture: DynSurfaceTexture + core::borrow::Borrow<Self::Texture>;
    type TextureView: DynTextureView;
    type Sampler: DynSampler;
    type AccelerationStructure: DynAccelerationStructure;

    // Shader and pipeline types
    type ShaderProgram: DynShaderProgram;
    type ShaderObject: DynShaderObject;
    type ShaderTable: DynShaderTable;
    type RenderPipeline: DynRenderPipeline;
    type ComputePipeline: DynComputePipeline;
    type RayTracingPipeline: DynRayTracingPipeline;

    // Pass encoder types
    type PassEncoder: DynPassEncoder;
    type RenderPassEncoder: RenderPassEncoder;
    type ComputePassEncoder: ComputePassEncoder;
    type RayTracingPassEncoder: DynRayTracingPassEncoder;

    // Other resource types
    type InputLayout: DynInputLayout;
    type Fence: DynFence;
    type QueryPool: DynQueryPool;
    type PersistentCache: DynPersistentCache;
    type Heap: DynHeap;
}
