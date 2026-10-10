//! Lunar Mare — the rendering RHI.
//!
//! Facade crate over the backend sub-crates. The backend-agnostic vocabulary
//! lives in `moonfield-rhi-core`; the concrete resource and command types
//! come from the selected backend sub-crate (`moonfield-rhi-vulkan` under
//! the `vulkan` feature). The re-export lists below are the RHI's entire
//! public surface — backend sub-crate items not listed here are not public
//! API. The engine layer (extraction, view snapshots, the window frame loop,
//! `RenderPlugin`) lives in `moonfield-render-core` (Selene), not here.

pub use moonfield_rhi_core::error::{Error, Result};
pub use moonfield_rhi_core::indirect::{DispatchIndirectArgs, DrawIndirectArgs};
pub use moonfield_rhi_core::shader::ShaderTarget;
pub use moonfield_rhi_core::types::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d, Filter,
    Format, FrontFace, LoadOp, Offset2d, Rect2d, SamplerDesc, StoreOp, Viewport, WrapMode,
};
pub use moonfield_rhi_core::{error, indirect, types};

#[cfg(all(feature = "vulkan", feature = "metal"))]
compile_error!(
    "moonfield-rhi: the `vulkan` and `metal` backend features are mutually exclusive — \
     enable exactly one (the editor is the selection point)"
);

#[cfg(feature = "vulkan")]
pub use moonfield_rhi_vulkan::{
    Access, BlendMode, BufferRange, BumpAlloc, CommandBuffer, CommandPool, CompiledShader,
    Compiler, ComputePipeline, CullState, DESCRIPTOR_HEAP_IMAGE_CAPACITY,
    DESCRIPTOR_HEAP_SAMPLER_CAPACITY, DepthBuffer, DepthState, DescriptorHeap,
    DescriptorHeapProperties, Device, Fence, FrameUploader, GpuAllocation, GpuBumpAllocator,
    GpuPtr, GraphicsPipeline, HostPtr, Instance, Memory, OffscreenTarget, QueueFamilyIndices,
    RETIRE_RING, Reflection, RenderAttachment, RenderDevice, RenderPassDesc, RootBinder, RootParam,
    RootParamKind, RootParamPlace, SamplerHandle, Semaphore, ShaderCache, ShaderModule,
    ShaderStageDesc, Stage, Surface, Swapchain, Texture, TextureHandle, TextureView,
    TimestampQueryPool, UPLOAD_ARENA_SIZE, UPLOAD_FRAME_RING, UserAttributeArg, UserAttributeRef,
};

#[cfg(feature = "metal")]
pub use moonfield_rhi_metal::{
    Access, BlendMode, BumpAlloc, CommandBuffer, CommandPool, ComputePipeline, CullState,
    DepthState, Device, GpuAllocation, GpuBumpAllocator, GpuPtr, GraphicsPipeline, HostPtr,
    Instance, Memory, RenderAttachment, RenderDevice, RenderPassDesc, Semaphore, ShaderModule,
    Stage, Surface, Swapchain, Texture, TextureView,
};
