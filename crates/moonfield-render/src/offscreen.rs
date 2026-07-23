//! Offscreen color target that can be sampled as a texture.
//!
//! Provides [`OffscreenTarget`], a color image + render pass + framebuffer
//! bundle used for editor viewports: the scene is rendered into the image and
//! a UI toolkit (e.g. egui) samples it afterwards. The render pass finishes in
//! `SHADER_READ_ONLY_OPTIMAL`, so the image is ready for sampling at the end
//! of every render pass without explicit transitions.

use crate::device::Device;
use crate::error::{Error, Result};
use crate::framebuffer::Framebuffer;
use crate::render_pass::RenderPass;
use crate::{CommandBuffer, CommandPool};
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator};
use gpu_allocator::MemoryLocation;
use std::sync::{Arc, Mutex};

/// A renderable and sampleable offscreen color target.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: framebuffer and render pass first, then view, sampler,
/// image and its allocation, and finally the device-owning handles.
pub struct OffscreenTarget {
    framebuffer: Framebuffer,
    render_pass: RenderPass,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    image: vk::Image,
    allocation: Option<Allocation>,
    allocator: Arc<Mutex<Allocator>>,
    device: ash::Device,
    format: vk::Format,
    extent: vk::Extent2D,
}

impl OffscreenTarget {
    /// Create an offscreen target of `width`×`height` with the given color
    /// format. The image is transitioned to `SHADER_READ_ONLY_OPTIMAL` so it
    /// can be sampled before the first frame is rendered.
    pub fn new(
        device: &Device,
        allocator: Arc<Mutex<Allocator>>,
        width: u32,
        height: u32,
        format: vk::Format,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "offscreen target dimensions must be non-zero, got {}x{}",
                width, height
            )));
        }

        let extent = vk::Extent2D { width, height };
        let (image, allocation) = create_color_image(device, &allocator, extent, format)?;
        let image_view = create_image_view(device, image, format)?;
        let sampler = create_sampler(device)?;
        let render_pass = RenderPass::new_with_final_layout(
            device,
            format,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        )?;
        let framebuffer = Framebuffer::new(device, &render_pass, &[image_view], extent)?;

        transition_to_shader_read(device, image)?;

        Ok(Self {
            framebuffer,
            render_pass,
            image_view,
            sampler,
            image,
            allocation: Some(allocation),
            allocator,
            device: device.raw().clone(),
            format,
            extent,
        })
    }

    /// Resize the target, recreating the image, view, and framebuffer.
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
        let (image, allocation) = create_color_image(device, &self.allocator, extent, self.format)?;
        self.image_view = create_image_view(device, image, self.format)?;
        self.image = image;
        self.allocation = Some(allocation);
        self.extent = extent;
        self.framebuffer = Framebuffer::new(device, &self.render_pass, &[self.image_view], extent)?;

        transition_to_shader_read(device, image)?;
        Ok(())
    }

    /// Access the color image view (for sampling in a UI renderer).
    pub fn image_view(&self) -> vk::ImageView {
        self.image_view
    }

    /// Access the sampler paired with the color image.
    pub fn sampler(&self) -> vk::Sampler {
        self.sampler
    }

    /// Access the render pass targeting this offscreen image.
    pub fn render_pass(&self) -> &RenderPass {
        &self.render_pass
    }

    /// Access the framebuffer for recording the scene pass.
    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    /// The `(width, height)` of the target.
    pub fn extent(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    /// Destroy image, view and free the allocation. The caller must ensure
    /// the GPU is idle (see [`resize`] and `Drop`).
    fn destroy_image_resources(&mut self) {
        // SAFETY: the GPU is idle by contract of the callers, so these
        // handles are no longer in use.
        unsafe {
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
        if let Some(allocation) = self.allocation.take() {
            let mut allocator = self.allocator.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = allocator.free(allocation) {
                log_free_error(&e);
            }
        }
    }
}

impl Drop for OffscreenTarget {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so the image is not destroyed while in use.
        unsafe {
            let _ = self.device.device_wait_idle();
        }
        self.destroy_image_resources();
        // SAFETY: the sampler is no longer referenced once the image is gone.
        unsafe {
            self.device.destroy_sampler(self.sampler, None);
        }
    }
}

fn create_color_image(
    device: &Device,
    allocator: &Arc<Mutex<Allocator>>,
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
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
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
) -> Result<vk::ImageView> {
    let create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(1)
                .base_array_layer(0)
                .layer_count(1),
        );
    // SAFETY: the image is valid and lives longer than the view.
    unsafe {
        device
            .raw()
            .create_image_view(&create_info, None)
            .map_err(|e| Error::Backend(format!("failed to create offscreen image view: {:?}", e)))
    }
}

fn create_sampler(device: &Device) -> Result<vk::Sampler> {
    let create_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .max_lod(0.0);
    // SAFETY: the device is valid.
    unsafe {
        device
            .raw()
            .create_sampler(&create_info, None)
            .map_err(|e| Error::Backend(format!("failed to create sampler: {:?}", e)))
    }
}

/// Transition the image from UNDEFINED to SHADER_READ_ONLY_OPTIMAL via a
/// one-shot command buffer, so sampling is valid before the first render.
fn transition_to_shader_read(device: &Device, image: vk::Image) -> Result<()> {
    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(device, queue_family_index)?;
    let mut command_buffer: CommandBuffer = command_pool.allocate_command_buffer()?;

    command_buffer.begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)?;
    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
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
