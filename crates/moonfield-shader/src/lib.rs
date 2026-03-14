//! # Slang Rust Bindings
//!
//! Rust bindings for the Slang shading language compiler.
//!
//! ## Overview
//!
//! This crate provides Rust bindings to the Slang compiler, allowing you to:
//! - Compile HLSL/Slang shaders to various targets (SPIRV, DXIL, GLSL, etc.)
//! - Perform shader reflection
//! - Manage shader compilation sessions
//!
//! ## Architecture
//!
//! This crate is organized into several layers:
//!
//! - **`ffi`** - Low-level FFI bindings generated from Slang C headers
//! - **`sys`** - System-level wrappers around FFI (COM interface handling, etc.)
//! - **High-level API** - Safe, idiomatic Rust wrappers (TODO)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use slang_rs::GlobalSession;
//!
//! let session = GlobalSession::new()?;
//! // ... compile shaders
//! ```

#![allow(non_camel_case_types, non_snake_case)]

pub mod ffi;
pub mod sys;

// Re-export low-level types from ffi for convenience
pub use ffi::root::*;

// Version information
pub const SLANG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result type alias for Slang operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for Slang operations
#[derive(Debug, Clone)]
pub enum Error {
    /// Slang API returned an error code
    ApiError(i32),
    /// Invalid argument passed to function
    InvalidArgument(&'static str),
    /// Null pointer encountered
    NullPointer,
    /// Operation not supported
    NotSupported,
    /// String conversion error
    StringConversion,
    /// Custom error message
    Custom(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ApiError(code) => write!(f, "Slang API error: {}", code),
            Error::InvalidArgument(msg) => write!(f, "Invalid argument: {}", msg),
            Error::NullPointer => write!(f, "Null pointer encountered"),
            Error::NotSupported => write!(f, "Operation not supported"),
            Error::StringConversion => write!(f, "String conversion error"),
            Error::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for Error {}

/// Convert a SlangResult to our Error type
pub use sys::result_from_slang;