//! GPU textures: a sampled 2D image with owned allocation and upload path.
//!
//! [`Texture`] is the crate-vocabulary counterpart of "image + view +
//! allocation" for shader-sampled data (e.g. UI atlas textures). Uploads are
//! staged in a [`FrameUploader`]'s bump arena and recorded into its current
//! frame, transitioning the image to a shader-readable layout; the caller
//! submits with [`FrameUploader::end_frame`].

use std::sync::Arc;

use super::device::{Device, DeviceContext};
use super::formats::ToVk;
use super::image::Image2d;
use super::retire::RetireAction;
use crate::error::{Error, Result};
use crate::types::Format;
use crate::{DescriptorHeap, FrameUploader, TextureHandle};
use ash::vk;
use gpu_allocator::vulkan::Allocation;

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
    /// Shared device state: keeps the device alive and takes the deferred
    /// teardown in `Drop`.
    ctx: DeviceContext,
    width: u32,
    height: u32,
    format: Format,
    slot: Option<TextureSlot>,
    /// The storage-image slot of a compute-writable texture
    /// ([`Texture::storage_image`]); `None` for upload-only textures.
    storage_slot: Option<TextureSlot>,
}

impl Texture {
    fn create_image(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<(Image2d, DeviceContext)> {
        let ctx = device.context();
        let image = Image2d::new(
            &ctx,
            "texture",
            width,
            height,
            format.to_vk(),
            usage,
            vk::ImageAspectFlags::COLOR,
        )?;
        Ok((image, ctx))
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
        let (image, ctx) = Self::create_image(
            device,
            width,
            height,
            format,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        )?;
        Ok(Self {
            image_view: image.view,
            image: image.image,
            allocation: Some(image.allocation),
            ctx,
            width,
            height,
            format,
            slot: None,
            storage_slot: None,
        })
    }

    /// Create a `width`×`height` renderable color target with CPU readback
    /// (the test and editor-viewport carrier of the raster contract). The image
    /// is created directly in the unified `GENERAL` layout via a one-time
    /// internal submission, so a render pass may target it immediately.
    pub fn new_render_target(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "texture dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let (image, ctx) = Self::create_image(
            device,
            width,
            height,
            format,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::SAMPLED,
        )?;
        let texture = Self {
            image_view: image.view,
            image: image.image,
            allocation: Some(image.allocation),
            ctx,
            width,
            height,
            format,
            slot: None,
            storage_slot: None,
        };
        texture.transition_to_general(device)?;
        Ok(texture)
    }

    /// Transition a fresh (`UNDEFINED`) image into the unified `GENERAL`
    /// layout with a one-time command buffer, submitted and waited for
    /// before `new_render_target` returns — the target is renderable the
    /// moment it exists, with no uploader flush for the caller to order.
    fn transition_to_general(&self, device: &Device) -> Result<()> {
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let transition = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
            .src_access_mask(vk::AccessFlags2::NONE)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(
                vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            )
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        let command_buffer =
            device.command_buffer(crate::types::CommandBufferUsage::ONE_TIME_SUBMIT)?;
        command_buffer.image_barriers(std::slice::from_ref(&transition));
        device.submit_and_wait(&[&command_buffer])
    }

    /// Read the whole texture back as tightly packed bytes (RGBA order for
    /// 4-byte formats). The copy rides a one-time internal submission, so
    /// the caller's rendering must already be complete (`submit_and_wait`)
    /// when this is called.
    pub fn read_pixels(&self, device: &Device) -> Result<Vec<u8>> {
        let bytes = self.format.bytes_per_pixel();
        let readback = super::memory::GpuAllocation::new(
            device,
            (self.width as usize * self.height as usize * bytes) as u64,
            crate::types::MemoryResidency::CpuReadOnly,
        )?;

        let command_buffer =
            device.command_buffer(crate::types::CommandBufferUsage::ONE_TIME_SUBMIT)?;
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        // Order the copy after the pass's color-attachment writes (the image
        // stays in GENERAL throughout).
        let to_transfer = vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
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
                width: self.width,
                height: self.height,
                depth: 1,
            });
        // SAFETY: the image is in GENERAL (a valid transfer source) and the
        // readback allocation is exactly width*height*bytes_per_pixel.
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
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(
                vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE,
            )
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(subresource);
        command_buffer.image_barriers(std::slice::from_ref(&back));
        device.submit_and_wait(&[&command_buffer])?;

        let mut out = vec![0u8; self.width as usize * self.height as usize * bytes];
        readback.read_bytes(&mut out)?;
        Ok(out)
    }

    /// Create a compute-writable, shader-readable 2D texture (`SAMPLED |
    /// STORAGE` usage) with two descriptor-heap slots over one image view: a
    /// storage-image slot for compute `RWTexture2D` writes
    /// ([`Texture::storage_handle`]) and a sampled-image slot for shader
    /// reads ([`Texture::handle`]).
    ///
    /// The image starts `UNDEFINED`; `uploader` records its `UNDEFINED ->
    /// GENERAL` initialization transition (the unified-layout guarantee's
    /// sanctioned exception), and the caller submits with
    /// [`FrameUploader::end_frame`] before the first dispatch writes it.
    ///
    /// Fails with [`Error::Unsupported`] when the device's `storage_image`
    /// capability is `false` (see [`Device::capabilities`]).
    pub fn storage_image(
        device: &Device,
        uploader: &mut FrameUploader,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<Self> {
        if !device.capabilities().storage_image {
            return Err(Error::Unsupported(
                "this device does not support compute-writable storage images".to_string(),
            ));
        }
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "texture dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let (image, ctx) = Self::create_image(
            device,
            width,
            height,
            format,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
        )?;
        uploader.transition_image(image.image)?;
        let heap = device.descriptor_heap();
        let storage_handle = heap.alloc_image_slot()?;
        let sampled_handle = heap.alloc_image_slot()?;
        heap.write_resource_descriptors(&[
            (
                storage_handle,
                super::descriptor_heap::TextureSlotDesc::new(
                    &image.view_create_info,
                    vk::ImageLayout::GENERAL,
                )
                .storage(),
            ),
            (
                sampled_handle,
                super::descriptor_heap::TextureSlotDesc::new(
                    &image.view_create_info,
                    vk::ImageLayout::GENERAL,
                ),
            ),
        ])?;
        let view_create_info = image.view_create_info;
        Ok(Self {
            image_view: image.view,
            image: image.image,
            allocation: Some(image.allocation),
            ctx,
            width,
            height,
            format,
            slot: Some(TextureSlot {
                handle: sampled_handle,
                heap: heap.clone(),
                view_create_info,
            }),
            storage_slot: Some(TextureSlot {
                handle: storage_handle,
                heap,
                view_create_info,
            }),
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
        let (image, ctx) = Self::create_image(
            device,
            width,
            height,
            format,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        )?;
        uploader.upload_image(image.image, bytes, None, (width, height))?;
        let heap = device.descriptor_heap();
        let handle = heap.alloc_image_slot()?;
        heap.write_resource_descriptors(&[(
            handle,
            super::descriptor_heap::TextureSlotDesc::new(
                &image.view_create_info,
                vk::ImageLayout::GENERAL,
            ),
        )])?;
        Ok(Self {
            image_view: image.view,
            image: image.image,
            allocation: Some(image.allocation),
            ctx,
            width,
            height,
            format,
            slot: Some(TextureSlot {
                handle,
                heap,
                view_create_info: image.view_create_info,
            }),
            storage_slot: None,
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
    pub fn view(&self) -> super::view::TextureView {
        super::view::TextureView::borrow_raw(
            self.image_view,
            self.ctx.clone(),
            self.width,
            self.height,
            self.format,
        )
    }

    /// The `(width, height)` of the texture.
    pub fn extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The texture's format.
    pub fn format(&self) -> Format {
        self.format
    }

    /// The bindless heap slot, `None` for escape-hatch textures (e.g. the
    /// egui interop path) that do not participate in the descriptor heap.
    /// For [`Texture::storage_image`] this is the *sampled* slot; compute
    /// writes go through [`Texture::storage_handle`].
    pub fn handle(&self) -> Option<TextureHandle> {
        self.slot.as_ref().map(|slot| slot.handle)
    }

    /// The storage-image heap slot of a [`Texture::storage_image`]
    /// (compute `RWTexture2D` writes); `None` for upload-only textures.
    pub fn storage_handle(&self) -> Option<TextureHandle> {
        self.storage_slot.as_ref().map(|slot| slot.handle)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still sample the image
        // through its heap slots. Each slot action carries the view create
        // info — the heap's encoded descriptor references it by pointer, so
        // it must stay alive until the slot is freed.
        for slot in [self.slot.take(), self.storage_slot.take()]
            .into_iter()
            .flatten()
        {
            self.ctx.ring().push(RetireAction::ImageSlot {
                slots: slot.heap.image_slots(),
                handle: slot.handle,
                view_create_info: slot.view_create_info,
            });
        }
        self.ctx.ring().push(RetireAction::Image {
            device: self.ctx.raw().clone(),
            view: self.image_view,
            image: self.image,
            allocation: self.allocation.take(),
            allocator: self.ctx.allocator().clone(),
        });
    }
}
