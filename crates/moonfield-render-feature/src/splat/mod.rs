//! 3D Gaussian splatting: scene data, I/O, and the forward rasterizer
//! (`forward_pass` — kernels in `assets/shaders/gs/forward.slang`).
//!
//! Domain types only; training lives in `moonfield-ml` (see its `gs` method).

pub mod cloud;
pub mod forward;
pub mod forward_pass;
pub mod io;
pub mod prepare;
pub mod sort_pass;
