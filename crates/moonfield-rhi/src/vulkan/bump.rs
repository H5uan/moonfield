//! Linear (bump) GPU allocation from reusable host-visible blocks.
//!
//! Allocations are carved
//! sequentially from blocks of `Memory::Default` (CpuToGpu) memory; on
//! overflow a new block is grown; `free_all` resets to the first block so
//! blocks are reused wholesale once the GPU has finished reading them. Each
//! [`BumpAlloc`] carries both views over the same bytes — the CPU pointer
//! (memcpy source) and the buffer device address (copy source) — with the
//! owning buffer handle, so the translation from allocation to (cpu, gpu,
//! vk::Buffer) happens exactly once per block.
use crate::bindless::{GpuAllocation, GpuPtr, HostPtr, Memory};
use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use ash::vk;
use gpu_allocator::vulkan::Allocator;
use moonfield_math::gpu::align_up;
use std::sync::{Arc, Mutex};

const MIN_ALIGN: usize = 16;

pub struct BumpAlloc {
    /// CPU view — write upload data through [`HostPtr::typed`].
    pub cpu: HostPtr,
    /// GPU view — the buffer device address of the same bytes.
    pub gpu: GpuPtr,
    /// The block buffer backing this region (copy source).
    #[allow(dead_code)] // consumed by the FrameUploader's cmd_copy_buffer (next step)
    pub(crate) src: vk::Buffer,
    /// Offset into `src` (copy source offset).
    #[allow(dead_code)] // consumed by the FrameUploader's cmd_copy_buffer (next step)
    pub(crate) src_offset: u64,
}

/// One grown block plus the base alignment it was created with. Sub-allocation
/// requests whose alignment exceeds the block's base alignment must move to
/// another block (or grow one) — see `alloc_in_block`.
struct Block {
    alloc: GpuAllocation,
    align: usize,
}

pub struct GpuBumpAllocator {
    // Owned resources pulled from `Device` at construction, so the allocator
    // needs no lifetime: long-lived owners (the frame uploader, ECS
    // resources) can store it directly.
    device: ash::Device,
    allocator: Arc<Mutex<Allocator>>,
    blocks: Vec<Block>, // 每块独立 buffer + 一次地址翻译 + 底座对齐
    block_size: u64,
    block_idx: usize,
    offset: usize,
}

impl GpuBumpAllocator {
    pub fn new(device: &Device, block_size: u64) -> Result<Self> {
        let align = MIN_ALIGN;
        let first = Block {
            alloc: GpuAllocation::from_resources(
                device.raw(),
                device.allocator(),
                block_size,
                Memory::Default,
                align as u64,
            )?,
            align,
        };
        Self::check_co_align(&first.alloc, align)?;
        Ok(Self {
            device: device.raw().clone(),
            allocator: device.allocator().clone(),
            blocks: vec![first],
            block_size,
            block_idx: 0,
            offset: 0,
        })
    }

    /// The reference implementation's panic condition, surfaced as an error:
    /// the CPU and GPU base pointers of a block must be congruent modulo
    /// `align`, otherwise a single offset can never align both views of a
    /// sub-allocation. Raising the base alignment at allocation time
    /// (`GpuAllocation::new_aligned`) makes this hold as a rule rather than
    /// by luck; the check keeps a driver/allocator regression loud.
    fn check_co_align(alloc: &GpuAllocation, align: usize) -> Result<()> {
        let cpu = alloc
            .host()
            .expect("CpuToGpu block has a CPU view")
            .as_ptr() as usize;
        let gpu = alloc.gpu().as_raw() as usize;
        if (gpu as isize - cpu as isize).rem_euclid(align as isize) != 0 {
            return Err(Error::Backend(format!(
                "bump allocator block bases cannot co-align at {align}B (cpu {cpu:#x}, gpu {gpu:#x})"
            )));
        }
        Ok(())
    }

    pub fn alloc(&mut self, bytes: usize, align: usize) -> Result<BumpAlloc> {
        if bytes == 0 {
            return Err(Error::Validation("zero-size bump allocation".into()));
        }

        let align = align.max(MIN_ALIGN);
        debug_assert!(align.is_power_of_two(), "align must be a power of two");
        self.alloc_in_block(bytes, align)
    }
    pub fn alloc_typed<T>(&mut self, count: usize) -> Result<BumpAlloc> {
        self.alloc(std::mem::size_of::<T>() * count, std::mem::align_of::<T>())
    }

    pub fn free_all(&mut self) {
        self.block_idx = 0;
        self.offset = 0;
    }

    /// Number of blocks grown so far (each block is a separate buffer +
    /// allocation). Diagnostics/sizing aid for callers that want to observe
    /// growth beyond the initial block.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    fn alloc_in_block(&mut self, bytes: usize, align: usize) -> Result<BumpAlloc> {
        // 当前块底座对齐不够 → 换一块底座够的（参考项目此处直接 panic，
        // 我们走 grow，行为更稳）。
        if align > self.blocks[self.block_idx].align {
            self.next_block(bytes, align)?;
            return self.alloc_in_block(bytes, align);
        }
        let block = &self.blocks[self.block_idx];
        let gpu_addr = block.alloc.gpu().as_raw() as usize + self.offset;

        let aligned = align_up(gpu_addr, align);
        let offset = aligned - block.alloc.gpu().as_raw() as usize;
        if offset + bytes > block.alloc.size() as usize {
            self.next_block(bytes, align)?; // 之后 offset == 0
            return self.alloc_in_block(bytes, align);
        }
        self.offset = offset + bytes;
        let host = block
            .alloc
            .host()
            .expect("CpuToGpu block has a CPU view")
            .offset(offset);
        Ok(BumpAlloc {
            cpu: host,
            gpu: block.alloc.gpu().offset(offset as u64),
            src: block.alloc.buffer(),
            src_offset: offset as u64,
        })
    }

    fn next_block(&mut self, bytes: usize, align: usize) -> Result<()> {
        self.block_idx += 1;
        self.offset = 0;
        let size = self.block_size.max(bytes as u64);
        match self.block_idx.cmp(&self.blocks.len()) {
            std::cmp::Ordering::Less => {
                let slot = self.block_idx;
                // 复用条件：size 和底座对齐都够，否则静默复用旧块会让本块
                // 服务它撑不起的对齐请求（参考项目靠 panic 兜底，我们换新块）。
                if self.blocks[slot].alloc.size() < bytes as u64 || self.blocks[slot].align < align
                {
                    let alloc = GpuAllocation::from_resources(
                        &self.device,
                        &self.allocator,
                        size,
                        Memory::Default,
                        align as u64,
                    )?;
                    Self::check_co_align(&alloc, align)?;
                    self.blocks[slot] = Block { alloc, align };
                }
            }
            std::cmp::Ordering::Equal => {
                // 所有已有块都被用过且放不下，追加一块新的（最常见增长路径）。
                let alloc = GpuAllocation::from_resources(
                    &self.device,
                    &self.allocator,
                    size,
                    Memory::Default,
                    align as u64,
                )?;
                Self::check_co_align(&alloc, align)?;
                self.blocks.push(Block { alloc, align });
            }
            std::cmp::Ordering::Greater => unreachable!("block_idx advances by one"),
        }
        Ok(())
    }
}
