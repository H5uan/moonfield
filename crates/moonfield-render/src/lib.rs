//! Lunar Mare rendering infrastructure.
//!
//! Vulkan RHI implemented on top of `ash`. This crate exposes a safe Rust API
//! surface over instance, physical device, logical device, and swapchain
//! creation.

pub mod buffer;
pub mod command;
pub mod device;
pub mod error;
pub mod framebuffer;
pub mod headless;
pub mod instance;
pub mod pipeline;
pub mod plugin;
pub mod render_pass;
pub mod shader;
pub mod shader_module;
pub mod swapchain;
pub mod sync;

pub use buffer::Buffer;
pub use command::{CommandBuffer, CommandPool};
pub use device::{Device, QueueFamilyIndices};
pub use error::{Error, Result};
pub use framebuffer::Framebuffer;
pub use headless::HeadlessContext;
pub use instance::Instance;
pub use pipeline::GraphicsPipeline;
pub use plugin::RenderPlugin;
pub use render_pass::RenderPass;
pub use shader::Compiler;
pub use shader_module::ShaderModule;
pub use swapchain::{Surface, Swapchain};
pub use sync::{Fence, Semaphore};

use std::ffi::CStr;

/// Common required instance extensions for surface rendering on the current platform.
pub fn required_instance_extensions() -> Vec<&'static CStr> {
    let mut extensions = vec![ash::khr::surface::NAME];

    #[cfg(target_os = "windows")]
    extensions.push(ash::khr::win32_surface::NAME);

    #[cfg(target_os = "linux")]
    extensions.push(ash::khr::xlib_surface::NAME);

    #[cfg(target_os = "macos")]
    extensions.push(ash::ext::metal_surface::NAME);

    extensions
}
