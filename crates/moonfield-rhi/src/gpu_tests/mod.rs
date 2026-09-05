//! GPU integration tests, in-crate so they can verify `pub(crate)` internals
//! (the public API exposes no raw handles). Every test skips gracefully when
//! no compatible driver is present — see [`common`].
//!
//! Run with `cargo test -p moonfield-rhi` (or a single module, e.g.
//! `cargo test -p moonfield-rhi gpu_tests::headless_triangle`).

mod bindless_allocation;
mod bindless_barrier;
mod bindless_compute;
mod bindless_graphics_heap_sampling;
mod bindless_memcpy_dispatch_indirect;
mod bump_allocator;
mod command_push_data;
mod common;
mod depth_occlusion;
mod descriptor_heap;
mod descriptor_heap_properties;
mod descriptor_heap_sampling;
mod gaussian_fit;
mod graphics_heap_sampling;
mod headless_triangle;
mod indirect_draw;
mod offscreen_triangle;
mod texture_bindless;
mod upload_ring;
