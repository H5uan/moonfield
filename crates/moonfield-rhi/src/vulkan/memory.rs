//! GPU memory and pointer primitives — the RHI's memory model.
//!
//! [`GpuPtr`] values are device addresses storable in GPU-side structs;
//! [`HostPtr`] is the CPU view of the same allocation for direct writes.
//! [`GpuAllocation`] carries both views over the same bytes — CPU view +
//! device address in one object. [`Memory`] splits allocations into
//! CPU-writable default memory, GPU-private memory, and CPU read-back.

use std::sync::{Arc, Mutex};

use ash::vk;
use gpu_allocator::{
    MemoryLocation,
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator},
};

use crate::{
    Error, Result,
    retire::{RetireAction, RetirementRing},
};

/// GPU memory classes for allocations.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Memory {
    /// GPU-visible memory with a CPU-mapped pointer, optimized for CPU
    /// writes and GPU reads.
    ///
    /// This is the default memory type for bindless resources: the CPU
    /// writes directly into the mapping and the GPU reads it. CPU reads are
    /// possible but not optimized.
    #[default]
    Default,
    /// GPU-only memory not visible to the CPU.
    Gpu,
    /// GPU-visible memory mapped for CPU read-back.
    ///
    /// The GPU writes, the CPU reads. CPU writes are possible but not
    /// optimized.
    Readback,
}

impl Memory {
    /// Map the bindless memory class to the allocator's location.
    ///
    /// `gpu-allocator` picks the physical heap per location, preferring
    /// device-local memory on UMA or ReBAR systems and falling back to
    /// host-visible memory on discrete GPUs.
    pub(crate) fn to_location(self) -> MemoryLocation {
        match self {
            Memory::Default => MemoryLocation::CpuToGpu,
            Memory::Gpu => MemoryLocation::GpuOnly,
            Memory::Readback => MemoryLocation::GpuToCpu,
        }
    }
}

/// A GPU-visible memory pointer: the buffer device address usable in shaders
/// and storable in any bindless data structure.
///
/// The address is a plain value, not a handle: it can be stored inside other
/// GPU-side structs, passed as an argument, and adjusted with [`GpuPtr::offset`]
/// — the blog's "GPU pointer arithmetic" model, aligned to `u64` storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuPtr(u64);
impl GpuPtr {
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
    /// Construct from a raw device address.
    ///
    /// Crate-internal: callers receive addresses from the allocator layer,
    /// never construct a `GpuPtr` by hand.
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// A CPU-side pointer returned alongside a [`GpuPtr`] allocation, when the
/// allocation is CPU-visible.
///
/// The pointer is a value: it can be dereferenced on the CPU without an
/// intermediate handle lookup and coexists with the GPU-side view of the same
/// memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostPtr(pub(crate) *mut u8);
impl HostPtr {
    /// The raw byte pointer.
    ///
    /// Crate-internal: allocator code writes through this; consumers use
    /// [`HostPtr::typed`].
    pub(crate) fn as_ptr(&self) -> *mut u8 {
        self.0
    }

    /// Get the pointer reinterpreted for a given CPU type.
    pub fn typed<T>(&self) -> *mut T {
        self.0.cast()
    }

    /// Advance the pointer by `bytes` host bytes — the same offset applied to
    /// the GPU view, so the CPU/GPU pair stays in lockstep. Callers must keep
    /// the result inside the same allocation.
    pub(crate) fn offset(self, bytes: usize) -> Self {
        Self(self.0.wrapping_add(bytes))
    }
}

// Safety: a `HostPtr` is only created for an allocation that remains valid for
// the pointer's whole lifetime, and the allocation's bytes are owned by that
// one pointer — no other thread can write them. Nothing here is synchronized
// against a second write through the same pointer; callers must use
// per-frame allocation schemes that avoid aliased writes. Sharing a
// `&HostPtr` across threads is read-only, so `Sync` holds under the same
// single-writer contract (needed for the uploader to live in an ECS
// resource).
unsafe impl Send for HostPtr {}
unsafe impl Sync for HostPtr {}

/// A bindless GPU allocation: CPU view + device address in one object.
///
/// This is the Rust counterpart of the blog's `gpuMalloc` result — one
/// allocation, two views over the same bytes: [`HostPtr`] for direct CPU
/// writes, [`GpuPtr`] for shader access. The allocation owns its Vulkan
/// buffer and allocator chunk; dropping it defers the buffer's destruction
/// to the device's retirement ring.
pub struct GpuAllocation {
    /// Address carrier: BDA and memory requirements settle on this object.
    buffer: vk::Buffer,
    /// The underlying memory chunk; `None` after it is returned in `Drop`.
    allocation: Option<Allocation>,
    /// Requested size in bytes.
    size: u64,
    /// CPU view, present when the memory class is host-visible.
    host: Option<HostPtr>,
    /// GPU view (buffer device address), valid on all memory classes.
    gpu: GpuPtr,
    /// Device handle for teardown in `Drop`.
    device: ash::Device,
    /// Pool handle for returning the chunk in `Drop`.
    allocator: Arc<Mutex<Allocator>>,
    /// Device-level retirement ring; `Drop` enqueues the teardown here.
    ring: Arc<RetirementRing>,
}

impl GpuAllocation {
    /// Allocate `size` bytes in the given memory class and return the paired
    /// CPU/GPU views.
    ///
    /// A Vulkan buffer object must exist to settle either of the two queries
    /// this allocation relies on — `vkGetBufferMemoryRequirements` and
    /// `vkGetBufferDeviceAddress` — since Vulkan has no object-less BDA or
    /// requirement queries. The buffer exists only as that address carrier;
    /// consumers use the returned pointers directly.
    pub fn new(device: &crate::vulkan::device::Device, size: u64, memory: Memory) -> Result<Self> {
        // The allocator's default base alignment (16 bytes) satisfies every
        // standard use; arena-style carving that needs co-aligned CPU/GPU
        // sub-allocations past 16 bytes uses [`new_aligned`] instead.
        Self::new_aligned(device, size, memory, 16)
    }

    /// Like [`new`], but raises the block's base alignment to at least `align`
    /// bytes before allocating — the reference implementation's
    /// `mem_requirements.alignment = max(.., align)`. A host-visible block
    /// mapped and addressed on an `align` boundary lets one shared offset
    /// align both the CPU and GPU view of every sub-allocation up to `align`.
    pub fn new_aligned(
        device: &crate::vulkan::device::Device,
        size: u64,
        memory: Memory,
        align: u64,
    ) -> Result<Self> {
        Self::from_resources(
            device.raw(),
            device.allocator(),
            device.retirement_ring(),
            size,
            memory,
            align,
            false,
        )
    }

    /// Like [`new_aligned`], but marks the buffer as descriptor-heap backing
    /// memory (`VK_BUFFER_USAGE_DESCRIPTOR_HEAP_EXT` — required by the
    /// extension's bind commands; some drivers fault binding a heap without
    /// it).
    pub(crate) fn new_heap(
        device: &crate::vulkan::device::Device,
        size: u64,
        align: u64,
    ) -> Result<Self> {
        Self::from_resources(
            device.raw(),
            device.allocator(),
            device.retirement_ring(),
            size,
            Memory::Default,
            align,
            true,
        )
    }

    /// Resource-level constructor for long-lived owners that keep their own
    /// device handle and allocator (e.g. the bump arena): the lifetime-free
    /// form of [`new_aligned`]. Callers must keep the allocator alive for the
    /// allocation's whole lifetime.
    pub(crate) fn from_resources(
        device: &ash::Device,
        allocator: &Arc<Mutex<Allocator>>,
        ring: Arc<RetirementRing>,
        size: u64,
        memory: Memory,
        align: u64,
        descriptor_heap: bool,
    ) -> Result<Self> {
        // The buffer is a pure address carrier: Vulkan settles buffer device
        // addresses and memory requirements on an existing buffer object, so
        // one must exist before either can be queried. Its usage set covers
        // every way the bindless model touches memory — address taking,
        // transfer copies, indirect-argument reads, and descriptor-heap
        // backing — consumers fetch the address and dereference it from
        // shaders. Exclusive sharing keeps the buffer on one queue family.
        let mut usage = vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::TRANSFER_SRC
            | vk::BufferUsageFlags::TRANSFER_DST;
        if descriptor_heap {
            usage |= vk::BufferUsageFlags::DESCRIPTOR_HEAP_EXT;
        }
        let buffer = unsafe {
            device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size)
                        .usage(usage)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
                .map_err(|e| Error::Backend(format!("failed to create bindless buffer: {:?}", e)))?
        };
        // Query the size/alignment this buffer's memory must satisfy; Vulkan
        // settles this on the existing buffer object. Raise the alignment so
        // the allocator places the block on an `align` boundary, keeping the
        // CPU/GPU base-pointer delta a multiple of `align` (see
        // `GpuBumpAllocator::check_co_align`).
        let mut requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        requirements.alignment = requirements.alignment.max(align.max(16));

        let allocator = allocator.clone();
        // Carve a chunk out of the allocator's pool for the buffer.
        let allocation = allocator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name: "bindless buffer",
                requirements,
                location: memory.to_location(),
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| Error::Backend(format!("failed to allocate bindless memory: {e}")))?;

        // Attach the backing memory to the buffer at the chunk's offset.
        unsafe {
            device
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
                .map_err(|e| Error::Backend(format!("failed to bind bindless memory: {:?}", e)))?;
        }
        let address_info = vk::BufferDeviceAddressInfo::default().buffer(buffer);
        let gpu_ptr = unsafe { GpuPtr::from_raw(device.get_buffer_device_address(&address_info)) };
        let host = allocation
            .mapped_ptr()
            .map(|ptr| HostPtr(ptr.as_ptr().cast()));
        Ok(Self {
            buffer,
            allocation: Some(allocation),
            size,
            host,
            gpu: gpu_ptr,
            device: device.clone(),
            allocator,
            ring,
        })
    }

    /// The CPU view of this allocation, if the memory class is host-visible.
    ///
    /// `Some` for [`Memory::Default`] and [`Memory::Readback`], `None` for
    /// [`Memory::Gpu`]. Write through [`HostPtr::typed`].
    pub fn host(&self) -> Option<HostPtr> {
        self.host
    }
    /// The GPU address usable in shaders.
    pub fn gpu(&self) -> GpuPtr {
        self.gpu
    }
    /// Size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The raw `vk::Buffer` address carrier. Crate-internal: command recording
    /// (`cmd_memcpy`, `dispatch_indirect`) and the bump arena's copy-source
    /// handle use it; consumers work with the paired CPU/GPU pointers.
    pub(crate) fn buffer(&self) -> vk::Buffer {
        self.buffer
    }
}

impl Drop for GpuAllocation {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still dereference the
        // device address. The ring drains RETIRE_RING frames later.
        self.ring.push(RetireAction::Buffer {
            device: self.device.clone(),
            buffer: self.buffer,
            allocation: self.allocation.take(),
            allocator: self.allocator.clone(),
        });
    }
}
