//! Headless smoke test for [`CommandBuffer::push_data`] — the descriptor-heap
//! extension's push-constant storage class.
//!
//! Recording two push-data updates (offset 0 and 64) must succeed and go
//! through validation cleanly; the command buffer is not submitted, so the
//! test covers the host-side recording path only (the GPU-side payload is
//! exercised once a pipeline consumes root data in phase 4).

mod common;

use moonfield_rhi::{CommandBufferUsage, CommandPool, Device, Instance};

#[test]
fn push_data_records_cleanly() {
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

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");

    // Two updates at distinct offsets, as the extension's push data bank is
    // offset-addressed like push constants.
    cmd.push_data(0, &[0u8; 64]);
    cmd.push_data(64, &[1u8; 64]);

    cmd.end().expect("end");
    // Not submitted: recording without validation errors is the contract.
}
