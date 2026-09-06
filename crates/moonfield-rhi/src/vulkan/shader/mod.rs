//! Slang shader compiler integration.
//!
//! Wraps the `shader-slang` crate to compile Slang source into SPIR-V
//! bytecode. Errors are mapped to the [`Error`](crate::error::Error) type.
//!
//! The module is split by responsibility, with a one-way dependency chain
//! `compile` ← `reflection` ← `root_binder`:
//!
//! - `compile` — [`Compiler`], [`CompiledShader`], and the memoizing
//!   [`ShaderCache`]: Slang source in, SPIR-V out.
//! - `reflection` — [`Reflection`], a self-referential raw-pointer wrapper
//!   (its module doc states the invariants), plus [`Layout`] and the
//!   user-attribute types.
//! - `root_binder` — the root-parameter vocabulary ([`RootParam`],
//!   [`RootParamKind`], [`RootParamPlace`]) and [`RootBinder`], which turns
//!   reflection results into push-data blobs.

use crate::error::Error as RenderError;

mod compile;
mod reflection;
mod root_binder;

pub use compile::{CompiledShader, Compiler, ShaderCache};
pub use reflection::{Layout, Reflection, UserAttributeArg, UserAttributeRef};
pub use root_binder::{RootBinder, RootParam, RootParamKind, RootParamPlace};

fn map_slang_error(err: shader_slang::Error) -> RenderError {
    let message = match err {
        shader_slang::Error::Code(code) => format!("Slang error code: {}", code),
        shader_slang::Error::Blob(blob) => {
            blob.as_str().unwrap_or("unknown Slang error").to_string()
        }
    };
    RenderError::ShaderCompilation(message)
}
