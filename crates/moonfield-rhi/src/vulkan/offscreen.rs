//! Offscreen color target that can be sampled as a texture.
//!
//! Provides [`OffscreenTarget`], a renderable image used for editor viewports:
//! the scene is rendered into the image and a UI toolkit (e.g. egui) samples
//! it afterwards. The caller picks the attachment layout when beginning a
//! rendering pass; `GENERAL` outside a pass keeps the image sampleable with
//! no explicit transitions. Sampling goes through the
//! descriptor heap: the target owns one image slot, and its sampler comes
//! from the heap's description cache; a resize allocates a new image slot
//! and retires the old one, so holders re-register when the handles change.
//!
//! [`OffscreenTarget::new_with_depth`] adds a `D32Sfloat` depth attachment for
//! depth-tested scene rendering (reverse-Z: the depth clear value is 0.0).

use crate::error::{Error, Result};
use crate::types::{Filter, Format, SamplerDesc, WrapMode};
use crate::vulkan::device::{Device, DeviceContext};
use crate::vulkan::image::Image2d;
use crate::vulkan::memory::{GpuAllocation, Memory};
use crate::vulkan::retire::RetireAction;
use crate::{CommandPool, DescriptorHeap, SamplerHandle, TextureHandle};
use ash::vk;
use gpu_allocator::vulkan::Allocation;
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
    /// Shared device state; `Drop` enqueues the slot teardown into its
    /// retirement ring.
    ctx: DeviceContext,
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
            ctx: device.context(),
        })
    }
}

impl Drop for HeapSlots {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still index the image
        // slot. The action carries the view create info (the heap's
        // encoded descriptor references it by pointer). The sampler slot
        // is cached and never freed.
        self.ctx.ring().push(RetireAction::ImageSlot {
            slots: self.heap.image_slots(),
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
    /// Shared device state: keeps the device alive and takes the deferred
    /// image teardown in `Drop`.
    ctx: DeviceContext,
    format: Format,
    extent: vk::Extent2D,
    has_depth: bool,
}

impl OffscreenTarget {
    /// Create an offscreen target of `width`×`height` with the given color
    /// format. The fresh image's transition to `GENERAL` records into the
    /// device's shared frame uploader and executes at its next flush — the
    /// frame loop's submit, which orders the frame's command buffer behind
    /// the uploader batch. Callers outside a frame loop flush the uploader
    /// (`Device::uploader`) before submitting work that touches the target.
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
        let ctx = device.context();
        let color = create_color_image(&ctx, extent, format_vk)?;
        let depth = if with_depth {
            Some(create_depth_image(&ctx, extent)?)
        } else {
            None
        };

        transition_to_shader_read(device, color.image)?;

        // Publish the color view and the fixed sampler to the descriptor
        // heap.
        let heap_slots = HeapSlots::new(device, color.view_create_info)?;

        Ok(Self {
            heap_slots,
            image_view: color.view,
            image: color.image,
            allocation: Some(color.allocation),
            depth_image_view: depth.as_ref().map(|d| d.view),
            depth_image: depth.as_ref().map(|d| d.image),
            depth_allocation: depth.map(|d| d.allocation),
            ctx,
            format,
            extent,
            has_depth: with_depth,
        })
    }

    /// Resize the target: allocate a new image, view, and image slot; the
    /// old one retires through the ring when the fields are replaced, so
    /// in-flight frames keep sampling valid memory. Holders re-register
    /// when [`texture_handle`](Self::texture_handle) changes. Zero
    /// dimensions are ignored (e.g. a minimized viewport panel). The new
    /// image's `GENERAL` transition rides the shared uploader, same as at
    /// creation (see [`new`](Self::new)).
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.extent.width == width && self.extent.height == height {
            return Ok(());
        }

        let extent = vk::Extent2D { width, height };
        let format_vk = self.format.to_vk();
        let color = create_color_image(&self.ctx, extent, format_vk)?;
        let heap_slots = HeapSlots::new(device, color.view_create_info)?;
        let depth = if self.has_depth {
            Some(create_depth_image(&self.ctx, extent)?)
        } else {
            None
        };

        transition_to_shader_read(device, color.image)?;

        // Swap in the new target; the old image, views, allocations, and
        // heap slots retire through the ring.
        self.retire_images();
        self.heap_slots = heap_slots;
        self.image_view = color.view;
        self.image = color.image;
        self.allocation = Some(color.allocation);
        self.depth_image_view = depth.as_ref().map(|d| d.view);
        self.depth_image = depth.as_ref().map(|d| d.image);
        self.depth_allocation = depth.map(|d| d.allocation);
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
        crate::vulkan::view::TextureView::borrow_raw(self.image_view, self.ctx.clone())
    }

    /// Borrow the depth image view, if present (for the depth attachment of a
    /// [`RenderPassDesc`](crate::RenderPassDesc)).
    pub fn depth_view(&self) -> Option<crate::vulkan::view::TextureView> {
        self.depth_image_view
            .map(|view| crate::vulkan::view::TextureView::borrow_raw(view, self.ctx.clone()))
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
        let readback = GpuAllocation::new(device, (width * height * 4) as u64, Memory::Readback)?;

        let command_pool = CommandPool::new(device, device.queue_family_indices().graphics)?;
        let mut command_buffer = command_pool.allocate_command_buffer()?;
        command_buffer.begin(crate::CommandBufferUsage::ONE_TIME_SUBMIT)?;
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let to_transfer = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_READ)
            .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
            .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.image_barriers(std::slice::from_ref(&to_transfer));
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
        // SAFETY: the target is in GENERAL (readable as a transfer source)
        // and the buffer fits it.
        unsafe {
            device.raw().cmd_copy_image_to_buffer(
                command_buffer.raw(),
                self.image,
                vk::ImageLayout::GENERAL,
                readback.buffer(),
                std::slice::from_ref(&region),
            );
        }
        let back = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
            .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.image_barriers(std::slice::from_ref(&back));
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
        readback.read_bytes(&mut pixels)?;
        Ok(pixels)
    }

    /// Enqueue teardown for the color and depth images (and their
    /// allocations). The heap slots retire separately through `HeapSlots`'s
    /// own `Drop`.
    fn retire_images(&mut self) {
        self.ctx.ring().push(RetireAction::Image {
            device: self.ctx.raw().clone(),
            view: self.image_view,
            image: self.image,
            allocation: self.allocation.take(),
            allocator: self.ctx.allocator().clone(),
        });
        if let (Some(view), Some(image), Some(allocation)) = (
            self.depth_image_view.take(),
            self.depth_image.take(),
            self.depth_allocation.take(),
        ) {
            self.ctx.ring().push(RetireAction::Image {
                device: self.ctx.raw().clone(),
                view,
                image,
                allocation: Some(allocation),
                allocator: self.ctx.allocator().clone(),
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

/// A standalone `D32Sfloat` depth attachment, sized to match a color target
/// it accompanies (e.g. a window's swapchain extent).
///
/// Unlike [`OffscreenTarget`] the depth buffer is never sampled, so it owns
/// no descriptor-heap slots. Teardown is deferred through the device's
/// retirement ring, same as `OffscreenTarget`'s images.
pub struct DepthBuffer {
    image: vk::Image,
    image_view: vk::ImageView,
    allocation: Option<Allocation>,
    /// Shared device state: keeps the device alive and takes the deferred
    /// teardown in `Drop`.
    ctx: DeviceContext,
    extent: vk::Extent2D,
}

impl DepthBuffer {
    /// Create a depth buffer of `width`×`height` (reverse-Z: the pass clears
    /// depth to 0.0, near → 1).
    pub fn new(device: &Device, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "depth buffer dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let extent = vk::Extent2D { width, height };
        let depth = create_depth_image(&device.context(), extent)?;
        Ok(Self {
            image: depth.image,
            image_view: depth.view,
            allocation: Some(depth.allocation),
            ctx: device.context(),
            extent,
        })
    }

    /// Resize to a new extent; the old image retires through the ring, so
    /// in-flight frames keep referencing valid memory.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.extent.width == width && self.extent.height == height {
            return Ok(());
        }
        *self = Self::new(device, width, height)?;
        Ok(())
    }

    /// Borrow the depth image view (for the depth attachment of a
    /// [`RenderPassDesc`](crate::RenderPassDesc)). The view borrows this
    /// buffer's; it must not outlive the buffer.
    pub fn view(&self) -> crate::vulkan::view::TextureView {
        crate::vulkan::view::TextureView::borrow_raw(self.image_view, self.ctx.clone())
    }

    /// The `(width, height)` of the buffer.
    pub fn extent(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }
}

impl Drop for DepthBuffer {
    fn drop(&mut self) {
        // Deferred teardown, same contract as `OffscreenTarget::retire_images`.
        self.ctx.ring().push(RetireAction::Image {
            device: self.ctx.raw().clone(),
            view: self.image_view,
            image: self.image,
            allocation: self.allocation.take(),
            allocator: self.ctx.allocator().clone(),
        });
    }
}

/// Create the color attachment image (renderable + sampleable + copy source).
fn create_color_image(
    ctx: &DeviceContext,
    extent: vk::Extent2D,
    format: vk::Format,
) -> Result<Image2d> {
    Image2d::new(
        ctx,
        "offscreen-color",
        extent.width,
        extent.height,
        format,
        vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC,
        vk::ImageAspectFlags::COLOR,
    )
}

/// Create a `D32Sfloat` depth attachment image. No explicit transition is
/// needed: the render pass moves it from `UNDEFINED` to
/// `DEPTH_STENCIL_ATTACHMENT_OPTIMAL`.
fn create_depth_image(ctx: &DeviceContext, extent: vk::Extent2D) -> Result<Image2d> {
    Image2d::new(
        ctx,
        "offscreen-depth",
        extent.width,
        extent.height,
        vk::Format::D32_SFLOAT,
        vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        vk::ImageAspectFlags::DEPTH,
    )
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

/// Record the fresh image's `UNDEFINED` → `GENERAL` transition (the unified
/// layout, so sampling is valid before the first render) into the device's
/// shared frame uploader. Recording only, never blocking: the batch submits
/// at the next uploader flush, and the frame loop's submit orders the frame
/// command buffer behind it through the uploader's timeline.
fn transition_to_shader_read(device: &Device, image: vk::Image) -> Result<()> {
    device
        .uploader()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transition_image(image)
}
