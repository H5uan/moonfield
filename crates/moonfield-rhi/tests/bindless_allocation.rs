//! Headless smoke test for the bindless allocation path.
//!
//! Verifies that a `GpuAllocation` pairs a writable CPU pointer with a
//! non-zero buffer device address, that the memory classes map to the
//! expected CPU visibility, and that dropping defers teardown to the
//! retirement drain.

use moonfield_rhi::{Device, GpuAllocation, Instance, Memory};
use std::sync::Mutex;
mod common;

/// Serializes the tests in this binary. Each test creates its own Vulkan
/// instance, device, and allocator; doing so concurrently on one GPU
/// access-violates on some Windows drivers, and the crate confines Vulkan
/// objects to a single thread by rule — tests must not create devices in
/// parallel.
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// A CPU-side view must exist for host-visible classes and be absent for
/// GPU-only memory; the GPU address must be non-zero for every class.
#[test]
fn bindless_allocation_views_and_addresses() {
    let _guard = DEVICE_LOCK.lock().unwrap();
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let default = GpuAllocation::new(&device, 4096, Memory::Default).expect("Default allocation");
    assert!(
        default.host().is_some(),
        "Default must expose a CPU pointer"
    );
    assert_ne!(
        default.gpu().as_raw(),
        0,
        "Default must expose a GPU address"
    );

    let gpu_only = GpuAllocation::new(&device, 4096, Memory::Gpu).expect("Gpu allocation");
    assert!(gpu_only.host().is_none(), "GpuOnly must not expose a host");
    assert_ne!(
        gpu_only.gpu().as_raw(),
        0,
        "Gpu allocation must expose a GPU address"
    );

    let readback =
        GpuAllocation::new(&device, 4096, Memory::Readback).expect("Readback allocation");
    assert!(readback.host().is_some(), "Readback must expose a host");
    assert_ne!(
        readback.gpu().as_raw(),
        0,
        "Readback must expose a GPU address"
    );
}

/// A full allocation can be CPU-written and dropped without panic.
#[test]
fn bindless_allocation_write_and_drop() {
    let _guard = DEVICE_LOCK.lock().unwrap();
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let allocation = GpuAllocation::new(&device, 4096, Memory::Default).expect("allocation");
    let host = allocation.host().expect("host view");

    // CPU writes through the mapped pointer; on coherent host memory these
    // are immediately visible to the GPU (verified implicitly next milestone).
    let values = [1u32, 2, 4, 8];
    unsafe {
        std::ptr::copy_nonoverlapping(values.as_ptr(), host.typed::<u32>(), values.len());
        assert_eq!(*host.typed::<u32>(), 1);
    }

    drop(allocation); // teardown deferred into the retirement ring
    // No GPU work ever referenced the allocation, so draining now is safe
    // and exercises the deferred teardown path.
    device.flush_retirements();
}
