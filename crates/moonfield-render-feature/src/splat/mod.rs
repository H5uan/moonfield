//! 3D Gaussian splatting: scene data, I/O, and rasterization.
//!
//! Domain types only; training lives in `moonfield-ml` (see its `gs` method).

pub mod cloud;
pub mod io;
pub mod rasterize;
pub mod scene;
