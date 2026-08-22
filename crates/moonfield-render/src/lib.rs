//! Lunar Mare rendering infrastructure.
//!
//! Rendering RHI for Vulkan, implemented with `ash`.

pub mod bind;
pub mod camera;
pub mod error;
pub mod indirect;
pub mod scene;
pub mod types;

pub mod vulkan;

pub use camera::perspective_reverse_z;
pub use scene::{view_matrix, Camera, PrimaryCamera};

pub use bind::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    BufferRef, Sampler, ShaderStage, TextureView,
};
pub use error::{Error, Result};
pub use indirect::{DispatchIndirectArgs, DrawIndexedIndirectArgs, DrawIndirectArgs, IndexFormat};
pub use types::{BufferUsage, Format, VertexAttribute, VertexBufferLayout, VertexFormat};
pub use vulkan::*;
