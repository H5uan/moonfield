//! Headless integration tests for [`Texture::bindless`] — the bindless 2.0
//! texture path: creation uploads RGBA8 pixels through a frame uploader and
//! writes the view's descriptor into the shared descriptor heap in one step.
//!
//! Covers slot allocation order, slot reuse after the retirement drain, and
//! the escape-hatch [`Texture::new`] path that must not touch the heap.

use super::common;

use crate::{Device, Format, FrameUploader, Instance, Texture, UPLOAD_ARENA_SIZE};
use std::sync::Mutex;

/// Serializes the tests in this binary. Each test creates its own Vulkan
/// instance, device, and uploader; doing so concurrently on one GPU
/// access-violates on some Windows drivers, and the crate confines Vulkan
/// objects to a single thread by rule — tests must not create devices in
/// parallel (mirrors `bindless_allocation.rs`).
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// Create a headless instance + device with a private frame uploader,
/// skipping on machines without one (mirrors the other RHI test setups).
fn setup() -> Option<(Instance, Device, FrameUploader)> {
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
    let uploader = match FrameUploader::new(&device, UPLOAD_ARENA_SIZE) {
        Ok(uploader) => uploader,
        Err(err) => {
            eprintln!("skipping: no frame uploader available ({err})");
            return None;
        }
    };
    Some((instance, device, uploader))
}

/// A tiny RGBA8 checkerboard (4x4).
fn checkerboard() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4u8 {
        for x in 0..4u8 {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            bytes.extend_from_slice(&[v, v, v, 255]);
        }
    }
    bytes
}

#[test]
fn bindless_allocates_slots_in_order() {
    let _guard = DEVICE_LOCK.lock().unwrap();
    let Some((_instance, device, mut uploader)) = setup() else {
        return;
    };
    let pixels = checkerboard();
    let a = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("texture a");
    let b = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("texture b");
    assert_eq!(a.handle(), Some(crate::TextureHandle(0)), "first slot");
    assert_eq!(b.handle(), Some(crate::TextureHandle(1)), "second slot");

    // The upload path must submit cleanly: both uploads were queued on the
    // shared uploader; end the frame and wait for it to be executed.
    uploader.end_frame().expect("end upload frame");
    uploader.wait_idle().expect("wait for uploads");
}

#[test]
fn drop_releases_slot_for_reuse() {
    let _guard = DEVICE_LOCK.lock().unwrap();
    let Some((_instance, device, mut uploader)) = setup() else {
        return;
    };
    let pixels = checkerboard();
    let a = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("texture a");
    assert_eq!(a.handle(), Some(crate::TextureHandle(0)));

    // Dropping the texture retires its slot; the drain returns it to the
    // freelist. The upload must complete first (nothing in flight may
    // reference the image), so the retirement drain is safe.
    drop(a);
    uploader.end_frame().expect("end upload frame");
    uploader.wait_idle().expect("wait for uploads");
    device.flush_retirements();
    let b = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("texture b");
    assert_eq!(b.handle(), Some(crate::TextureHandle(0)), "slot reused");

    uploader.end_frame().expect("end upload frame");
    uploader.wait_idle().expect("wait for uploads");
}

#[test]
fn escape_hatch_has_no_slot() {
    let _guard = DEVICE_LOCK.lock().unwrap();
    let Some((_instance, device, mut _uploader)) = setup() else {
        return;
    };
    // The egui interop path: plain image, no descriptor heap slot.
    let texture = Texture::new(&device, 4, 4, Format::R8G8B8A8Unorm).expect("texture");
    assert_eq!(texture.handle(), None, "escape-hatch textures have no slot");
}
