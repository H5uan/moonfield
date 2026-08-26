//! Lunar Mare — the Vulkan rendering RHI, implemented with `ash`.
//!
//! Pure backend surface: GPU resources, command recording, and the resource
//! description vocabulary in [`types`]. The engine layer (extraction, view
//! snapshots, the window frame loop, `RenderPlugin`) lives in
//! `moonfield-render-core` (Selene), not here.

pub mod bind;
pub mod error;
pub mod indirect;
pub mod types;

pub mod vulkan;

pub use bind::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    BufferRef, Sampler, ShaderStage, TextureView,
};
pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndexedIndirectArgs, DrawIndirectArgs, IndexFormat};
pub use types::{
    AttachmentLayout, BufferUsage, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d,
    Filter, Format, FrontFace, LoadOp, Offset2d, PushConstantRange, Rect2d, SamplerDesc,
    ShaderStages, StoreOp, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport, WrapMode,
};
pub use vulkan::*;
