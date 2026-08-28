//! Headless integration tests for the GPU bump allocator.
//!
//! Exercises the pointer math of [`GpuBumpAllocator`] against a real device:
//! alignment (including base-alignment raising past 16 bytes), monotonic
//! offsets, `free_all` reuse, and grow-on-overflow. Copies are exercised in
//! the upload tests (`upload_ring.rs`); here only the carve surface is
//! verified.

mod common;

use moonfield_math::gpu::align_up;
use moonfield_rhi::{Device, Error, GpuBumpAllocator, Instance};

/// Create a headless instance + device, skipping on machines without one
/// (mirrors `headless_triangle.rs`).
fn setup() -> Option<(Instance, Device)> {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return None;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return None;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return None;
        }
    };
    Some((instance, device))
}

#[test]
fn alloc_offsets_are_aligned_and_monotonic() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1 << 20).expect("arena");

    let a = arena.alloc(100, 16).expect("alloc a");
    assert_eq!(a.gpu.as_raw() % 16, 0, "a must be 16-aligned");

    // A 64-aligned request exceeds the first block's 16B base alignment and
    // so moves to a grown block; it must still be aligned, and later
    // allocations in that block follow it contiguously.
    let b = arena.alloc(100, 64).expect("alloc b");
    assert_eq!(b.gpu.as_raw() % 64, 0, "b must be 64-aligned");
    assert!(b.gpu.as_raw() > a.gpu.as_raw(), "grown block starts past block 0");

    let c = arena.alloc(8, 64).expect("alloc c");
    assert_eq!(
        c.gpu.as_raw() - b.gpu.as_raw(),
        align_up(100, 64) as u64,
        "c follows b within the grown block"
    );
}

#[test]
fn cpu_and_gpu_views_share_the_same_offset() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1 << 20).expect("arena");

    let a = arena.alloc(3 * 4, 4).expect("alloc a"); // 12 bytes, aligned to 16
    let b = arena.alloc(64, 16).expect("alloc b");
    // CPU and GPU deltas between two allocations must match: both views use
    // the same offset from their respective bases.
    let cpu_delta = b.cpu.typed::<u8>() as usize - a.cpu.typed::<u8>() as usize;
    let gpu_delta = b.gpu.as_raw() as usize - a.gpu.as_raw() as usize;
    assert_eq!(cpu_delta, gpu_delta);
    assert_eq!(gpu_delta, align_up(12, 16) as usize);
}

#[test]
fn typed_alloc_strides_by_size_of_t() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1 << 20).expect("arena");

    let a = arena.alloc_typed::<u32>(4).expect("alloc a");
    let b = arena.alloc_typed::<u32>(4).expect("alloc b");
    // 16 bytes each, aligned to 4 → rounded to MIN_ALIGN 16.
    assert_eq!(b.gpu.as_raw() - a.gpu.as_raw(), 16);
}

#[test]
fn free_all_resets_to_first_block() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1 << 20).expect("arena");

    let first = arena.alloc(64, 16).expect("first alloc");
    let first_gpu = first.gpu.as_raw();
    let first_cpu = first.cpu.typed::<u8>() as usize;

    arena.alloc(256, 16).expect("second alloc");
    arena.free_all();

    let again = arena.alloc(64, 16).expect("alloc after free_all");
    assert_eq!(
        again.gpu.as_raw(),
        first_gpu,
        "gpu view returns to block start"
    );
    assert_eq!(
        again.cpu.typed::<u8>() as usize,
        first_cpu,
        "cpu view returns to block start"
    );
}

#[test]
fn overflow_grows_a_new_block() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    // Tiny first block so an overflow is cheap to provoke.
    let mut arena = GpuBumpAllocator::new(&device, 1024).expect("arena");

    let first = arena.alloc(1000, 16).expect("fills most of block 0");
    let block0_start = first.gpu.as_raw();
    arena.alloc(100, 16).expect("overflows into block 1");
    // The overflow landed in a second block, not silently wrapped.
    assert_eq!(arena.block_count(), 2, "grow-on-overflow, not ring wrap");

    arena.free_all();
    let back = arena.alloc(64, 16).expect("alloc after free_all");
    // free_all resets to block 0, whose first allocation was `first`.
    assert_eq!(back.gpu.as_raw(), block0_start, "returns to block 0 start");
}

#[test]
fn high_alignment_raises_block_base_alignment() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1 << 20).expect("arena");

    // A 256B request cannot be served by the 16B-aligned first block: the
    // allocator must grow a block whose base is co-aligned at 256, so one
    // offset aligns both the CPU and GPU view.
    let mem = arena.alloc(100, 256).expect("256-aligned alloc");
    assert_eq!(mem.gpu.as_raw() % 256, 0, "gpu view aligned");
    assert_eq!(
        mem.cpu.typed::<u8>() as usize % 256,
        0,
        "cpu view aligned — base raising took effect"
    );
    assert!(arena.block_count() >= 2, "grew past the 16B-aligned block");
}

#[test]
fn zero_size_is_rejected() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut arena = GpuBumpAllocator::new(&device, 1024).expect("arena");
    assert!(matches!(arena.alloc(0, 16), Err(Error::Validation(_))));
}
