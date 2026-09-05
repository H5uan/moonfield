//! Headless integration tests for the frame-scoped uploader.
//!
//! Verifies that many uploads in one frame complete with a single submit,
//! and that arena reuse across frames never clobbers in-flight copies.

use super::common;

use crate::Memory;
use crate::{CommandBufferUsage, CommandPool, Device, FrameUploader, GpuAllocation, Instance};

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

/// Deterministic byte pattern so each upload is distinguishable after
/// round-tripping through the ring arena.
fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

/// Copy a GPU-only allocation into a fresh readback allocation and read it
/// back. A second, independent submit: the upload path is exercised, then
/// the result is drained through a separate copy + `submit_and_wait`.
fn readback(device: &Device, src: &GpuAllocation, n: usize) -> Vec<u8> {
    let dst = GpuAllocation::new(device, n as u64, Memory::Readback).expect("readback allocation");
    let pool = CommandPool::new(device, device.queue_family_indices().graphics).expect("pool");
    let mut cb = pool.allocate_command_buffer().expect("command buffer");
    cb.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cb.cmd_memcpy(&dst, src, n as u64);
    cb.end().expect("end");
    device.submit_and_wait(&[&cb]).expect("submit and wait");

    let mut out = vec![0u8; n];
    dst.read_bytes(&mut out).expect("read back");
    out
}

#[test]
fn one_frame_carries_many_uploads() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut uploader = FrameUploader::new(&device, 1 << 20).expect("uploader");

    // Three GPU-only destinations; the carrier always carries TRANSFER_SRC,
    // so readback can use them as copy sources.
    let sizes = [256usize, 512, 1024];
    let dsts: Vec<GpuAllocation> = sizes
        .iter()
        .map(|&n| {
            GpuAllocation::new(&device, n as u64, Memory::Gpu).expect("destination allocation")
        })
        .collect();
    let expected: Vec<Vec<u8>> = sizes.iter().map(|&n| pattern(n, 1)).collect();

    uploader.begin_frame().expect("begin");
    for (dst, data) in dsts.iter().zip(&expected) {
        uploader.upload_alloc(dst, data.as_slice()).expect("upload");
    }
    uploader.end_frame().expect("end");
    uploader.wait_idle().expect("wait");

    // All three copies were produced by the single end_frame submit.
    for (dst, data) in dsts.iter().zip(&expected) {
        assert_eq!(readback(&device, dst, data.len()), *data, "uploaded data");
    }
}

#[test]
fn cross_frame_reuse_does_not_clobber() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut uploader = FrameUploader::new(&device, 1 << 20).expect("uploader");

    // Three frames, each staging through the ring arena (slot 0 is reused by
    // frame 3 after wait(3 - RING)). If free_all ran before the first frame's
    // copy finished, the reused arena bytes would corrupt the first result.
    let n = 4096usize;
    let seeds = [0xAAu8, 0x55, 0x5A];
    let dsts: Vec<GpuAllocation> = seeds
        .iter()
        .map(|_| {
            GpuAllocation::new(&device, n as u64, Memory::Gpu).expect("destination allocation")
        })
        .collect();
    let expected: Vec<Vec<u8>> = seeds.iter().map(|&s| pattern(n, s)).collect();

    for (dst, data) in dsts.iter().zip(&expected) {
        uploader.begin_frame().expect("begin");
        uploader.upload_alloc(dst, data.as_slice()).expect("upload");
        uploader.end_frame().expect("end");
        // No wait here: the next begin_frame's timeline wait is the reclaim.
    }
    uploader.wait_idle().expect("wait");

    for (dst, data) in dsts.iter().zip(&expected) {
        assert_eq!(readback(&device, dst, data.len()), *data, "frame data");
    }
}
