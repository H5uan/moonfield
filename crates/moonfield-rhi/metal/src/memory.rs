//! GPU memory: shared-storage buffers with device addresses.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLBuffer, MTLDevice, MTLResourceOptions};

use crate::device::Device;

/// A host pointer into CPU-visible GPU memory (shared storage): one
/// allocation, two views — this CPU view and the [`GpuPtr`] GPU view of the
/// same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPtr {
    ptr: *mut u8,
}

impl HostPtr {
    /// Wrap a host pointer into shared GPU memory.
    pub(crate) fn new(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Get the pointer reinterpreted for a given CPU type.
    pub fn typed<T>(&self) -> *mut T {
        self.ptr.cast()
    }
}

// Safety: a `HostPtr` is only created for an allocation that remains valid
// for the pointer's whole lifetime, and the allocation's bytes are owned by
// that one pointer — no other thread can write them. Sharing a `&HostPtr`
// across threads is read-only, so `Sync` holds under the same
// single-writer contract.
unsafe impl Send for HostPtr {}
unsafe impl Sync for HostPtr {}

/// A GPU device address, in bytes (the Metal counterpart of the Vulkan
/// backend's buffer-device-address carrier).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuPtr(u64);

impl GpuPtr {
    /// Wrap a raw device address.
    pub(crate) fn new(address: u64) -> Self {
        Self(address)
    }

    /// The raw device address value.
    pub fn as_raw(self) -> u64 {
        self.0
    }

    /// Offset the pointer by `bytes` GPU bytes.
    ///
    /// The caller must keep the result inside the same allocation.
    pub fn offset(self, bytes: u64) -> Self {
        Self(self.0 + bytes)
    }
}

/// A buffer-range allocation: one shared-storage `MTLBuffer` plus its size.
#[derive(Clone)]
pub struct GpuAllocation {
    memory: Memory,
    offset: u64,
    size: u64,
}

impl GpuAllocation {
    /// The allocation's device address.
    pub fn gpu_ptr(&self) -> GpuPtr {
        self.memory.gpu_ptr().offset(self.offset)
    }

    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }
}

/// A CPU- and GPU-visible buffer (shared storage, unified memory). ObjC
/// reference counting owns the lifetime; the buffer outliving its `Device`
/// handle is safe by construction.
#[derive(Clone)]
pub struct Memory {
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    size: u64,
}

impl Memory {
    /// Create a shared-storage buffer of `size` bytes and register it with
    /// the device-wide residency set.
    pub fn new(device: &Device, size: u64) -> Self {
        let shared = device.ctx().shared();
        let buffer = shared
            .device()
            .newBufferWithLength_options(size as usize, MTLResourceOptions::StorageModeShared)
            .expect("newBufferWithLength failed");
        shared.register_allocation(buffer.as_ref());
        Self { buffer, size }
    }

    /// The buffer's base device address.
    pub fn gpu_ptr(&self) -> GpuPtr {
        GpuPtr(self.buffer.gpuAddress())
    }

    /// A raw host pointer to the buffer's bytes (shared storage — the same
    /// bytes the GPU sees through [`gpu_ptr`](Self::gpu_ptr)).
    pub(crate) fn host_ptr(&self) -> *mut u8 {
        self.buffer.contents().as_ptr().cast()
    }

    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// A whole-buffer allocation.
    pub fn allocation(&self) -> GpuAllocation {
        GpuAllocation {
            memory: self.clone(),
            offset: 0,
            size: self.size,
        }
    }

    /// Copy `bytes` into the buffer starting at `offset`.
    ///
    /// The caller must not write while the GPU reads the same range; the
    /// RHI's single-threaded frame model makes that the frame-loop's job.
    pub fn copy_host(&self, offset: u64, bytes: &[u8]) {
        assert!(offset + bytes.len() as u64 <= self.size);
        let contents = self.buffer.contents();
        // SAFETY: `contents` points at `size` writable bytes (shared
        // storage), the range is bounds-checked above, and there is no
        // concurrent GPU access under the frame model.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                contents.as_ptr().add(offset as usize).cast(),
                bytes.len(),
            );
        }
    }
}
