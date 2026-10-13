//! Memory contract tests against the facade's public API.
//!
//! The suite pins the public behavior of `GpuAllocation`/`GpuPtr`/`HostPtr`
//! (residency semantics, host visibility, readback validation, and base
//! alignment) — the contract any later backend has to honor.

mod common;

use moonfield_rhi::{Error, GpuAllocation, MemoryResidency};

#[test]
fn test_allocation_per_residency() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let cpu_write = GpuAllocation::new(&device, 4096, MemoryResidency::CpuWriteOnly)
        .expect("CpuWriteOnly allocation");
    let gpu_only =
        GpuAllocation::new(&device, 4096, MemoryResidency::GpuOnly).expect("GpuOnly allocation");
    let cpu_read = GpuAllocation::new(&device, 4096, MemoryResidency::CpuReadOnly)
        .expect("CpuReadOnly allocation");

    for alloc in [&cpu_write, &gpu_only, &cpu_read] {
        assert_eq!(alloc.size(), 4096);
        assert_ne!(
            alloc.gpu_ptr().as_raw(),
            0,
            "device address must be non-zero"
        );
    }
    let addresses = [
        cpu_write.gpu_ptr().as_raw(),
        gpu_only.gpu_ptr().as_raw(),
        cpu_read.gpu_ptr().as_raw(),
    ];
    for (i, a) in addresses.iter().enumerate() {
        for (j, b) in addresses.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "distinct allocations must have distinct addresses");
            }
        }
    }

    assert!(cpu_write.host().is_some(), "CpuWriteOnly is host-visible");
    assert!(gpu_only.host().is_none(), "GpuOnly is not host-visible");
    assert!(cpu_read.host().is_some(), "CpuReadOnly is host-visible");
}

#[test]
fn test_host_write_read_bytes_roundtrip() {
    let Some(device) = common::headless_device() else {
        return;
    };

    for residency in [MemoryResidency::CpuWriteOnly, MemoryResidency::CpuReadOnly] {
        let alloc = GpuAllocation::new(&device, 64, residency).expect("host-visible allocation");
        let values: *mut u32 = alloc.host().expect("host-visible allocation").typed();
        let expected: [u32; 16] = std::array::from_fn(|i| (i as u32) * 7 + 1);
        // SAFETY: the allocation is 64 bytes (exactly 16 u32s), host-visible
        // and owned by this test — no GPU or CPU aliasing of the written range.
        unsafe {
            for (i, value) in expected.iter().enumerate() {
                *values.add(i) = *value;
            }
        }
        let mut expected_bytes = [0u8; 64];
        for (i, value) in expected.iter().enumerate() {
            expected_bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut readback = [0u8; 64];
        alloc.read_bytes(&mut readback).expect("readback");
        assert_eq!(readback, expected_bytes, "{residency:?} roundtrip");
    }
}

#[test]
fn test_gpu_only_rejects_host_access() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let alloc = GpuAllocation::new(&device, 4096, MemoryResidency::GpuOnly).expect("allocation");
    assert!(alloc.host().is_none());
    let mut out = [0u8; 64];
    let err = alloc
        .read_bytes(&mut out)
        .expect_err("device-local readback must fail");
    assert!(matches!(err, Error::Validation(_)), "got {err:?}");
}

#[test]
fn test_read_bytes_range_validation() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let alloc = GpuAllocation::new(&device, 64, MemoryResidency::CpuWriteOnly).expect("allocation");
    let mut out = [0u8; 128];
    let err = alloc
        .read_bytes(&mut out)
        .expect_err("overlong readback must fail");
    assert!(matches!(err, Error::Validation(_)), "got {err:?}");
}

#[test]
fn test_new_aligned_co_alignment() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let alloc = GpuAllocation::new_aligned(&device, 4096, MemoryResidency::CpuWriteOnly, 256)
        .expect("aligned allocation");
    assert_eq!(
        alloc.gpu_ptr().as_raw() % 256,
        0,
        "GPU base address must satisfy the requested alignment"
    );
    let host = alloc.host().expect("host-visible allocation");
    assert_eq!(
        host.typed::<u8>() as usize % 256,
        0,
        "host base address must satisfy the requested alignment"
    );
}
