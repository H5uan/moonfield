//! Headless integration tests for the bindless [`DescriptorHeap`].
//!
//! Covers the slot allocator (alloc/reuse/exhaustion) and the descriptor
//! write paths against a real device's `VK_EXT_descriptor_heap` CPU-visible
//! heap: buffer descriptors encode live device-address ranges into the heap
//! mapping and sampler create infos likewise. Image-descriptor writes are
//! crate-internal (`TextureSlotDesc` is not public); their coverage lives in
//! the end-to-end sampling tests (`texture_bindless.rs`,
//! `descriptor_heap_sampling.rs`).

mod common;

use moonfield_rhi::{
    BufferRange, DescriptorHeap, Device, GpuAllocation, Instance, Memory, SamplerDesc,
    TextureHandle,
};

/// Create a headless instance + device, skipping on machines without one
/// (mirrors `bump_allocator.rs`).
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
fn slots_alloc_reuse_and_exhaust() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 4, 4).expect("descriptor heap");

    // Bump allocation hands out ascending slots.
    let a = heap.alloc_image_slot().expect("slot 0");
    assert_eq!(a, TextureHandle(0));
    let b = heap.alloc_image_slot().expect("slot 1");
    assert_eq!(b, TextureHandle(1));
    assert_eq!(heap.alloc_image_slot().expect("slot 2"), TextureHandle(2));

    // Freeing makes the slot the next allocation returns.
    heap.free_image_slot(b).expect("free slot 1");
    assert_eq!(
        heap.alloc_image_slot().expect("reused slot"),
        TextureHandle(1)
    );

    // Exhaustion errors instead of overflowing.
    heap.alloc_image_slot().expect("slot 3");
    assert!(
        heap.alloc_image_slot().is_err(),
        "heap is full (capacity 4)"
    );

    // Double-free and out-of-range frees are rejected.
    heap.free_image_slot(TextureHandle(1))
        .expect("free slot 1 again");
    assert!(
        heap.free_image_slot(TextureHandle(1)).is_err(),
        "double free"
    );
    assert!(
        heap.free_image_slot(TextureHandle(99)).is_err(),
        "never-allocated slot"
    );

    // Sampler slots have the same contract.
    let s = heap.alloc_sampler_slot().expect("sampler slot 0");
    assert_eq!(s.0, 0);
    heap.free_sampler_slot(s).expect("free sampler slot");
}

#[test]
fn write_buffer_descriptors_accepts_live_ranges() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 16, 8).expect("descriptor heap");
    let alloc = GpuAllocation::new(&device, 64, Memory::Default).expect("allocation");

    let handle = heap.alloc_image_slot().expect("slot");
    heap.write_buffer_descriptors(&[(
        handle,
        BufferRange {
            address: alloc.gpu(),
            size: alloc.size(),
        },
    )])
    .expect("write buffer descriptor");

    // Overwrite in place — the bump-arena contract: reallocate, then write
    // again before referencing.
    let handle2 = heap.alloc_image_slot().expect("slot 2");
    heap.free_image_slot(handle).expect("free slot");
    heap.write_buffer_descriptors(&[(
        handle,
        BufferRange {
            address: alloc.gpu(),
            size: alloc.size(),
        },
    )])
    .expect("rewrite buffer descriptor");
    assert_ne!(handle, handle2);
}

#[test]
fn write_samplers_encodes_slots() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 16, 8).expect("descriptor heap");
    let desc = SamplerDesc::default();

    let s = heap.alloc_sampler_slot().expect("sampler slot");
    heap.write_samplers(&[(s, desc)]).expect("write sampler");
    // Same slot, same descriptor: re-encoding in place must not error.
    heap.write_samplers(&[(s, desc)]).expect("rewrite sampler");
    let s2 = heap.alloc_sampler_slot().expect("sampler slot 2");
    assert_ne!(s, s2, "slots are distinct until freed");
}
