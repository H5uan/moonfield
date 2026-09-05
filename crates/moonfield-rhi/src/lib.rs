//! Lunar Mare — the Vulkan rendering RHI, implemented with `ash`.
//!
//! Pure backend surface: GPU resources, command recording, and the resource
//! description vocabulary in [`types`]. The engine layer (extraction, view
//! snapshots, the window frame loop, `RenderPlugin`) lives in
//! `moonfield-render-core` (Selene), not here.

pub mod error;
pub mod indirect;
pub mod types;

pub mod vulkan;

#[cfg(test)]
mod gpu_tests;

pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndirectArgs};
pub use types::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d, Filter,
    Format, FrontFace, LoadOp, Offset2d, Rect2d, SamplerDesc, StoreOp, Viewport, WrapMode,
};
pub use vulkan::*;
