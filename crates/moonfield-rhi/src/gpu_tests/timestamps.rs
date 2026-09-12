//! Headless smoke test for address-resolved GPU timestamps.
//!
//! Two timestamps bracket a `cmd_memcpy`, the pool is resolved straight to
//! a GPU address (`vkCmdCopyQueryPoolResultsToMemoryKHR` — no
//! `vkGetQueryPoolResults` host sync), and after the submit-and-wait the
//! CPU reads two u64 tick values from the mapped allocation: both non-zero
//! and monotonic.

use super::common;
use crate::{
    CommandBufferUsage, CommandPool, Device, GpuAllocation, Instance, Memory, Stage,
    TimestampQueryPool,
};

#[test]
fn timestamps_resolve_to_gpu_address() {
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

    let queries = TimestampQueryPool::new(&device, 2).expect("timestamp query pool");
    assert!(
        queries.timestamp_period_ns() > 0.0,
        "timestamp period must be positive"
    );
    let src = GpuAllocation::new(&device, 256, Memory::Default).expect("src allocation");
    let dst = GpuAllocation::new(&device, 256, Memory::Default).expect("dst allocation");
    let results = GpuAllocation::new(&device, 16, Memory::Readback).expect("results allocation");

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.reset_timestamps(&queries);
    cmd.write_timestamp(&queries, 0, Stage::TRANSFER);
    cmd.cmd_memcpy(dst.gpu(), src.gpu(), 256);
    cmd.write_timestamp(&queries, 1, Stage::TRANSFER);
    cmd.resolve_timestamps(&queries, 0, 2, results.gpu());
    cmd.end().expect("end");
    device.submit_and_wait(&[&cmd]).expect("submit and wait");

    let host = results.host().expect("results host view");
    let (before, after) = unsafe { (*host.typed::<u64>(), *host.typed::<u64>().add(1)) };
    assert_ne!(before, 0, "first timestamp is zero");
    assert_ne!(after, 0, "second timestamp is zero");
    assert!(
        after >= before,
        "timestamps not monotonic: {before} then {after}"
    );
}
