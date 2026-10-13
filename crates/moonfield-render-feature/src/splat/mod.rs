//! 3D Gaussian splatting: scene data, I/O, and rasterization.
//!
//! Domain types only; training lives in `moonfield-ml` (see its `gs` method).

pub mod cloud;
pub mod forward;
pub mod io;
pub mod prepare;
pub mod rasterize;
pub mod scene;
pub mod sort_pass;
