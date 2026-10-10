//! Slang shader compiler integration.
//!
//! Wraps the `shader-slang` crate to compile Slang source into target
//! bytecode plus reflection. Errors are mapped to the
//! [`Error`](crate::error::Error) type. The layer is target-agnostic: a
//! [`ShaderTarget`] selects the emitted code (SPIR-V for the Vulkan backend,
//! a Metal library for the Metal backend), and both backends consume the
//! same [`Compiler`]/[`ShaderCache`] and [`Reflection`].
//!
//! The module is split by responsibility, with a one-way dependency chain
//! `compile` ← `reflection`:
//!
//! - `compile` — [`Compiler`], [`CompiledShader`], and the memoizing
//!   [`ShaderCache`]: Slang source in, target code out.
//! - `reflection` — [`Reflection`], a self-referential raw-pointer wrapper
//!   (its module doc states the invariants), plus [`Layout`] and the
//!   user-attribute types.

use crate::error::Error as RenderError;

mod compile;
mod reflection;

pub use compile::{CompiledShader, Compiler, ShaderCache, ShaderTarget};
pub use reflection::{
    Layout, Reflection, RootParam, RootParamKind, UserAttributeArg, UserAttributeRef,
};

/// Map a `shader-slang` error to the RHI error type. Public for backends:
/// they compile through the shared [`Compiler`] and surface the same error
/// vocabulary.
pub fn map_slang_error(err: shader_slang::Error) -> RenderError {
    let message = match err {
        shader_slang::Error::Code(code) => format!("Slang error code: {}", code),
        shader_slang::Error::Blob(blob) => {
            blob.as_str().unwrap_or("unknown Slang error").to_string()
        }
    };
    RenderError::ShaderCompilation(message)
}
