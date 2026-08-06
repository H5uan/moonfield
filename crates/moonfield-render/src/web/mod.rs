//! Web rendering backend (wgpu).
//!
//! Mirrors the public type names of the native backend (`Device`, `Buffer`,
//! `ShaderModule`, `GraphicsPipeline`, `OffscreenTarget`) so downstream code
//! can be written backend-agnostically. The surface is intentionally
//! asymmetric where the underlying APIs differ: wgpu has no render pass
//! objects and device creation is async (there is no blocking executor on
//! wasm), so [`Device::new_headless`] is an `async fn` and
//! [`GraphicsPipeline::new`] takes a target [`Format`] instead of a
//! `RenderPass`.
//!
//! # Shader authoring convention
//!
//! Both backends accept SPIR-V bytecode via `ShaderModule::from_spirv`:
//! natively it goes straight to `vkCreateShaderModule`; on web it is fed
//! through naga's SPIR-V frontend (wgpu's `spirv` feature), which translates
//! to WGSL on WebGPU targets. So a single offline Slang compile
//! (`slangc -target spirv`) serves both backends — embed the precompiled
//! bytes with `include_bytes!` (runtime Slang compilation is native-only and
//! there is no filesystem on wasm). [`ShaderModule::from_wgsl`] remains
//! available for hand-written WGSL.

mod buffer;
mod command;
mod compute_pipeline;
mod device;
mod offscreen;
mod pipeline;
mod pipeline_layout;
mod shader_module;

pub use buffer::Buffer;
pub use command::{CommandEncoder, RenderPass};
pub use compute_pipeline::ComputePipeline;
pub use device::Device;
pub use offscreen::OffscreenTarget;
pub use pipeline::GraphicsPipeline;
pub use pipeline_layout::PipelineLayout;
pub use shader_module::ShaderModule;
