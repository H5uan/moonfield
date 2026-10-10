//! Metal 4 rendering backend.
//!
//! Metal RHI implemented on top of `objc2-metal` (the `MTL4*` API surface).
//! The backend-agnostic vocabulary (`Format`, `Viewport`, error types, ...) is
//! re-exported from `moonfield-rhi-core` at the crate root, so in-crate
//! `crate::types`-style paths resolve.

pub use moonfield_rhi_core::*;

pub mod bump;
pub mod command;
pub mod device;
pub mod formats;
pub mod instance;
pub mod memory;
pub mod pipeline;
pub mod plugin;
pub mod shader_module;
pub mod swapchain;
pub mod sync;
pub mod texture;
pub mod view;

#[cfg(test)]
mod gpu_tests;

pub use bump::{BumpAlloc, GpuBumpAllocator};
pub use command::{CommandBuffer, CommandPool, RenderAttachment, RenderPassDesc};
pub use device::Device;
pub use instance::Instance;
pub use memory::{GpuAllocation, GpuPtr, HostPtr, Memory};
pub use pipeline::{ComputePipeline, GraphicsPipeline};
pub use plugin::RenderDevice;
pub use shader_module::ShaderModule;
pub use swapchain::{Surface, Swapchain};
pub use sync::Semaphore;
pub use texture::Texture;
pub use view::TextureView;
