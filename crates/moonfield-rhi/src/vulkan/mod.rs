//! Vulkan rendering backend.
//!
//! Vulkan RHI implemented on top of `ash`. This module exposes a safe Rust API
//! surface over instance, physical device, logical device, and swapchain
//! creation.

pub mod bindless;
pub mod buffer;
pub mod bump;
pub mod command;
pub mod descriptor_heap;
pub mod device;
pub mod instance;
pub mod offscreen;
pub mod pipeline;
pub mod plugin;
pub mod shader;
pub mod shader_module;
pub mod swapchain;
pub mod sync;
pub mod texture;
pub mod upload;

/// Aggregated device-extension loaders, built once at device creation and
/// shared with command buffers through an `Arc` — the same shape
/// `wgpu_hal::vulkan::DeviceExtensionFunctions` has inside `Arc<DeviceShared>`.
/// A loader is a function-pointer table; cloning the `Arc` copies no tables.
pub(crate) struct DeviceExtensionFunctions {
    /// `VK_EXT_extended_dynamic_state3` per-draw blend commands. Loaders are
    /// plain function-pointer tables, not promoted core features; the RHI
    /// keeps them here so extension entry points are loaded once and shared
    /// with every command buffer through the `Arc`.
    pub(crate) extended_dynamic_state3: ash::ext::extended_dynamic_state3::Device,
    /// `VK_EXT_descriptor_heap` CPU-visible descriptor heap writes and binding.
    /// Loaded at device creation, consumed by `DescriptorHeap`.
    pub(crate) descriptor_heap: ash::ext::descriptor_heap::Device,
}

pub use buffer::Buffer;
pub use bump::{BumpAlloc, GpuBumpAllocator};
pub use command::{
    CommandBuffer, CommandPool, CullState, DepthState, RenderAttachment, RenderPassDesc,
};
pub use descriptor_heap::{
    DescriptorHeap, SamplerHandle, TextureHandle, TextureSlotDesc, DESCRIPTOR_HEAP_IMAGE_CAPACITY,
    DESCRIPTOR_HEAP_SAMPLER_CAPACITY,
};
pub use device::{DescriptorHeapProperties, Device, QueueFamilyIndices};
pub use instance::Instance;
pub use offscreen::OffscreenTarget;
pub use pipeline::{
    BlendMode, GraphicsPipeline, HeapMapping, HeapMappingResource, PipelineOptions,
};
pub use plugin::RenderDevice;
pub use shader::Compiler;
pub use shader_module::ShaderModule;
pub use swapchain::{Surface, Swapchain};
pub use sync::{Fence, Semaphore};
pub use texture::Texture;
pub use upload::{FrameUploader, UPLOAD_ARENA_SIZE, UPLOAD_FRAME_RING};
