//! Lunar Mare — the Vulkan rendering RHI, implemented with `ash`.
//!
//! Pure backend surface: GPU resources, command recording, and the resource
//! description vocabulary in [`types`]. The engine layer (extraction, view
//! snapshots, the window frame loop, `RenderPlugin`) lives in
//! `moonfield-render-core` (Selene), not here.

pub mod error;
pub mod indirect;
pub mod types;
pub mod view;

pub mod vulkan;

pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndexedIndirectArgs, DrawIndirectArgs, IndexFormat};
pub use types::{
    AttachmentLayout, BufferUsage, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d,
    Filter, Format, FrontFace, LoadOp, Offset2d, Rect2d, SamplerDesc, StoreOp, VertexAttribute,
    VertexBufferLayout, VertexFormat, Viewport, WrapMode,
};
pub use view::TextureView;
pub use vulkan::*;
