//! 2D image creation: image + memory binding + view in one call.
//!
//! [`Image2d`] folds the "create image → query requirements → allocate →
//! bind → create view" sequence shared by textures, offscreen targets, and
//! depth buffers into one helper. The image is always a single-mip,
//! optimally-tiled, GPU-only 2D image with an exclusive-sharing full-range
//! view — the only shape the RHI creates today.

use crate::error::{Error, Result};
use crate::vulkan::device::DeviceContext;
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

/// A created 2D image with its bound allocation and full-range view.
pub(crate) struct Image2d {
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    /// The view's create info, owned for its *lifetime*: a descriptor-heap
    /// write encodes a pointer to it (`ImageDescriptorInfoEXT.p_view`), so
    /// it must outlive any slot written from it.
    pub(crate) view_create_info: vk::ImageViewCreateInfo<'static>,
    pub(crate) allocation: Allocation,
}

impl Image2d {
    /// Create a `width`×`height` 2D image (single mip, optimal tiling,
    /// GPU-only memory) plus a full-range view over `aspect`. `name` labels
    /// the allocation for diagnostics.
    pub(crate) fn new(
        ctx: &DeviceContext,
        name: &'static str,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
    ) -> Result<Self> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: the device is valid (kept alive by `ctx`) and the create
        // info describes a legal image.
        let image = unsafe {
            ctx.raw()
                .create_image(&image_info, None)
                .map_err(|e| Error::Backend(format!("failed to create {name} image: {e:?}")))?
        };
        // SAFETY: the image was just created and has no bound memory yet.
        let requirements = unsafe { ctx.raw().get_image_memory_requirements(image) };
        let allocation = ctx
            .allocator()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name,
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| Error::Backend(format!("failed to allocate {name} image memory: {e}")))?;
        // SAFETY: the allocation satisfies the image's memory requirements
        // (queried above) and the image has no bound memory yet.
        unsafe {
            ctx.raw()
                .bind_image_memory(image, allocation.memory(), allocation.offset())
                .map_err(|e| {
                    Error::Backend(format!("failed to bind {name} image memory: {e:?}"))
                })?;
        }

        let view_create_info = vk::ImageViewCreateInfo::default()
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
        // SAFETY: the image is valid and outlives the view.
        let view = unsafe {
            ctx.raw()
                .create_image_view(&view_create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create {name} image view: {e:?}")))?
        };
        Ok(Self {
            image,
            view,
            view_create_info,
            allocation,
        })
    }
}
