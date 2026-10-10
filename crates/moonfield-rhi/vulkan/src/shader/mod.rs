//! Slang shader compiler integration (Vulkan side).
//!
//! The target-agnostic compiler and reflection live in
//! `moonfield-rhi-core::shader`; this module adds the Vulkan-specific
//! binding vocabulary (`root_binder`) and re-exports the shared types so
//! the crate surface keeps one path to them.
//!
//! - core `shader::compile` — [`Compiler`], [`CompiledShader`], and the
//!   memoizing [`ShaderCache`]: Slang source in, target code out.
//! - core `shader::reflection` — [`Reflection`], a self-referential
//!   raw-pointer wrapper, plus [`Layout`] and the user-attribute types.
//! - `root_binder` — the root-parameter vocabulary ([`RootParam`],
//!   [`RootParamKind`], [`RootParamPlace`]) and [`RootBinder`], which turns
//!   reflection results into push-data blobs.

mod root_binder;

pub use moonfield_rhi_core::shader::{
    CompiledShader, Compiler, Layout, Reflection, RootParam, RootParamKind, ShaderCache,
    ShaderTarget, UserAttributeArg, UserAttributeRef,
};
pub use root_binder::{RootBinder, RootParamPlace};
