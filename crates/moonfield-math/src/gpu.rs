//! GPU upload support: `Pod` guarantees and memory-alignment invariants.
//!
//! This module is deliberately **layout-agnostic**. The slang shader compiler
//! owns the buffer layout rules (`structuredBuffer` / `[[block]]` / std430 etc.)
//! — how shader structs are *interpreted* in memory is slang's job. This module
//! only guarantees that moonfield's math types are **uploadable** (`Pod`) and
//! **correctly aligned** for GPU storage, so that uploading a `&[T]` to a
//! compute/storage buffer is always safe *given* the shader side matches.
//!
//! # Alignment model
//!
//! The classic GPU pitfall is `Vec3` (and `Mat3`): in Rust it is 12 bytes and
//! 4-byte aligned, but GPU storage alignment for a `vec3<f32>` is 16 bytes.
//! Moonfield's strategy is to **re-export `glam`'s `Vec3`/`Mat4` as-is** (they
//! are `Pod` and 16-byte aligned for the matrix types) and, where a struct must
//! be stored in a GPU buffer, rely on:
//!
//! - `#[repr(C)]` field ordering mirroring the slang struct declaration order,
//! - `bytemuck::Pod` for cast-to-bytes,
//! - [`align_up`]/[`align_to_u32`]/[`align_to_vec4`] helpers for explicit
//!   padding when a field needs it.
//!
//! # The `Vec3` padding trap
//!
//! `glam::Vec3` is 12 bytes and 4-byte aligned in Rust — useful as a CPU
//! vector, but **not** directly storable as a GPU `vec3<f32>` in a struct
//! without manual padding, because GPU storage alignment for `vec3` is 16 bytes.
//! Always lay out GPU-facing structs with explicit `[f32; 3] + pad` or
//! `Vec4` fields, and let [`padded_size`] confirm the array stride.
//!
//! The **single source of truth for the exact byte layout is the slang shader**;
//! a reflection-based guard (see `moonfield-rhi`) asserts Rust
//! `size_of`/`offset_of` against the slang struct so the two can never drift.

use core::mem::{align_of, size_of};

/// The GPU alignment requirement for all `vec4`-shaped storage (16 bytes).
pub const ALIGN_VEC4: usize = 16;

/// Returns `size` rounded up to the next multiple of `align`.
///
/// Used to compute field offsets in a manually-packed GPU struct.
#[must_use]
pub const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

/// Aligns a byte offset to `u32` (4-byte) alignment.
#[must_use]
pub const fn align_to_u32(offset: usize) -> usize {
    align_up(offset, align_of::<u32>())
}

/// Aligns a byte offset to `u64` (8-byte) alignment.
#[must_use]
pub const fn align_to_u64(offset: usize) -> usize {
    align_up(offset, align_of::<u64>())
}

/// Aligns a byte offset to GPU `vec4` (16-byte) alignment.
#[must_use]
pub const fn align_to_vec4(offset: usize) -> usize {
    align_up(offset, ALIGN_VEC4)
}

/// The padded byte size of a `Pod` type `T` when rounded to a whole multiple of
/// `align` — the size a GPU buffer must allocate for a single instance so that
/// a `[T; N]` array keeps every element aligned.
///
/// # Panics
///
/// Panics if `align` is not a power of two.
#[must_use]
pub const fn padded_size<T: bytemuck::Pod>(align: usize) -> usize {
    assert!(align.is_power_of_two(), "align must be a power of two");
    align_up(size_of::<T>(), align)
}

/// Returns the byte size of a `Pod` type `T` at compile time.
///
/// Convenience wrapper around [`core::mem::size_of`] so GPU struct definitions
/// read uniformly.
#[must_use]
pub const fn byte_size<T: bytemuck::Pod>() -> usize {
    size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mat4, Vec3};

    #[test]
    fn test_align_up_rounds_up() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(15, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 16), 32);
    }

    #[test]
    fn test_vec3_padded_to_vec4() {
        // Vec3 is 12 bytes / 4-aligned in Rust; GPU storage wants 16.
        assert_eq!(size_of::<Vec3>(), 12);
        assert_eq!(align_up(size_of::<Vec3>(), ALIGN_VEC4), 16);
    }

    #[test]
    fn test_mat4_is_pod_and_16_aligned() {
        fn assert_pod<T: bytemuck::Pod>() {}
        assert_pod::<Mat4>();
        assert_pod::<Vec3>();
        assert_eq!(align_of::<Mat4>(), 16);
        assert_eq!(size_of::<Mat4>(), 64);
    }

    #[test]
    fn test_padded_size_matches_array_element() {
        // A [T; N] array pads each element to the alignment.
        assert_eq!(padded_size::<Vec3>(ALIGN_VEC4), 16);
        assert_eq!(padded_size::<Mat4>(ALIGN_VEC4), 64);
    }

    #[test]
    fn test_domain_volumes_are_pod_and_uploadable() {
        use crate::{Aabb3d, BoundingSphere};
        fn assert_pod<T: bytemuck::Pod>() {}
        assert_pod::<Aabb3d>();
        assert_pod::<BoundingSphere>();
        // Aabb3d = 2 * Vec3 (12 bytes each) = 24 bytes.
        assert_eq!(size_of::<Aabb3d>(), 24);
        // BoundingSphere = Vec3 (12) + f32 radius (4) = 16 bytes.
        assert_eq!(size_of::<BoundingSphere>(), 16);
        // Each is a single uploadable blob with no internal padding.
        assert_eq!(align_up(size_of::<Aabb3d>(), ALIGN_VEC4), 32);
        assert_eq!(align_up(size_of::<BoundingSphere>(), ALIGN_VEC4), 16);
    }
}
