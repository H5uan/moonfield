use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::error::{Error, Result};
use crate::{vulkan::Device, CommandBuffer, CommandPool, GpuBumpAllocator, Semaphore};
use crate::{Buffer, CommandBufferUsage};
pub const UPLOAD_FRAME_RING: usize = 2;

pub const UPLOAD_ARENA_SIZE: u64 = 4 * 1024 * 1024;

pub struct FrameUploader<'a> {
    device: &'a Device,
    arenas: Vec<GpuBumpAllocator<'a>>,
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

impl<'a> FrameUploader<'a> {
    pub fn new(device: &'a Device, arena_size: u64) -> Result<Self> {
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
            device,
            arenas,
            cb,
            pool,
            timeline,
            next_frame: 1,
            recording: false,
        })
    }

    pub fn begin_frame(&mut self) -> Result<()> {
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
        if dst.location() != MemoryLocation::GpuOnly {
            return Err(Error::Validation(
              "FrameUploader stages into GpuOnly buffers; host-visible targets are written directly".into(),
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
                .raw()
                .cmd_copy_buffer(self.cb[slot].raw(), mem.src, dst.raw(), &[copy]);
        }
        Ok(())
    }

    pub fn end_frame(&mut self) -> Result<()> {
        if !self.recording {
            return Err(Error::Validation("end_frame without begin_frame".into()));
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
            self.device.raw().queue_submit2(
                self.device.graphics_queue(),
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
