//! GPU bump allocator over shared-storage blocks.
//!
//! Unified memory makes the bump trivial: every block is one shared
//! `MTLBuffer`, and a [`BumpAlloc`] is a CPU/GPU pointer pair into it —
//! writes through the CPU view are immediately visible to shaders through
//! the GPU view, with no upload pass.

use moonfield_math::gpu::align_up;

use crate::device::Device;
use crate::memory::{GpuPtr, HostPtr, Memory};
use moonfield_rhi_core::Result;

const MIN_ALIGN: usize = 16;

/// One bump allocation: CPU view (write through [`HostPtr::typed`]) and GPU
/// view (the device address of the same bytes).
pub struct BumpAlloc {
    /// CPU view — write upload data through [`HostPtr::typed`].
    pub cpu: HostPtr,
    /// GPU view — the device address of the same bytes.
    pub gpu: GpuPtr,
}

/// A frame-scope bump allocator: grow-on-demand shared blocks, bumped from
/// the front, freed all at once (`free_all`), matching the Vulkan backend's
/// `GpuBumpAllocator` surface.
pub struct GpuBumpAllocator {
    device: Device,
    block_size: u64,
    blocks: Vec<Memory>,
    /// Bump cursor into the last block.
    cursor: u64,
}

impl GpuBumpAllocator {
    /// Create an empty allocator whose blocks hold `block_size` bytes.
    pub fn new(device: &Device, block_size: u64) -> Result<Self> {
        Ok(Self {
            device: device.clone(),
            block_size: block_size.max(1),
            blocks: Vec::new(),
            cursor: 0,
        })
    }

    /// Bump-allocate `bytes` aligned to `align` and return the CPU/GPU
    /// pair. Grows a block when the current one is full.
    pub fn alloc(&mut self, bytes: usize, align: usize) -> Result<BumpAlloc> {
        let align = align.max(MIN_ALIGN);
        let cursor = align_up(self.cursor as usize, align);
        let end = cursor + bytes;
        if end as u64 > self.block_size || self.blocks.is_empty() {
            self.blocks.push(Memory::new(&self.device, self.block_size));
            self.cursor = 0;
            return self.alloc(bytes, align);
        }
        let block = self.blocks.last().expect("a block exists after grow");
        self.cursor = end as u64;
        let base = block.gpu_ptr().as_raw();
        let host: *mut u8 = block.host_ptr().wrapping_add(cursor);
        Ok(BumpAlloc {
            cpu: HostPtr::new(host),
            gpu: GpuPtr::new(base + cursor as u64),
        })
    }

    /// Bump-allocate `count` elements of `T`.
    pub fn alloc_typed<T>(&mut self, count: usize) -> Result<BumpAlloc> {
        self.alloc(count * std::mem::size_of::<T>(), std::mem::align_of::<T>())
    }

    /// Reset the cursor; blocks stay allocated for reuse.
    pub fn free_all(&mut self) {
        self.cursor = 0;
    }

    /// Number of allocated blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
}
