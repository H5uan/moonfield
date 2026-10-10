//! Shared RHI vocabulary: resource descriptions, formats, indirect-argument
//! layouts, and error types.
//!
//! Backend-agnostic by construction. The vocabulary lives below the backend
//! sub-crates (`moonfield-rhi-vulkan`, `moonfield-rhi-metal`): every backend
//! consumes these types, and nothing here may mention a backend type.

pub mod error;
pub mod indirect;
pub mod shader;
pub mod types;

pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndirectArgs};
pub use shader::{
    CompiledShader, Compiler, Layout, Reflection, RootParam, RootParamKind, ShaderCache,
    ShaderTarget, UserAttributeArg, UserAttributeRef,
};
pub use types::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, Extent2d, Filter,
    Format, FrontFace, LoadOp, Offset2d, Rect2d, SamplerDesc, StoreOp, Viewport, WrapMode,
};
