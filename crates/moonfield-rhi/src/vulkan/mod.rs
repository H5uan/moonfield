//! Vulkan rendering backend.
//!
//! Vulkan RHI implemented on top of `ash`. This module exposes a safe Rust API
//! surface over instance, physical device, logical device, and swapchain
//! creation.

pub mod bindless;
pub mod buffer;
pub mod bump;
pub mod command;
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

/// Aggregated device-extension loaders, built once at device creation and
/// shared with command buffers through an `Arc` — the same shape
/// `wgpu_hal::vulkan::DeviceExtensionFunctions` has inside `Arc<DeviceShared>`.
/// A loader is a function-pointer table; cloning the `Arc` copies no tables.
pub(crate) struct DeviceExtensionFunctions {
    /// `VK_EXT_extended_dynamic_state3` per-draw blend commands. Not an
    /// `ExtensionFn`-style promoted marker: no extension this RHI uses has a
    /// core counterpart, and enriched enums for a one-extension table would be
    /// dead code.
    pub(crate) extended_dynamic_state3: ash::ext::extended_dynamic_state3::Device,
}

pub use buffer::Buffer;
pub use bump::{BumpAlloc, GpuBumpAllocator};
pub use command::{
    CommandBuffer, CommandPool, CullState, DepthState, RenderAttachment, RenderPassDesc,
};
pub use device::{Device, QueueFamilyIndices};
pub use instance::Instance;
pub use offscreen::OffscreenTarget;
pub use pipeline::{BlendMode, GraphicsPipeline, PipelineLayout, PipelineOptions};
pub use plugin::RenderDevice;
pub use shader::Compiler;
pub use shader_module::ShaderModule;
pub use swapchain::{Surface, Swapchain};
pub use sync::{Fence, Semaphore};
pub use texture::Texture;
