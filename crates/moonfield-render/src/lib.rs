//! Lunar Mare rendering infrastructure.
//!
//! Rendering RHI with pluggable backends: `native` (Vulkan via `ash`) and
//! `web` (wgpu). Exactly one backend feature must be enabled.

#[cfg(all(feature = "native", feature = "web"))]
compile_error!("features `native` and `web` are mutually exclusive");
#[cfg(not(any(feature = "native", feature = "web")))]
compile_error!("either feature `native` or `web` must be enabled");

pub mod error;
pub mod types;

#[cfg(feature = "native")]
pub mod native;
#[cfg(feature = "web")]
pub mod web;

pub use error::{Error, Result};
#[cfg(feature = "native")]
pub use native::*;
pub use types::{BufferUsage, Format, VertexAttribute, VertexBufferLayout, VertexFormat};
#[cfg(feature = "web")]
pub use web::*;
