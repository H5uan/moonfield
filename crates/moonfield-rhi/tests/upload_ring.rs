//! Headless integration tests for the frame-scoped uploader.
//!
//! Verifies that many uploads in one frame complete with a single submit,
//! that arena reuse across frames never clobbers in-flight copies, and that
//! host-visible targets are rejected (they are written directly, not staged).

mod common;

use ash::vk;
use moonfield_rhi::Memory;
use moonfield_rhi::{
    Buffer, BufferUsage, CommandBufferUsage, CommandPool, Device, Error, FrameUploader, Instance,
};

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

/// Copy a GpuOnly buffer into a fresh GpuToCpu buffer and read it back.
/// A second, independent submit: the upload path is exercised, then the
/// result is drained through a separate copy + `submit_and_wait`.
fn readback(device: &Device, src: &Buffer, n: usize) -> Vec<u8> {
    let dst = Buffer::new(device, n as u64, BufferUsage::STORAGE, Memory::Readback)
        .expect("readback buffer");
    let pool = CommandPool::new(device, device.queue_family_indices().graphics).expect("pool");
    let mut cb = pool.allocate_command_buffer().expect("command buffer");
    cb.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    let copy = vk::BufferCopy::default()
        .src_offset(0)
        .dst_offset(0)
        .size(n as u64);
    // SAFETY: both buffers exist, the copy fits, and the command buffer is
    // recording.
    unsafe {
        device
            .raw()
            .cmd_copy_buffer(cb.raw(), src.raw(), dst.raw(), &[copy]);
    }
    cb.end().expect("end");
    device.submit_and_wait(&[&cb]).expect("submit and wait");

    let mut out = vec![0u8; n];
    dst.read(&mut out).expect("read back");
    out
}

#[test]
fn one_frame_carries_many_uploads() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut uploader = FrameUploader::new(&device, 1 << 20).expect("uploader");

    // Three GpuOnly destinations; COPY_SRC so readback can use them as copy
    // sources (uploads only need COPY_DST, which Buffer::new ORs in).
    let sizes = [256usize, 512, 1024];
    let bufs: Vec<Buffer> = sizes
        .iter()
        .map(|&n| {
            Buffer::new(
                &device,
                n as u64,
                BufferUsage::STORAGE | BufferUsage::COPY_SRC,
                Memory::Gpu,
            )
            .expect("destination buffer")
        })
        .collect();
    let expected: Vec<Vec<u8>> = sizes.iter().map(|&n| pattern(n, 1)).collect();

    uploader.begin_frame().expect("begin");
    for (buf, data) in bufs.iter().zip(&expected) {
        uploader.upload(buf, data.as_slice()).expect("upload");
    }
    uploader.end_frame().expect("end");
    uploader.wait_idle().expect("wait");

    // All three copies were produced by the single end_frame submit.
    for (buf, data) in bufs.iter().zip(&expected) {
        assert_eq!(readback(&device, buf, data.len()), *data, "uploaded data");
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
    let bufs: Vec<Buffer> = seeds
        .iter()
        .map(|_| {
            Buffer::new(
                &device,
                n as u64,
                BufferUsage::STORAGE | BufferUsage::COPY_SRC,
                Memory::Gpu,
            )
            .expect("destination buffer")
        })
        .collect();
    let expected: Vec<Vec<u8>> = seeds.iter().map(|&s| pattern(n, s)).collect();

    for (buf, data) in bufs.iter().zip(&expected) {
        uploader.begin_frame().expect("begin");
        uploader.upload(buf, data.as_slice()).expect("upload");
        uploader.end_frame().expect("end");
        // No wait here: the next begin_frame's timeline wait is the reclaim.
    }
    uploader.wait_idle().expect("wait");

    for (buf, data) in bufs.iter().zip(&expected) {
        assert_eq!(readback(&device, buf, data.len()), *data, "frame data");
    }
}

#[test]
fn host_visible_target_is_rejected() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut uploader = FrameUploader::new(&device, 1 << 20).expect("uploader");

    // Host-visible buffers are written directly by the caller; the uploader
    // refuses to stage into them so no one accidentally double-paths.
    let host =
        Buffer::new(&device, 64, BufferUsage::STORAGE, Memory::Default).expect("host buffer");
    assert!(matches!(
        uploader.upload(&host, &[1u8, 2, 3]),
        Err(Error::Validation(_))
    ));
}

#[test]
fn upload_and_wait_sync_path() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let mut uploader = FrameUploader::new(&device, 1 << 20).expect("uploader");

    // The one-shot helper runs a full frame and waits — the load-time path.
    let n = 2048usize;
    let data = pattern(n, 0x42);
    let dst = Buffer::new(
        &device,
        n as u64,
        BufferUsage::STORAGE | BufferUsage::COPY_SRC,
        Memory::Gpu,
    )
    .expect("destination buffer");

    uploader
        .upload_and_wait(&dst, data.as_slice())
        .expect("upload and wait");
    assert_eq!(readback(&device, &dst, n), data);
}
