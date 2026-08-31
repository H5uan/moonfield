//! Headless integration tests for `Device::descriptor_heap_properties`.
//!
//! Verifies that `VK_EXT_descriptor_heap` limits are queried at device
//! creation and that the values the `DescriptorHeap` sizes its heaps from —
//! descriptor sizes, heap alignment, caps — are sane on a real driver. The
//! RHI requires these limits unconditionally, so the plain values are read
//! directly off the device. Prints the values so machine-specific data can be
//! recorded.

mod common;

use moonfield_rhi::{DescriptorHeapProperties, Device, Instance};

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
fn descriptor_heap_properties_are_reported() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let props: DescriptorHeapProperties = device.descriptor_heap_properties();
    println!("descriptor heap properties: {props:#?}");

    // Slot sizing: whatever the hardware descriptor format is, a slot must
    // occupy at least one byte and the heap base must satisfy the driver's
    // alignment requirement (a power of two, per the extension's VUIDs).
    assert!(
        props.image_descriptor_size > 0,
        "image descriptor size unset"
    );
    assert!(
        props.resource_heap_alignment > 0 && props.resource_heap_alignment.is_power_of_two(),
        "resource heap alignment must be a non-zero power of two, got {}",
        props.resource_heap_alignment
    );

    // The heap capacity and the reserved range the binding needs must be
    // positive, or a DescriptorHeap cannot be sized at all.
    assert!(
        props.max_resource_heap_size > 0,
        "max resource heap size unset"
    );
    assert!(
        props.min_resource_heap_reserved_range > 0,
        "min resource heap reserved range unset"
    );

    // Sampler heaps follow the same contract; they only carry samplers.
    assert!(
        props.sampler_descriptor_size > 0,
        "sampler descriptor size unset"
    );
    assert!(
        props.sampler_heap_alignment > 0 && props.sampler_heap_alignment.is_power_of_two(),
        "sampler heap alignment must be a non-zero power of two, got {}",
        props.sampler_heap_alignment
    );
}
