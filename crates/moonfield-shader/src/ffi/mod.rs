//! # FFI Bindings
//!
//! Low-level FFI bindings to the Slang C API.
//!
//! This module contains the raw bindings generated from the Slang headers.
//! For a safer, more idiomatic Rust API, use the high-level wrappers.

#![allow(non_camel_case_types, non_snake_case)]

// Include the generated bindings from bindgen
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// Re-export commonly used types from the root module for convenience
pub use root::*;
