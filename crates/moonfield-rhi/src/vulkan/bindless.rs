//! Bindless GPU pointer primitives for the Lunar Mare render foundation.
//!
//! The bindless model replaces retained-mode descriptor binding: shader root
//! data is a single GPU pointer per stage. [`GpuPtr`] values are device
//! addresses storable in GPU-side structs; [`HostPtr`] is the CPU view of the
//! same allocation for direct writes. [`Memory`] splits allocations into
//! CPU-writable default memory, GPU-private memory, and CPU read-back.

use std::sync::{Arc, Mutex};

use ash::vk;
use gpu_allocator::{
    vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator},
    MemoryLocation,
};

use crate::{device::Device, shader_module::ShaderModule, Error, Result};

/// GPU memory classes for bindless allocations.
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
    #[allow(dead_code)] // internal accessor for the allocator layer
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
/// buffer and allocator chunk; dropping it destroys the buffer and returns
/// the memory to the pool.
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
}

impl GpuAllocation {
    /// Allocate `size` bytes in the given memory class and return the paired
    /// CPU/GPU views.
    ///
    /// A Vulkan buffer object must exist to settle either of the two queries
    /// this allocation relies on — `vkGetBufferMemoryRequirements` and
    /// `vkGetBufferDeviceAddress` — since Vulkan has no object-less BDA or
    /// requirement queries. The buffer exists only as that address carrier;
    /// it carries no fixed usage, and consumers use the returned pointers
    /// directly.
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
        Self::from_resources(device.raw(), device.allocator(), size, memory, align)
    }

    /// Resource-level constructor for long-lived owners that keep their own
    /// device handle and allocator (e.g. the bump arena): the lifetime-free
    /// form of [`new_aligned`]. Callers must keep the allocator alive for the
    /// allocation's whole lifetime.
    pub(crate) fn from_resources(
        device: &ash::Device,
        allocator: &Arc<Mutex<Allocator>>,
        size: u64,
        memory: Memory,
        align: u64,
    ) -> Result<Self> {
        // The buffer is a pure address carrier: Vulkan settles buffer device
        // addresses and memory requirements on an existing buffer object, so
        // one must exist before either can be queried. It is not bound to any
        // fixed usage; consumers fetch the address and dereference it from
        // shaders. Exclusive sharing keeps the buffer on one queue family.
        let buffer = unsafe {
            device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size)
                        .usage(
                            vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                                | vk::BufferUsageFlags::TRANSFER_SRC
                                | vk::BufferUsageFlags::TRANSFER_DST,
                        )
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

    pub fn buffer(&self) -> vk::Buffer {
        self.buffer
    }
}

impl Drop for GpuAllocation {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }

        if let Some(allocation) = self.allocation.take() {
            if let Err(e) = self
                .allocator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .free(allocation)
            {
                moonfield_log::error!("failed to free bindless allocation: {e}");
            }
        }
    }
}

/// Which class of GPU work a queue serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    /// Graphics-capable queue (also accepts compute and transfer work).
    Graphics,
    /// Dedicated async-compute queue when the device has one, otherwise the
    /// graphics queue is used for compute work too.
    Compute,
}

/// A GPU pipeline stage mask for bindless barriers.
///
/// Bindless synchronization is stage-to-stage: the barrier orders the end of
/// a producer stage against the start of a consumer stage, without naming any
/// resource — shaders address memory indirectly through pointers, so a
/// resource list would be both impossible and meaningless. The access mask is
/// the widest possible read/write, matching the pointer model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stage(pub(crate) vk::PipelineStageFlags2);

impl Stage {
    /// Vertex shader stage
    pub const VERTEX: Self = Self(vk::PipelineStageFlags2::VERTEX_SHADER);
    /// Fragment shader stage
    pub const FRAGMENT: Self = Self(vk::PipelineStageFlags2::FRAGMENT_SHADER);
    /// Compute shader stage (dispatch).
    pub const COMPUTE: Self = Self(vk::PipelineStageFlags2::COMPUTE_SHADER);
    /// Transfer stage (buffer/image copy).
    pub const TRANSFER: Self = Self(vk::PipelineStageFlags2::TRANSFER);
    /// All stages; implies the widest dependency and ignores access masks.
    pub const ALL: Self = Self(vk::PipelineStageFlags2::ALL_COMMANDS);

    pub(crate) fn to_vk(self) -> vk::PipelineStageFlags2 {
        self.0
    }
}

impl std::ops::BitOr for Stage {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// What kind of hazard a barrier orders — the blog's barrier flags. A plain
/// memory hazard covers pointer-accessed data; a descriptor hazard additionally
/// exposes the descriptor read the next stage performs through non-uniform
/// heap indices (a sampled image read).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarrierHazard {
    /// Plain memory read/write hazard (current behavior).
    #[default]
    Memory,
    /// Descriptor-heap hazard: a stage (or the CPU, through the host mapping)
    /// just wrote heap descriptors that the next stage samples.
    Descriptors,
}

/// A Vulkan compute pipeline for the bindless model.
///
/// Root data is a single push-constant range holding the entry point's GPU
/// pointers (two 64-bit addresses: input @ offset 0, output @ offset 8),
/// matching the Slang `EntryPointParams_std430` layout of a kernel with two
/// pointer parameters.
pub struct ComputePipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    device: ash::Device,
}

impl ComputePipeline {
    /// Create a compute pipeline from a compiled compute shader module.
    pub fn new(device: &Device, shader: &ShaderModule) -> Result<Self> {
        let entry = std::ffi::CString::new("main").unwrap();
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(shader.raw())
            .name(&entry);
        // Match EntryPointParams_std430: struct { input@0, output@8 }.
        let push_constant_range = vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .offset(0)
            .size(16);
        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(std::slice::from_ref(&push_constant_range));
        let layout = unsafe { device.raw().create_pipeline_layout(&layout_info, None) }
            .map_err(|e| Error::Backend(format!("failed to create pipeline layout: {:?}", e)))?;
        let pipeline_info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);
        let pipelines = unsafe {
            device.raw().create_compute_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&pipeline_info),
                None,
            )
        }
        .map_err(|e| Error::Backend(format!("failed to create compute pipeline: {:?}", e)))?;
        Ok(Self {
            pipeline: pipelines[0],
            layout,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Pipeline` handle.
    pub fn raw(&self) -> vk::Pipeline {
        self.pipeline
    }

    /// Access the raw `vk::PipelineLayout` handle.
    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }
}

impl Drop for ComputePipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}
