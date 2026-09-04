//! Offscreen color target that can be sampled as a texture.
//!
//! Provides [`OffscreenTarget`], a renderable image used for editor viewports:
//! the scene is rendered into the image and a UI toolkit (e.g. egui) samples
//! it afterwards. The caller picks the attachment layout when beginning a
//! rendering pass; `SHADER_READ_ONLY_OPTIMAL` outside a pass keeps the image
//! sampleable with no explicit transitions. Sampling goes through the
//! descriptor heap: the target owns one image slot, and its sampler comes
//! from the heap's description cache; a resize allocates a new image slot
//! and retires the old one, so holders re-register when the handles change.
//!
//! [`OffscreenTarget::new_with_depth`] adds a `D32Sfloat` depth attachment for
//! depth-tested scene rendering (reverse-Z: the depth clear value is 0.0).

use crate::error::{Error, Result};
use crate::types::{Filter, Format, SamplerDesc, WrapMode};
use crate::vulkan::device::Device;
use crate::vulkan::retire::{RetireAction, RetirementRing};
use crate::vulkan::sync::Fence;
use crate::{CommandBuffer, CommandPool, DescriptorHeap, SamplerHandle, TextureHandle};
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};
use std::sync::Arc;

/// The target's descriptor-heap slots: the color view's image slot and the
/// linear/clamp sampler from the heap's description cache. A resize
/// allocates a fresh image slot (the old one retires through the ring);
/// the cached sampler handle is shared by every target with the same
/// description.
struct HeapSlots {
    texture: TextureHandle,
    sampler: SamplerHandle,
    heap: Arc<DescriptorHeap>,
    /// The color view's create info, owned for its *lifetime*: the heap's
    /// descriptor write encoded a pointer to it (`ImageDescriptorInfoEXT.
    /// p_view`), so it must outlive the slot. `Drop` moves it into the
    /// retirement action, which frees the slot.
    view_create_info: vk::ImageViewCreateInfo<'static>,
    /// Device-level retirement ring; `Drop` enqueues the teardown here.
    ring: Arc<RetirementRing>,
}

impl HeapSlots {
    /// Allocate the image slot, write its descriptor, and fetch the cached
    /// sampler.
    fn new(device: &Device, view_create_info: vk::ImageViewCreateInfo<'static>) -> Result<Self> {
        let heap = device.descriptor_heap();
        let texture = heap.alloc_image_slot()?;
        heap.write_resource_descriptors(&[(
            texture,
            crate::vulkan::descriptor_heap::TextureSlotDesc::new(
                &view_create_info,
                vk::ImageLayout::GENERAL,
            ),
        )])?;
        let sampler = heap.sampler_for(target_sampler_desc())?;
        Ok(Self {
            texture,
            sampler,
            heap,
            view_create_info,
            ring: device.retirement_ring(),
        })
    }
}

impl Drop for HeapSlots {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still index the image
        // slot. The action carries the view create info (the heap's
        // encoded descriptor references it by pointer). The sampler slot
        // is cached and never freed.
        self.ring.push(RetireAction::ImageSlot {
            heap: self.heap.clone(),
            handle: self.texture,
            view_create_info: self.view_create_info,
        });
    }
}

/// A renderable and sampleable offscreen color target.
///
/// Teardown is deferred through the device's retirement ring: `Drop`
/// retires the color and depth images, and the heap slots retire through
/// [`HeapSlots`]'s own `Drop`. There is no render pass or framebuffer —
/// with dynamic rendering the caller builds attachments inline via
/// [`RenderPassDesc`](crate::RenderPassDesc).
pub struct OffscreenTarget {
    heap_slots: HeapSlots,
    image_view: vk::ImageView,
    image: vk::Image,
    allocation: Option<Allocation>,
    depth_image_view: Option<vk::ImageView>,
    depth_image: Option<vk::Image>,
    depth_allocation: Option<Allocation>,
    device: ash::Device,
    allocator: std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    /// Device-level retirement ring; `Drop` enqueues the image teardown here.
    ring: Arc<RetirementRing>,
    format: Format,
    extent: vk::Extent2D,
    has_depth: bool,
}

impl OffscreenTarget {
    /// Create an offscreen target of `width`×`height` with the given color
    /// format. The image is transitioned to `SHADER_READ_ONLY_OPTIMAL` so it
    /// can be sampled before the first frame is rendered.
    pub fn new(device: &Device, width: u32, height: u32, format: Format) -> Result<Self> {
        Self::create(device, width, height, format, false)
    }

    /// Create an offscreen target with an additional `D32Sfloat` depth
    /// attachment (framebuffer attachment index 1).
    ///
    /// The render pass clears depth to 0.0 (reverse-Z: near → 1, far → 0) and
    /// leaves it in `DEPTH_STENCIL_ATTACHMENT_OPTIMAL`; pair it with a
    /// pipeline created with `depth_test: true`. A begun pass must supply two
    /// clear values: color first, then depth 0.0.
    pub fn new_with_depth(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<Self> {
        Self::create(device, width, height, format, true)
    }

    fn create(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
        with_depth: bool,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "offscreen target dimensions must be non-zero, got {}x{}",
                width, height
            )));
        }

        let format_vk = format.to_vk();
        let extent = vk::Extent2D { width, height };
        let allocator = device.allocator().clone();
        let (image, allocation) = create_color_image(device, &allocator, extent, format_vk)?;
        let (image_view, view_create_info) =
            create_image_view(device, image, format_vk, vk::ImageAspectFlags::COLOR)?;
        let (depth_image, depth_allocation, depth_image_view) = if with_depth {
            let (image, allocation) = create_depth_image(device, &allocator, extent)?;
            let view = create_image_view(
                device,
                image,
                vk::Format::D32_SFLOAT,
                vk::ImageAspectFlags::DEPTH,
            )?
            .0;
            (Some(image), Some(allocation), Some(view))
        } else {
            (None, None, None)
        };

        transition_to_shader_read(device, image)?;

        // Publish the color view and the fixed sampler to the descriptor
        // heap.
        let heap_slots = HeapSlots::new(device, view_create_info)?;

        Ok(Self {
            heap_slots,
            image_view,
            image,
            allocation: Some(allocation),
            depth_image_view,
            depth_image,
            depth_allocation,
            device: device.raw().clone(),
            allocator,
            ring: device.retirement_ring(),
            format,
            extent,
            has_depth: with_depth,
        })
    }

    /// Resize the target: allocate a new image, view, and image slot; the
    /// old one retires through the ring when the fields are replaced, so
    /// in-flight frames keep sampling valid memory. Holders re-register
    /// when [`texture_handle`](Self::texture_handle) changes. Zero
    /// dimensions are ignored (e.g. a minimized viewport panel).
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.extent.width == width && self.extent.height == height {
            return Ok(());
        }

        let extent = vk::Extent2D { width, height };
        let format_vk = self.format.to_vk();
        let (image, allocation) = create_color_image(device, &self.allocator, extent, format_vk)?;
        let (image_view, view_create_info) =
            create_image_view(device, image, format_vk, vk::ImageAspectFlags::COLOR)?;
        let heap_slots = HeapSlots::new(device, view_create_info)?;
        let (depth_image, depth_allocation, depth_image_view) = if self.has_depth {
            let (image, allocation) = create_depth_image(device, &self.allocator, extent)?;
            let view = create_image_view(
                device,
                image,
                vk::Format::D32_SFLOAT,
                vk::ImageAspectFlags::DEPTH,
            )?
            .0;
            (Some(image), Some(allocation), Some(view))
        } else {
            (None, None, None)
        };

        transition_to_shader_read(device, image)?;

        // Swap in the new target; the old image, views, allocations, and
        // heap slots retire through the ring.
        self.retire_images();
        self.heap_slots = heap_slots;
        self.image_view = image_view;
        self.image = image;
        self.allocation = Some(allocation);
        self.depth_image_view = depth_image_view;
        self.depth_image = depth_image;
        self.depth_allocation = depth_allocation;
        self.extent = extent;
        Ok(())
    }

    /// Whether this target has a depth attachment (see [`Self::new_with_depth`]).
    pub fn has_depth(&self) -> bool {
        self.has_depth
    }

    /// Borrow the color image view as a backend-neutral [`TextureView`], for
    /// sampling in a UI pass or as the color attachment of a
    /// [`RenderPassDesc`](crate::RenderPassDesc).
    ///
    /// The returned view borrows this target's underlying `vk::ImageView`; it
    /// does not own it and must not outlive the target.
    pub fn view(&self) -> crate::vulkan::view::TextureView {
        crate::vulkan::view::TextureView::borrow_raw(self.image_view, self.device.clone())
    }

    /// Borrow the depth image view, if present (for the depth attachment of a
    /// [`RenderPassDesc`](crate::RenderPassDesc)).
    pub fn depth_view(&self) -> Option<crate::vulkan::view::TextureView> {
        self.depth_image_view
            .map(|view| crate::vulkan::view::TextureView::borrow_raw(view, self.device.clone()))
    }

    /// The color attachment format of this target.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The color view's descriptor-heap slot, for bindless sampling (e.g.
    /// the editor's egui pass). Changed by a resize (a resize allocates new
    /// slots); holders re-register when it changes.
    pub fn texture_handle(&self) -> TextureHandle {
        self.heap_slots.texture
    }

    /// The target's fixed sampler's descriptor-heap slot (linear filtering,
    /// clamp-to-edge), from the heap's sampler cache — every target with
    /// this description shares it.
    pub fn sampler_handle(&self) -> SamplerHandle {
        self.heap_slots.sampler
    }

    /// The `(width, height)` of the target.
    pub fn extent(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    /// Copy the target's pixels into a host buffer and return them (BGRA,
    /// row-major). Debug/readback path: blocks on the graphics queue.
    pub fn read_pixels(&self, device: &Device) -> Result<Vec<u8>> {
        let (width, height) = self.extent();
        let readback = crate::Buffer::new(
            device,
            (width * height * 4) as u64,
            crate::BufferUsage::COPY_DST,
            crate::Memory::Readback,
        )?;

        let command_pool = CommandPool::new(device, device.queue_family_indices().graphics)?;
        let mut command_buffer = command_pool.allocate_command_buffer()?;
        command_buffer.begin(crate::CommandBufferUsage::ONE_TIME_SUBMIT)?;
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let to_transfer = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.pipeline_barrier(
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        // SAFETY: the target is in TRANSFER_SRC_OPTIMAL and the buffer fits it.
        unsafe {
            device.raw().cmd_copy_image_to_buffer(
                command_buffer.raw(),
                self.image,
                vk::ImageLayout::GENERAL,
                readback.raw(),
                std::slice::from_ref(&region),
            );
        }
        let back = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.pipeline_barrier(
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[back],
        );
        command_buffer.end()?;

        let command_buffers = [command_buffer.raw()];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        // SAFETY: the command buffer is fully recorded and the queue is valid.
        unsafe {
            device
                .raw()
                .queue_submit(
                    device.graphics_queue(),
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit target readback: {e:?}")))?;
            device
                .raw()
                .queue_wait_idle(device.graphics_queue())
                .map_err(|e| {
                    Error::Backend(format!("failed to wait for target readback: {e:?}"))
                })?;
        }

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        readback.read(&mut pixels)?;
        Ok(pixels)
    }

    /// Enqueue teardown for the color and depth images (and their
    /// allocations). The heap slots retire separately through `HeapSlots`'s
    /// own `Drop`.
    fn retire_images(&mut self) {
        self.ring.push(RetireAction::Image {
            device: self.device.clone(),
            view: self.image_view,
            image: self.image,
            allocation: self.allocation.take(),
            allocator: self.allocator.clone(),
        });
        if let (Some(view), Some(image), Some(allocation)) = (
            self.depth_image_view.take(),
            self.depth_image.take(),
            self.depth_allocation.take(),
        ) {
            self.ring.push(RetireAction::Image {
                device: self.device.clone(),
                view,
                image,
                allocation: Some(allocation),
                allocator: self.allocator.clone(),
            });
        }
    }
}

impl Drop for OffscreenTarget {
    fn drop(&mut self) {
        // Teardown is deferred: the ring drains RETIRE_RING frames later,
        // or at device teardown. The heap slots retire through `HeapSlots`'s
        // field drop after this body.
        self.retire_images();
    }
}

fn create_color_image(
    device: &Device,
    allocator: &std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    extent: vk::Extent2D,
    format: vk::Format,
) -> Result<(vk::Image, Allocation)> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // SAFETY: the device is valid and the create info describes a legal image.
    let image = unsafe {
        device
            .raw()
            .create_image(&image_info, None)
            .map_err(|e| Error::Backend(format!("failed to create offscreen image: {:?}", e)))?
    };

    // SAFETY: the image was just created and has no bound memory yet.
    let requirements = unsafe { device.raw().get_image_memory_requirements(image) };
    let allocation = allocator
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allocate(&AllocationCreateDesc {
            name: "offscreen-color",
            requirements,
            location: MemoryLocation::GpuOnly,
            linear: false,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })
        .map_err(|e| Error::Backend(format!("failed to allocate offscreen image memory: {e}")))?;

    // SAFETY: the allocation satisfies the image's memory requirements.
    unsafe {
        device
            .raw()
            .bind_image_memory(image, allocation.memory(), allocation.offset())
            .map_err(|e| {
                Error::Backend(format!("failed to bind offscreen image memory: {:?}", e))
            })?;
    }

    Ok((image, allocation))
}

fn create_image_view(
    device: &Device,
    image: vk::Image,
    format: vk::Format,
    aspect: vk::ImageAspectFlags,
) -> Result<(vk::ImageView, vk::ImageViewCreateInfo<'static>)> {
    let create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(aspect)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );
    // SAFETY: the image is valid and lives longer than the view.
    let view = unsafe {
        device
            .raw()
            .create_image_view(&create_info, None)
            .map_err(|e| {
                Error::Backend(format!("failed to create offscreen image view: {:?}", e))
            })?
    };
    Ok((view, create_info))
}

/// Create a `D32Sfloat` depth attachment image. No explicit transition is
/// needed: the render pass moves it from `UNDEFINED` to
/// `DEPTH_STENCIL_ATTACHMENT_OPTIMAL`.
fn create_depth_image(
    device: &Device,
    allocator: &std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    extent: vk::Extent2D,
) -> Result<(vk::Image, Allocation)> {
    let image_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::D32_SFLOAT)
        .extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    // SAFETY: the device is valid and the create info describes a legal image.
    let image = unsafe {
        device
            .raw()
            .create_image(&image_info, None)
            .map_err(|e| Error::Backend(format!("failed to create depth image: {:?}", e)))?
    };

    // SAFETY: the image was just created and has no bound memory yet.
    let requirements = unsafe { device.raw().get_image_memory_requirements(image) };
    let allocation = allocator
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allocate(&AllocationCreateDesc {
            name: "offscreen-depth",
            requirements,
            location: MemoryLocation::GpuOnly,
            linear: false,
            allocation_scheme: AllocationScheme::GpuAllocatorManaged,
        })
        .map_err(|e| Error::Backend(format!("failed to allocate depth image memory: {e}")))?;

    // SAFETY: the allocation satisfies the image's memory requirements.
    unsafe {
        device
            .raw()
            .bind_image_memory(image, allocation.memory(), allocation.offset())
            .map_err(|e| Error::Backend(format!("failed to bind depth image memory: {:?}", e)))?;
    }

    Ok((image, allocation))
}

/// The target's fixed sampler settings (linear filtering, clamp to edge),
/// written into the sampler heap slot at creation.
fn target_sampler_desc() -> SamplerDesc {
    SamplerDesc {
        min_filter: Filter::Linear,
        mag_filter: Filter::Linear,
        mipmap_filter: Some(Filter::Linear),
        wrap: WrapMode::ClampToEdge,
    }
}

/// Transition the image from UNDEFINED to SHADER_READ_ONLY_OPTIMAL via a
/// one-shot command buffer, so sampling is valid before the first render.
/// The submission waits on its own fence, not a queue wait: the transition
/// depends on no prior work, and same-queue submission order already puts
/// it ahead of the frame command buffers recorded afterwards.
fn transition_to_shader_read(device: &Device, image: vk::Image) -> Result<()> {
    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(device, queue_family_index)?;
    let mut command_buffer: CommandBuffer = command_pool.allocate_command_buffer()?;

    command_buffer.begin(crate::CommandBufferUsage::ONE_TIME_SUBMIT)?;
    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::GENERAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );
    command_buffer.pipeline_barrier(
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[barrier],
    );
    command_buffer.end()?;

    let fence = Fence::new(device, false)?;
    let command_buffers = [command_buffer.raw()];
    let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
    // SAFETY: the command buffer is fully recorded and the queue is valid.
    unsafe {
        device
            .raw()
            .queue_submit(
                device.graphics_queue(),
                std::slice::from_ref(&submit_info),
                fence.raw(),
            )
            .map_err(|e| Error::Backend(format!("failed to submit layout transition: {:?}", e)))?;
        device
            .raw()
            .wait_for_fences(std::slice::from_ref(&fence.raw()), true, u64::MAX)
            .map_err(|e| Error::Backend(format!("failed to wait for transition: {:?}", e)))?;
    }
    Ok(())
}
