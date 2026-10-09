//! Headless integration tests for [`DepthBuffer`].
//!
//! The window-targeted 3D pass depth-tests against a per-window
//! `DepthBuffer` owned by the surface data; these tests cover its standalone
//! contract: creation, extent, and resize (which retires the old image
//! through the ring).

use super::common;

use crate::{DepthBuffer, Device, Instance};

/// Create a headless instance + device, skipping on machines without one
/// (mirrors `descriptor_heap_properties.rs`).
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
fn depth_buffer_create_and_resize() {
    let Some((_instance, device)) = setup() else {
        return;
    };

    let mut depth = DepthBuffer::new(&device, 640, 480).expect("depth buffer");
    assert_eq!(depth.extent(), (640, 480));

    // Resize swaps in a new image; the old one retires through the ring.
    depth.resize(&device, 800, 600).expect("resize");
    assert_eq!(depth.extent(), (800, 600));

    // Same-extent resize is a no-op; zero dimensions are ignored.
    depth.resize(&device, 800, 600).expect("same extent");
    assert_eq!(depth.extent(), (800, 600));
    depth.resize(&device, 0, 600).expect("zero width ignored");
    assert_eq!(depth.extent(), (800, 600));

    assert!(DepthBuffer::new(&device, 0, 0).is_err());
}
