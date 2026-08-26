//! GPU textures: a sampled 2D image with owned allocation and upload path.
//!
//! [`Texture`] is the crate-vocabulary counterpart of "image + view +
//! allocation" for shader-sampled data (e.g. UI atlas textures). Uploads go
//! through a blocking staging copy on the graphics queue, transitioning the
//! image into `SHADER_READ_ONLY_OPTIMAL`.

use crate::error::{Error, Result};
use crate::types::Format;
use crate::vulkan::device::Device;
use crate::{CommandBufferUsage, CommandPool};
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};
use gpu_allocator::MemoryLocation;

/// A sampled 2D texture with owned memory.
///
/// Field order matters for drop safety: the view is destroyed before the
/// image, and the allocation is freed after both.
pub struct Texture {
    image_view: vk::ImageView,
    image: vk::Image,
    allocation: Option<Allocation>,
    device: ash::Device,
    allocator: std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    width: u32,
    height: u32,
}

impl Texture {
    /// Create a `width`×`height` sampled texture (single mip, `TRANSFER_DST`
    /// for uploads). The image starts in `UNDEFINED`; the first
    /// [`upload`](Self::upload) transitions it to shader-readable.
    pub fn new(device: &Device, width: u32, height: u32, format: Format) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "texture dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format.to_vk())
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: the device is valid and the create info describes a legal image.
        let image = unsafe {
            device
                .raw()
                .create_image(&image_info, None)
                .map_err(|e| Error::Backend(format!("failed to create texture image: {e:?}")))?
        };
        // SAFETY: the image was just created and has no bound memory yet.
        let requirements = unsafe { device.raw().get_image_memory_requirements(image) };
        let allocation = device
            .allocator()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name: "texture",
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| Error::Backend(format!("failed to allocate texture memory: {e}")))?;
        // SAFETY: the allocation satisfies the image's memory requirements.
        unsafe {
            device
                .raw()
                .bind_image_memory(image, allocation.memory(), allocation.offset())
        }
        .map_err(|e| Error::Backend(format!("failed to bind texture memory: {e:?}")))?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format.to_vk())
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        // SAFETY: the image is valid and outlives the view.
        let image_view = unsafe {
            device
                .raw()
                .create_image_view(&view_info, None)
                .map_err(|e| Error::Backend(format!("failed to create texture view: {e:?}")))?
        };

        Ok(Self {
            image_view,
            image,
            allocation: Some(allocation),
            device: device.raw().clone(),
            allocator: device.allocator().clone(),
            width,
            height,
        })
    }

    /// Upload RGBA8 pixels (4 bytes per pixel, row-major). `offset: None`
    /// covers the full texture of a fresh (`UNDEFINED`) image; `Some((x, y))`
    /// updates a sub-region of a shader-readable one. `bytes` must cover the
    /// full texture for a full upload, or the sub-region for a partial one.
    /// Blocks on the graphics queue.
    pub fn upload(
        &self,
        device: &Device,
        upload_pool: &CommandPool,
        bytes: &[u8],
        offset: Option<(i32, i32)>,
        region: (u32, u32),
    ) -> Result<()> {
        let staging = crate::Buffer::new(
            device,
            bytes.len() as u64,
            crate::BufferUsage::COPY_SRC,
            MemoryLocation::CpuToGpu,
        )?;
        staging.upload(device, bytes)?;

        let mut command_buffer = upload_pool.allocate_command_buffer()?;
        command_buffer.begin(CommandBufferUsage::ONE_TIME_SUBMIT)?;

        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let (old_layout, src_access, src_stage) = match offset {
            Some(_) => (
                vk::ImageLayout::GENERAL,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            ),
            None => (
                vk::ImageLayout::UNDEFINED,
                vk::AccessFlags::empty(),
                vk::PipelineStageFlags::TOP_OF_PIPE,
            ),
        };
        let to_transfer = vk::ImageMemoryBarrier::default()
            .src_access_mask(src_access)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.pipeline_barrier(
            src_stage,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );

        let (x, y) = offset.unwrap_or((0, 0));
        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(0)
            .buffer_row_length(0)
            .buffer_image_height(0)
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_offset(vk::Offset3D { x, y, z: 0 })
            .image_extent(vk::Extent3D {
                width: region.0,
                height: region.1,
                depth: 1,
            });
        // SAFETY: the staging buffer holds `bytes`, the image is in
        // TRANSFER_DST_OPTIMAL, and the region fits the image.
        unsafe {
            device.raw().cmd_copy_buffer_to_image(
                command_buffer.raw(),
                staging.raw(),
                self.image,
                vk::ImageLayout::GENERAL,
                std::slice::from_ref(&copy_region),
            );
        }

        let to_shader_read = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
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
            &[to_shader_read],
        );
        command_buffer.end()?;

        let command_buffers = [command_buffer.raw()];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        let queue = device.graphics_queue();
        // SAFETY: the command buffer is fully recorded and the queue is valid.
        unsafe {
            device
                .raw()
                .queue_submit(queue, std::slice::from_ref(&submit_info), vk::Fence::null())
                .map_err(|e| Error::Backend(format!("failed to submit texture upload: {e:?}")))?;
            device
                .raw()
                .queue_wait_idle(queue)
                .map_err(|e| Error::Backend(format!("failed to wait for texture upload: {e:?}")))?;
        }
        Ok(())
    }

    /// Borrow the image view as a backend-neutral
    /// [`TextureView`](crate::TextureView); it must not outlive the texture.
    pub fn view(&self) -> crate::bind::TextureView {
        crate::bind::TextureView::borrow_raw(self.image_view, self.device.clone())
    }

    /// The `(width, height)` of the texture.
    pub fn extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        // SAFETY: the caller defers destruction past the in-flight frames
        // that sampled this texture.
        unsafe {
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
        if let Some(allocation) = self.allocation.take() {
            if let Err(e) = self
                .allocator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .free(allocation)
            {
                moonfield_log::error!("failed to free texture allocation: {e}");
            }
        }
    }
}
