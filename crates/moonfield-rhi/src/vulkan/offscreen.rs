//! Offscreen color target that can be sampled as a texture.
//!
//! Provides [`OffscreenTarget`], a renderable image used for editor viewports:
//! the scene is rendered into the image and a UI toolkit (e.g. egui) samples
//! it afterwards. The caller picks the attachment layout when beginning a
//! rendering pass; `SHADER_READ_ONLY_OPTIMAL` outside a pass keeps the image
//! sampleable with no explicit transitions. Sampling goes through the
//! descriptor heap: the target owns one image slot and one sampler slot
//! ([`texture_handle`](OffscreenTarget::texture_handle) /
//! [`sampler_handle`](OffscreenTarget::sampler_handle)), stable across
//! resizes — the image slot's descriptor is rewritten in place.
//!
//! [`OffscreenTarget::new_with_depth`] adds a `D32Sfloat` depth attachment for
//! depth-tested scene rendering (reverse-Z: the depth clear value is 0.0).

use crate::error::{Error, Result};
use crate::types::{Filter, Format, SamplerDesc, WrapMode};
use crate::vulkan::device::Device;
use crate::{CommandBuffer, CommandPool, DescriptorHeap, SamplerHandle, TextureHandle};
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};
use gpu_allocator::MemoryLocation;
use std::sync::Arc;

/// The target's descriptor-heap slots: the color view's image slot and the
/// fixed linear/clamp sampler slot. The image slot is rewritten in place on
/// resize, so both handles are stable for the target's lifetime.
struct HeapSlots {
    texture: TextureHandle,
    sampler: SamplerHandle,
    heap: Arc<DescriptorHeap>,
    /// The color view's create info, owned for its *lifetime*: the heap's
    /// descriptor write encoded a pointer to it (`ImageDescriptorInfoEXT.
    /// p_view`), so it must outlive the slot. Rebuilt on resize.
    view_create_info: vk::ImageViewCreateInfo<'static>,
}

impl HeapSlots {
    /// Allocate the image and sampler slots and write both descriptors.
    fn new(device: &Device, view_create_info: vk::ImageViewCreateInfo<'static>) -> Result<Self> {
        let heap = device.descriptor_heap();
        let texture = heap.alloc_image_slot()?;
        heap.write_resource_descriptors(&[(
            texture,
            crate::TextureSlotDesc {
                view_create_info: &view_create_info,
                layout: vk::ImageLayout::GENERAL,
            },
        )])?;
        let sampler = heap.alloc_sampler_slot()?;
        heap.write_samplers(&[(sampler, target_sampler_desc())])?;
        Ok(Self {
            texture,
            sampler,
            heap,
            view_create_info,
        })
    }

    /// Rewrite the image slot against a recreated view (resize). The device
    /// is idle when the caller resizes, so the in-place write cannot race an
    /// in-flight frame; the slot index itself is unchanged.
    fn rewrite(&mut self, view_create_info: vk::ImageViewCreateInfo<'static>) -> Result<()> {
        self.heap.write_resource_descriptors(&[(
            self.texture,
            crate::TextureSlotDesc {
                view_create_info: &view_create_info,
                layout: vk::ImageLayout::GENERAL,
            },
        )])?;
        self.view_create_info = view_create_info;
        Ok(())
    }
}

impl Drop for HeapSlots {
    fn drop(&mut self) {
        if let Err(e) = self.heap.free_image_slot(self.texture) {
            moonfield_log::error!("failed to free offscreen texture slot: {e}");
        }
        if let Err(e) = self.heap.free_sampler_slot(self.sampler) {
            moonfield_log::error!("failed to free offscreen sampler slot: {e}");
        }
    }
}

/// A renderable and sampleable offscreen color target.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: the heap slots first (a freed slot is never referenced
/// again), then view, image and its allocation, then the device-owning
/// handles. The optional depth image/view/allocation are destroyed explicitly
/// in [`OffscreenTarget::destroy_image_resources`] alongside the color ones.
/// There is no render pass or framebuffer — with dynamic rendering the caller
/// builds attachments inline via [`RenderPassDesc`](crate::RenderPassDesc).
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
        // heap; the slots stay stable for the target's lifetime.
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
            format,
            extent,
            has_depth: with_depth,
        })
    }

    /// Resize the target, recreating the image and view.
    ///
    /// Waits for the device to go idle before destroying the old resources.
    /// Zero dimensions are ignored (e.g. a minimized viewport panel).
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.extent.width == width && self.extent.height == height {
            return Ok(());
        }

        // SAFETY: the device is valid; waiting for idle guarantees the old
        // image is no longer sampled or rendered into.
        unsafe {
            self.device
                .device_wait_idle()
                .map_err(|e| Error::Backend(format!("failed to wait for device idle: {:?}", e)))?;
        }
        self.destroy_image_resources();

        let extent = vk::Extent2D { width, height };
        let format_vk = self.format.to_vk();
        let (image, allocation) = create_color_image(device, &self.allocator, extent, format_vk)?;
        let (image_view, view_create_info) =
            create_image_view(device, image, format_vk, vk::ImageAspectFlags::COLOR)?;
        self.image_view = image_view;
        self.image = image;
        self.allocation = Some(allocation);
        self.extent = extent;
        // The device is idle and the slot index is stable: rewrite the
        // descriptor in place so existing handles keep working.
        self.heap_slots.rewrite(view_create_info)?;
        if self.has_depth {
            let (depth_image, depth_allocation) =
                create_depth_image(device, &self.allocator, extent)?;
            self.depth_image_view = Some(
                create_image_view(
                    device,
                    depth_image,
                    vk::Format::D32_SFLOAT,
                    vk::ImageAspectFlags::DEPTH,
                )?
                .0,
            );
            self.depth_image = Some(depth_image);
            self.depth_allocation = Some(depth_allocation);
        }

        transition_to_shader_read(device, image)?;
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
    pub fn view(&self) -> crate::bind::TextureView {
        crate::bind::TextureView::borrow_raw(self.image_view, self.device.clone())
    }

    /// Borrow the depth image view, if present (for the depth attachment of a
    /// [`RenderPassDesc`](crate::RenderPassDesc)).
    pub fn depth_view(&self) -> Option<crate::bind::TextureView> {
        self.depth_image_view
            .map(|view| crate::bind::TextureView::borrow_raw(view, self.device.clone()))
    }

    /// The color attachment format of this target.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The color view's descriptor-heap slot, for bindless sampling (e.g.
    /// the editor's egui pass). Stable across resizes.
    pub fn texture_handle(&self) -> TextureHandle {
        self.heap_slots.texture
    }

    /// The target's fixed sampler's descriptor-heap slot (linear filtering,
    /// clamp-to-edge). Stable across resizes.
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
            MemoryLocation::GpuToCpu,
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

    /// Destroy image, view and free the allocation (color and, when present,
    /// depth). The caller must ensure the GPU is idle (see [`resize`] and
    /// `Drop`).
    fn destroy_image_resources(&mut self) {
        // SAFETY: the GPU is idle by contract of the callers, so these
        // handles are no longer in use.
        unsafe {
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
            if let Some(depth_view) = self.depth_image_view.take() {
                self.device.destroy_image_view(depth_view, None);
            }
            if let Some(depth_image) = self.depth_image.take() {
                self.device.destroy_image(depth_image, None);
            }
        }
        if let Some(allocation) = self.allocation.take() {
            let mut allocator = self.allocator.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = allocator.free(allocation) {
                log_free_error(&e);
            }
        }
        if let Some(depth_allocation) = self.depth_allocation.take() {
            let mut allocator = self.allocator.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = allocator.free(depth_allocation) {
                log_free_error(&e);
            }
        }
    }
}

impl Drop for OffscreenTarget {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so the image is not destroyed while in use
        // (heap slot frees in `HeapSlots::drop` are plain bookkeeping).
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        self.destroy_image_resources();
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
            .map_err(|e| Error::Backend(format!("failed to submit layout transition: {:?}", e)))?;
        device
            .raw()
            .queue_wait_idle(device.graphics_queue())
            .map_err(|e| Error::Backend(format!("failed to wait for transition: {:?}", e)))?;
    }
    Ok(())
}

fn log_free_error(err: &gpu_allocator::AllocationError) {
    // gpu-allocator reports double-frees and leaks here; destruction must not
    // panic, so surface the error through the log crate instead.
    moonfield_log::error!("failed to free offscreen image allocation: {err}");
}
