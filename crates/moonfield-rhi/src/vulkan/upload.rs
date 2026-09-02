use ash::vk;

use crate::error::{Error, Result};
use crate::vulkan::memory::Memory;
use crate::{Buffer, CommandBufferUsage};
use crate::{CommandBuffer, CommandPool, GpuBumpAllocator, Semaphore, vulkan::Device};
pub const UPLOAD_FRAME_RING: usize = 2;

pub const UPLOAD_ARENA_SIZE: u64 = 4 * 1024 * 1024;

pub struct FrameUploader {
    // Owned resources pulled from `Device` at construction so the uploader
    // is lifetime-free: it can live in ECS resources or behind an `Arc` and
    // outlive the `&Device` that created it.
    device: ash::Device,
    queue: vk::Queue,
    arenas: Vec<GpuBumpAllocator>,
    // One command buffer per slot, like the arenas: re-recording a
    // ONE_TIME_SUBMIT buffer that is still executing is undefined, so the
    // per-slot buffer is only reset after `wait(next_frame - RING)` — the
    // same signal that frees the slot's arena.
    cb: Vec<CommandBuffer>,
    // Drop order matters: command buffers must be freed before their pool is
    // destroyed — field order is Rust's drop order (unlike locals, which
    // drop in reverse declaration order). `CommandBuffer::drop` calls
    // vkFreeCommandBuffers; `CommandPool::drop` destroys the pool and
    // implicitly frees any still-live buffers, so a pool-first order would
    // free the command buffers twice.
    /// Held for drop order only: the pool must outlive its command buffers.
    #[allow(dead_code)]
    pool: CommandPool,
    timeline: Semaphore,
    next_frame: u64,
    recording: bool,
}

impl FrameUploader {
    pub fn new(device: &Device, arena_size: u64) -> Result<Self> {
        let arena_size = arena_size.max(256);
        let mut arenas = Vec::with_capacity(UPLOAD_FRAME_RING);
        for _ in 0..UPLOAD_FRAME_RING {
            arenas.push(GpuBumpAllocator::new(device, arena_size)?);
        }
        let pool = CommandPool::new(device, device.queue_family_indices().graphics)?;
        let mut cb = Vec::with_capacity(UPLOAD_FRAME_RING);
        for _ in 0..UPLOAD_FRAME_RING {
            cb.push(pool.allocate_command_buffer()?);
        }
        let timeline = Semaphore::new_timeline(device, 0)?;
        Ok(Self {
            device: device.raw().clone(),
            queue: device.graphics_queue(),
            arenas,
            cb,
            pool,
            timeline,
            next_frame: 1,
            recording: false,
        })
    }

    pub fn begin_frame(&mut self) -> Result<()> {
        // Idempotent: a frame boundary may call this once, or each texture
        // delta may begin it lazily.
        if self.recording {
            return Ok(());
        }
        if self.next_frame > UPLOAD_FRAME_RING as u64 {
            self.timeline
                .wait(self.next_frame - UPLOAD_FRAME_RING as u64, u64::MAX)?;
        }

        let slot = ((self.next_frame - 1) % UPLOAD_FRAME_RING as u64) as usize;
        self.arenas[slot].free_all();
        self.cb[slot].begin(CommandBufferUsage::ONE_TIME_SUBMIT)?;
        self.recording = true;
        Ok(())
    }

    pub fn upload<T: Copy>(&mut self, dst: &Buffer, data: &[T]) -> Result<()> {
        if dst.memory() != Memory::Gpu {
            return Err(Error::Validation(
              "FrameUploader stages into Memory::Gpu buffers; host-visible targets are written directly".into(),
          ));
        }
        let bytes = std::mem::size_of_val(data) as u64;
        if bytes > dst.size() {
            return Err(Error::Validation("upload data exceeds buffer size".into()));
        }
        let slot = ((self.next_frame - 1) % UPLOAD_FRAME_RING as u64) as usize;
        let mem = self.arenas[slot].alloc(bytes as usize, 16)?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr() as *const u8,
                mem.cpu.as_ptr(),
                bytes as usize,
            );
            let copy = vk::BufferCopy::default()
                .src_offset(mem.src_offset)
                .dst_offset(0)
                .size(bytes);

            self.device
                .cmd_copy_buffer(self.cb[slot].raw(), mem.src, dst.raw(), &[copy]);
        }
        Ok(())
    }

    /// Upload RGBA8 pixels (`bytes.len()` == `region.0 * region.1 * 4`) into
    /// `image`, leaving it in a shader-readable layout. `offset: None`
    /// uploads a fresh (`UNDEFINED`) image; `Some((x, y))` updates a
    /// sub-region of a shader-readable one — layout transitions mirror
    /// `Texture::upload`'s contract.
    pub fn upload_image(
        &mut self,
        image: vk::Image,
        bytes: &[u8],
        offset: Option<(i32, i32)>,
        region: (u32, u32),
    ) -> Result<()> {
        self.begin_frame()?;
        let slot = ((self.next_frame - 1) % UPLOAD_FRAME_RING as u64) as usize;
        let mem = self.arenas[slot].alloc(bytes.len(), 16)?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mem.cpu.as_ptr(), bytes.len());
        }

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
            .image(image)
            .subresource_range(subresource);
        self.cb[slot].pipeline_barrier(
            src_stage,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );

        let (x, y) = offset.unwrap_or((0, 0));
        let copy_region = vk::BufferImageCopy::default()
            .buffer_offset(mem.src_offset)
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
        unsafe {
            self.device.cmd_copy_buffer_to_image(
                self.cb[slot].raw(),
                mem.src,
                image,
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
            .image(image)
            .subresource_range(subresource);
        // The image is shader-readable for every stage, not just fragment:
        // bindless sampling happens from compute (and future mesh) stages too.
        self.cb[slot].pipeline_barrier(
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_shader_read],
        );
        Ok(())
    }

    pub fn end_frame(&mut self) -> Result<()> {
        // Idempotent: an empty frame (no uploads) submits nothing.
        if !self.recording {
            return Ok(());
        }
        let slot = ((self.next_frame - 1) % UPLOAD_FRAME_RING as u64) as usize;
        self.cb[slot].end()?;
        let command_infos =
            [vk::CommandBufferSubmitInfo::default().command_buffer(self.cb[slot].raw())];
        let signal_infos = [vk::SemaphoreSubmitInfo::default()
            .semaphore(self.timeline.raw())
            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .value(self.next_frame)];
        let submit_info = vk::SubmitInfo2::default()
            .command_buffer_infos(&command_infos)
            .signal_semaphore_infos(&signal_infos);
        unsafe {
            self.device.queue_submit2(
                self.queue,
                std::slice::from_ref(&submit_info),
                vk::Fence::null(),
            )?;
        }
        self.next_frame += 1;
        self.recording = false;
        Ok(())
    }

    pub fn wait_idle(&mut self) -> Result<()> {
        if self.next_frame > 1 {
            self.timeline.wait(self.next_frame - 1, u64::MAX)?;
        }
        Ok(())
    }

    pub fn upload_and_wait<T: Copy>(&mut self, dst: &Buffer, data: &[T]) -> Result<()> {
        self.begin_frame()?;
        self.upload(dst, data)?;
        self.end_frame()?;
        self.wait_idle()
    }
}
