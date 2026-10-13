//! Lunar Mare — the rendering RHI.
//!
//! The facade owns the public interface: the backend-agnostic vocabulary
//! (`types`, `error`, `indirect`, `shader`) and the re-export list below,
//! which is the RHI's entire public surface — anything not re-exported is
//! not public API. The backend is the private `backend::vulkan` module, the
//! pivot API the vocabulary is shaped around. The engine layer (extraction,
//! view snapshots, the window frame loop, `RenderPlugin`) lives in
//! `moonfield-render-core` (Selene), not here.

pub mod error;
pub mod indirect;
pub mod shader;
pub mod types;

mod backend;

pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndirectArgs};
pub use shader::{
    CompiledShader, Compiler, Layout, Reflection, RootBinder, RootParam, RootParamKind,
    RootParamPlace, ShaderCache, ShaderTarget, UserAttributeArg, UserAttributeRef,
};
pub use types::{
    AttachmentLayout, Capabilities, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d,
    Filter, Format, FrontFace, LoadOp, MemoryResidency, Offset2d, RETIRE_RING, Rect2d, SamplerDesc,
    Stage, StoreOp, Viewport, WrapMode,
};

/// The shader compilation target of the backend. Runtime shader setup
/// derives from this instead of naming a target per call site; explicit
/// targets remain for offline tooling.
pub const fn shader_target() -> ShaderTarget {
    ShaderTarget::Spirv
}

/// The Slang capability atoms the backend's bindless shaders require (the
/// descriptor-heap codegen needs `spvDescriptorHeapEXT`).
pub const fn shader_capabilities() -> &'static [&'static str] {
    &["spvDescriptorHeapEXT"]
}

pub use backend::vulkan::{
    AcquiredImage, BlendMode, BufferRange, BumpAlloc, CommandBuffer, ComputePipeline, CullState,
    DESCRIPTOR_HEAP_IMAGE_CAPACITY, DESCRIPTOR_HEAP_SAMPLER_CAPACITY, DepthBuffer, DepthState,
    DescriptorHeap, DescriptorHeapProperties, Device, FrameUploader, GpuAllocation,
    GpuBumpAllocator, GpuPtr, GraphicsPipeline, HostPtr, Instance, OffscreenTarget,
    RenderAttachment, RenderDevice, RenderPassDesc, SamplerHandle, ShaderModule, ShaderStageDesc,
    Surface, Swapchain, Texture, TextureHandle, TextureView, Timeline, UPLOAD_ARENA_SIZE,
    UPLOAD_FRAME_RING,
};
