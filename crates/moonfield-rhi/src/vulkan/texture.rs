//! GPU textures: a sampled 2D image with owned allocation and upload path.
//!
//! [`Texture`] is the crate-vocabulary counterpart of "image + view +
//! allocation" for shader-sampled data (e.g. UI atlas textures). Uploads are
//! staged in a [`FrameUploader`]'s bump arena and recorded into its current
//! frame, transitioning the image to a shader-readable layout; the caller
//! submits with [`FrameUploader::end_frame`].

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::types::Format;
use crate::vulkan::device::Device;
use crate::vulkan::retire::{RetireAction, RetirementRing};
use crate::{DescriptorHeap, FrameUploader, TextureHandle};
use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};

struct TextureSlot {
    handle: TextureHandle,
    heap: Arc<DescriptorHeap>,
    /// The view's create info, owned here for its *lifetime*: the heap's
    /// descriptor write encoded a pointer to it (`ImageDescriptorInfoEXT.
    /// p_view`), so it must outlive the slot. `Drop` moves it into the
    /// retirement action, which frees the slot and drops it.
    view_create_info: vk::ImageViewCreateInfo<'static>,
}

/// A sampled 2D texture with owned memory.
///
/// Teardown is deferred through the device's retirement ring; the
/// view-before-image-before-allocation order lives in the retirement
/// action.
pub struct Texture {
    image_view: vk::ImageView,
    image: vk::Image,
    allocation: Option<Allocation>,
    device: ash::Device,
    allocator: std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    /// Device-level retirement ring; `Drop` enqueues the teardown here.
    ring: Arc<RetirementRing>,
    width: u32,
    height: u32,
    slot: Option<TextureSlot>,
}

impl Texture {
    fn create_image(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<(
        vk::Image,
        vk::ImageView,
        vk::ImageViewCreateInfo<'static>,
        Allocation,
    )> {
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
        Ok((image, image_view, view_info, allocation))
    }
    /// Create a `width`×`height` sampled texture (single mip, `TRANSFER_DST`
    /// for uploads). The image starts in `UNDEFINED`; the first
    /// [`upload`](Self::upload) transitions it to shader-readable.
    pub fn new(device: &Device, width: u32, height: u32, format: Format) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "texture dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let (image, image_view, _view_create_info, allocation) =
            Self::create_image(device, width, height, format)?;
        Ok(Self {
            image_view,
            image,
            allocation: Some(allocation),
            device: device.raw().clone(),
            allocator: device.allocator().clone(),
            ring: device.retirement_ring(),
            width,
            height,
            slot: None,
        })
    }

    pub fn bindless(
        device: &Device,
        uploader: &mut FrameUploader,
        width: u32,
        height: u32,
        format: Format,
        bytes: &[u8],
    ) -> Result<Self> {
        let expected = width as usize * height as usize * format.bytes_per_pixel();
        if bytes.len() != expected {
            return Err(Error::Validation(format!(
                "texture upload needs {expected} bytes, got {}",
                bytes.len()
            )));
        }
        let (image, image_view, view_create_info, allocation) =
            Self::create_image(device, width, height, format)?;
        uploader.upload_image(image, bytes, None, (width, height))?;
        let heap = device.descriptor_heap();
        let handle = heap.alloc_image_slot()?;
        heap.write_resource_descriptors(&[(
            handle,
            crate::vulkan::descriptor_heap::TextureSlotDesc::new(
                &view_create_info,
                vk::ImageLayout::GENERAL,
            ),
        )])?;
        Ok(Self {
            image_view,
            image,
            allocation: Some(allocation),
            device: device.raw().clone(),
            allocator: device.allocator().clone(),
            ring: device.retirement_ring(),
            width,
            height,
            slot: Some(TextureSlot {
                handle,
                heap,
                view_create_info,
            }),
        })
    }

    /// Upload RGBA8 pixels (4 bytes per pixel, row-major). `offset: None`
    /// covers the full texture of a fresh (`UNDEFINED`) image; `Some((x, y))`
    /// updates a sub-region of a shader-readable one. `bytes` must cover the
    /// full texture for a full upload, or the sub-region for a partial one.
    ///
    /// The copy is staged in the [`FrameUploader`]'s arena and recorded into
    /// its current frame; callers submit with
    /// [`FrameUploader::end_frame`].
    pub fn upload(
        &self,
        uploader: &mut FrameUploader,
        bytes: &[u8],
        offset: Option<(i32, i32)>,
        region: (u32, u32),
    ) -> Result<()> {
        uploader.upload_image(self.image, bytes, offset, region)
    }

    /// Borrow the image view as a backend-neutral [`TextureView`]; it must not
    /// outlive the texture.
    pub fn view(&self) -> crate::vulkan::view::TextureView {
        crate::vulkan::view::TextureView::borrow_raw(self.image_view, self.device.clone())
    }

    /// The `(width, height)` of the texture.
    pub fn extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The bindless heap slot, `None` for escape-hatch textures (e.g. the
    /// egui interop path) that do not participate in the descriptor heap.
    pub fn handle(&self) -> Option<TextureHandle> {
        self.slot.as_ref().map(|slot| slot.handle)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still sample the image
        // through its heap slot. The slot action carries the view create
        // info — the heap's encoded descriptor references it by pointer, so
        // it must stay alive until the slot is freed.
        if let Some(slot) = self.slot.take() {
            self.ring.push(RetireAction::ImageSlot {
                heap: slot.heap,
                handle: slot.handle,
                view_create_info: slot.view_create_info,
            });
        }
        self.ring.push(RetireAction::Image {
            device: self.device.clone(),
            view: self.image_view,
            image: self.image,
            allocation: self.allocation.take(),
            allocator: self.allocator.clone(),
        });
    }
}
