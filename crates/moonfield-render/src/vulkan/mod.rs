//! Vulkan rendering backend.
//!
//! Vulkan RHI implemented on top of `ash`. This module exposes a safe Rust API
//! surface over instance, physical device, logical device, and swapchain
//! creation.

pub mod bindless;
pub mod buffer;
pub mod command;
pub mod device;
pub mod framebuffer;
pub mod instance;
pub mod offscreen;
pub mod pipeline;
pub mod plugin;
pub mod render_pass;
pub mod shader;
pub mod shader_module;
pub mod swapchain;
pub mod sync;
pub mod window_target;

pub use buffer::Buffer;
pub use command::{CommandBuffer, CommandPool};
pub use device::{Device, QueueFamilyIndices};
pub use framebuffer::Framebuffer;
pub use instance::Instance;
pub use offscreen::OffscreenTarget;
pub use pipeline::{BlendMode, CullMode, GraphicsPipeline, PipelineOptions};
pub use plugin::{RenderDevice, RenderPlugin};
pub use render_pass::RenderPass;
pub use shader::Compiler;
pub use shader_module::ShaderModule;
pub use swapchain::{Surface, Swapchain};
pub use sync::{Fence, Semaphore};
pub use window_target::WindowRenderer;
