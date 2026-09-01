//! Bindless 2.0 descriptor heap: CPU-visible texture and sampler slots.
//!
//! [`DescriptorHeap`] implements the `VK_EXT_descriptor_heap` model: texture
//! descriptors live in a host-visible GPU buffer that the CPU writes straight
//! into (`write_resource_descriptors` / `write_samplers`) and that the command
//! buffer binds by device address (`cmd_bind_resource_heap` /
//! `cmd_bind_sampler_heap`) — the blog's "no update-API abstraction" model.
//!
//! The shader sees two descriptor arrays, `binding 0` as `SAMPLED_IMAGE` and
//! `binding 1` as `SAMPLER`, both indexed with non-uniform 32-bit handles:
//! [`TextureHandle`] and [`SamplerHandle`].
//!
//! Slot semantics follow the bump-arena contract: freeing a slot invalidates
//! it, and a re-allocated slot must be written again before it is referenced —
//! the next frame's write, not slot zeroing, is what makes the new contents
//! visible.

use crate::bindless::{GpuAllocation, Memory};
use crate::error::{Error, Result};
use crate::types::{Filter, SamplerDesc};
use crate::vulkan::device::{DescriptorHeapProperties, Device};
use crate::CommandBuffer;
use ash::vk;
use moonfield_math::gpu::align_up;
use std::sync::Mutex;

/// Default descriptor heap capacities, matching the bindless texture budget.
pub const DESCRIPTOR_HEAP_IMAGE_CAPACITY: u32 = 4096;
pub const DESCRIPTOR_HEAP_SAMPLER_CAPACITY: u32 = 1024;

/// A slot index in the image descriptor array (`binding 0`), the handle
/// shaders store in root data and index with `NonUniformResourceIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureHandle(pub u32);

/// A slot index in the sampler descriptor array (`binding 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SamplerHandle(pub u32);

/// The description of one texture slot.
///
/// The heap encodes a view's *create info* directly into heap memory
/// (`ImageDescriptorInfoEXT.p_view` is a create info pointer, so it must
/// outlive the slot — the owning `Texture` guarantees that). `layout` is the
/// image layout the sample sees; the upload path leaves images in `GENERAL`.
pub struct TextureSlotDesc<'a> {
    pub view_create_info: &'a vk::ImageViewCreateInfo<'a>,
    pub layout: vk::ImageLayout,
}

/// Bump-counter + freelist slot allocator for one descriptor array.
struct SlotAllocator {
    next: u32,
    free: Vec<u32>,
    capacity: u32,
}

impl SlotAllocator {
    fn new(capacity: u32) -> Self {
        Self {
            next: 0,
            free: Vec::new(),
            capacity,
        }
    }

    fn alloc(&mut self) -> Result<u32> {
        if let Some(slot) = self.free.pop() {
            return Ok(slot);
        }
        if self.next < self.capacity {
            let slot = self.next;
            self.next += 1;
            Ok(slot)
        } else {
            Err(Error::Validation(format!(
                "descriptor heap is full ({} slots)",
                self.capacity
            )))
        }
    }

    fn free(&mut self, slot: u32) -> Result<()> {
        if slot >= self.next {
            return Err(Error::Validation(format!(
                "freeing slot {slot} that was never allocated"
            )));
        }
        if self.free.contains(&slot) {
            return Err(Error::Validation(format!("slot {slot} freed twice")));
        }
        self.free.push(slot);
        Ok(())
    }
}

/// Bindless 2.0 texture descriptor heap.
///
/// Owns the two heap buffers (resources and samplers), their slot allocators,
/// and the `VK_EXT_descriptor_heap` device entry points. All methods take
/// `&self`; the internal state is mutex-guarded so the heap can live behind an
/// `Arc` next to an uploader (the `Texture::new` pattern).
pub struct DescriptorHeap {
    image_slots: Mutex<SlotAllocator>,
    sampler_slots: Mutex<SlotAllocator>,
    resource_heap: GpuAllocation,
    sampler_heap: GpuAllocation,
    image_stride: usize,
    sampler_stride: usize,
    image_heap_size: u64,
    sampler_heap_size: u64,
    min_resource_reserved_range: u64,
    min_sampler_reserved_range: u64,
    ext: ash::ext::descriptor_heap::Device,
}

impl DescriptorHeap {
    /// Create a heap with `image_capacity` texture slots and
    /// `sampler_capacity` sampler slots on the device's descriptor heap.
    ///
    /// The RHI requires the `VK_EXT_descriptor_heap` CPU-visible heap
    /// unconditionally — device creation fails without it — so the limits
    /// needed to size these heaps are always present here.
    pub fn new(device: &Device, image_capacity: u32, sampler_capacity: u32) -> Result<Self> {
        // The RHI requires the CPU-visible descriptor heap unconditionally:
        // `Device::descriptor_heap_properties` always carries the limits.
        let props = device.descriptor_heap_properties();
        let (resource_heap, image_stride, image_heap_size, image_cap) =
            Self::heap_buffer(device, image_capacity, &props, HeapKind::Resource)?;
        let (sampler_heap, sampler_stride, sampler_heap_size, sampler_cap) =
            Self::heap_buffer(device, sampler_capacity, &props, HeapKind::Sampler)?;
        Ok(Self {
            image_slots: Mutex::new(SlotAllocator::new(image_cap)),
            sampler_slots: Mutex::new(SlotAllocator::new(sampler_cap)),
            resource_heap,
            sampler_heap,
            image_stride,
            sampler_stride,
            image_heap_size,
            sampler_heap_size,
            min_resource_reserved_range: props.min_resource_heap_reserved_range,
            min_sampler_reserved_range: props.min_sampler_heap_reserved_range,
            ext: device.extension_fns().descriptor_heap.clone(),
        })
    }

    /// Size one heap buffer from the driver properties: slot stride is the
    /// descriptor size rounded up to its alignment, the heap total is clamped
    /// to the driver's cap, and the effective capacity reports how many slots
    /// actually fit (the driver may cap below the request).
    fn heap_buffer(
        device: &Device,
        capacity: u32,
        props: &DescriptorHeapProperties,
        kind: HeapKind,
    ) -> Result<(GpuAllocation, usize, u64, u32)> {
        let (size, alignment, heap_size, heap_align) = match kind {
            HeapKind::Resource => (
                props.image_descriptor_size,
                props.image_descriptor_alignment,
                props.max_resource_heap_size,
                props.resource_heap_alignment,
            ),
            HeapKind::Sampler => (
                props.sampler_descriptor_size,
                props.sampler_descriptor_alignment,
                props.max_sampler_heap_size,
                props.sampler_heap_alignment,
            ),
        };
        let stride = size.max(alignment) as usize;
        let total =
            align_up(capacity as usize * stride, heap_align as usize).min(heap_size as usize);
        let effective = (total / stride) as u32;
        let allocation =
            GpuAllocation::new_aligned(device, total as u64, Memory::Default, heap_align)?;
        Ok((allocation, stride, total as u64, effective))
    }

    /// Allocate a texture slot. The slot must be written with
    /// [`write_resource_descriptors`](Self::write_resource_descriptors) before
    /// any GPU work references its handle.
    pub fn alloc_image_slot(&self) -> Result<TextureHandle> {
        Ok(TextureHandle(
            self.image_slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .alloc()?,
        ))
    }

    /// Return a texture slot to the pool. In-flight frames must not reference
    /// the handle afterwards; re-allocation hands it back for a new write.
    pub fn free_image_slot(&self, handle: TextureHandle) -> Result<()> {
        self.image_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .free(handle.0)
    }

    /// Allocate a sampler slot (see [`alloc_image_slot`](Self::alloc_image_slot)).
    pub fn alloc_sampler_slot(&self) -> Result<SamplerHandle> {
        Ok(SamplerHandle(
            self.sampler_slots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .alloc()?,
        ))
    }

    /// Return a sampler slot to the pool (see
    /// [`free_image_slot`](Self::free_image_slot)).
    pub fn free_sampler_slot(&self, handle: SamplerHandle) -> Result<()> {
        self.sampler_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .free(handle.0)
    }

    /// Write texture descriptors into their slots.
    ///
    /// `view_create_info` in each description must outlive the slot — the heap
    /// keeps it in heap memory by pointer. This is a direct host write into
    /// the resource heap's mapping, not an `vkUpdateDescriptorSets`.
    pub fn write_resource_descriptors(
        &self,
        writes: &[(TextureHandle, TextureSlotDesc<'_>)],
    ) -> Result<()> {
        let mut image_descs: Vec<vk::ImageDescriptorInfoEXT<'_>> = Vec::with_capacity(writes.len());
        for (_, desc) in writes {
            image_descs.push(
                vk::ImageDescriptorInfoEXT::default()
                    .view(desc.view_create_info)
                    .layout(desc.layout),
            );
        }
        let mut resources: Vec<vk::ResourceDescriptorInfoEXT<'_>> =
            Vec::with_capacity(writes.len());
        for (i, _) in writes.iter().enumerate() {
            resources.push(
                vk::ResourceDescriptorInfoEXT::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .data(vk::ResourceDescriptorDataEXT {
                        p_image: &image_descs[i],
                    }),
            );
        }
        let host = self
            .resource_heap
            .host()
            .ok_or_else(|| Error::Validation("resource heap is not host-visible".into()))?;
        let mut ranges: Vec<vk::HostAddressRangeEXT<'_>> = Vec::with_capacity(writes.len());
        for (handle, _) in writes {
            let address = host
                .offset(handle.0 as usize * self.image_stride)
                .as_ptr()
                .cast();
            ranges.push(vk::HostAddressRangeEXT {
                address,
                size: self.image_stride,
                _marker: std::marker::PhantomData,
            });
        }
        // SAFETY: every range points into the heap's host mapping and stays
        // alive for the call; the descriptor data encodes valid views.
        unsafe {
            self.ext.write_resource_descriptors(&resources, &ranges)?;
        }
        Ok(())
    }

    /// Write sampler descriptors into their slots.
    ///
    /// The heap stores a driver-encoded sampler per slot; no `vk::Sampler`
    /// object is ever created.
    pub fn write_samplers(&self, writes: &[(SamplerHandle, SamplerDesc)]) -> Result<()> {
        let mut create_infos: Vec<vk::SamplerCreateInfo<'_>> = Vec::with_capacity(writes.len());
        for (_, desc) in writes {
            create_infos.push(sampler_create_info(desc));
        }
        let host = self
            .sampler_heap
            .host()
            .ok_or_else(|| Error::Validation("sampler heap is not host-visible".into()))?;
        let mut ranges: Vec<vk::HostAddressRangeEXT<'_>> = Vec::with_capacity(writes.len());
        for (handle, _) in writes {
            let address = host
                .offset(handle.0 as usize * self.sampler_stride)
                .as_ptr()
                .cast();
            ranges.push(vk::HostAddressRangeEXT {
                address,
                size: self.sampler_stride,
                _marker: std::marker::PhantomData,
            });
        }
        // SAFETY: sampler create infos are plain data the driver encodes into
        // the heap mapping.
        unsafe {
            self.ext.write_sampler_descriptors(&create_infos, &ranges)?;
        }
        Ok(())
    }

    /// Bind both heaps to the command buffer: call once per frame while
    /// recording, before the draws that sample texture slots. Heap binding is
    /// bind-point agnostic — one call serves graphics and compute work.
    /// `reserved_range_size` satisfies the driver's minimum reserved range.
    pub fn cmd_bind_graphics(&self, cb: &CommandBuffer) -> Result<()> {
        let resource_bind = vk::BindHeapInfoEXT::default()
            .heap_range(vk::DeviceAddressRangeEXT {
                address: self.resource_heap.gpu().as_raw(),
                size: self.image_heap_size,
            })
            .reserved_range_offset(0)
            .reserved_range_size(self.min_resource_reserved_range);
        // SAFETY: the heap buffer is valid and not bound to anything else.
        unsafe {
            self.ext.cmd_bind_resource_heap(cb.raw(), &resource_bind);
        }
        let sampler_bind = vk::BindHeapInfoEXT::default()
            .heap_range(vk::DeviceAddressRangeEXT {
                address: self.sampler_heap.gpu().as_raw(),
                size: self.sampler_heap_size,
            })
            .reserved_range_offset(0)
            .reserved_range_size(self.min_sampler_reserved_range);
        // SAFETY: as above for the sampler heap.
        unsafe {
            self.ext.cmd_bind_sampler_heap(cb.raw(), &sampler_bind);
        }
        Ok(())
    }
}

enum HeapKind {
    Resource,
    Sampler,
}

/// Translate the crate's [`SamplerDesc`] into a Vulkan create info. Mirrors
/// the single-mip model of the upload path (`max_lod` 0 samples mip 0 only).
fn sampler_create_info(desc: &SamplerDesc) -> vk::SamplerCreateInfo<'_> {
    vk::SamplerCreateInfo::default()
        .mag_filter(desc.mag_filter.to_vk())
        .min_filter(desc.min_filter.to_vk())
        .mipmap_mode(match desc.mipmap_filter {
            Some(Filter::Linear) => vk::SamplerMipmapMode::LINEAR,
            _ => vk::SamplerMipmapMode::NEAREST,
        })
        .address_mode_u(desc.wrap.to_vk())
        .address_mode_v(desc.wrap.to_vk())
        .address_mode_w(desc.wrap.to_vk())
        .max_lod(0.0)
}
