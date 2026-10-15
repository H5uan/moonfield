//! Headless smoke test for address-resolved GPU timestamps.
//!
//! Two timestamps bracket a `cmd_memcpy`, the pool is resolved straight to
//! a GPU address (`vkCmdCopyQueryPoolResultsToMemoryKHR` — no
//! `vkGetQueryPoolResults` host sync), and after the submit-and-wait the
//! CPU reads two u64 tick values from the mapped allocation: both non-zero
//! and monotonic.

use super::super::sync::TimestampQueryPool;
use super::common;
use crate::{CommandBufferUsage, Device, GpuAllocation, Instance, MemoryResidency, Stage};

#[test]
fn timestamps_resolve_to_gpu_address() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            common::skip(format!("no Vulkan instance available ({err}"));
            return;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            common::skip(format!("no Vulkan device available ({err}"));
            return;
        }
    };
    if !device.device_address_commands() {
        common::skip("VK_KHR_device_address_commands is not supported by this driver");
        return;
    }

    let queries = TimestampQueryPool::new(&device, 2).expect("timestamp query pool");
    assert!(
        queries.timestamp_period_ns() > 0.0,
        "timestamp period must be positive"
    );
    let src =
        GpuAllocation::new(&device, 256, MemoryResidency::CpuWriteOnly).expect("src allocation");
    let dst =
        GpuAllocation::new(&device, 256, MemoryResidency::CpuWriteOnly).expect("dst allocation");
    let results =
        GpuAllocation::new(&device, 16, MemoryResidency::CpuReadOnly).expect("results allocation");

    let cmd = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("command buffer");
    cmd.reset_timestamps(&queries);
    cmd.write_timestamp(&queries, 0, Stage::TRANSFER);
    cmd.cmd_memcpy(dst.gpu_ptr(), src.gpu_ptr(), 256);
    cmd.write_timestamp(&queries, 1, Stage::TRANSFER);
    cmd.resolve_timestamps(&queries, 0, 2, results.gpu_ptr());
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
