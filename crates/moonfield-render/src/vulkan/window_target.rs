//! Windowed rendering: swapchain frame loop.
//!
//! Provides [`WindowRenderer`], which owns the window-bound Vulkan objects —
//! surface, swapchain, per-image framebuffers, and per-frame-in-flight
//! synchronization — and drives the acquire → record → submit → present
//! cycle. The device-level singletons (instance, logical device) are shared:
//! they come from the world's [`RenderDevice`](crate::RenderDevice) resource
//! (created by [`RenderPlugin`](crate::RenderPlugin)) and are held as `Arc`s.
//! A UI renderer (the editor's egui backend) records its draw commands into the
//! frame's command buffer between [`WindowRenderer::begin_frame`] and
//! [`WindowRenderer::end_frame`].

use crate::error::{Error, Result};
use crate::types::Format;
use crate::vulkan::device::Device;
use crate::vulkan::framebuffer::Framebuffer;
use crate::vulkan::instance::Instance;
use crate::vulkan::render_pass::RenderPass;
use crate::vulkan::swapchain::{Surface, Swapchain};
use crate::{CommandBuffer, CommandPool, Fence, RenderDevice, Semaphore};
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;

/// Number of frames that may be in flight concurrently.
const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// Swapchain frame loop for a window.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: per-frame sync and command objects first, then
/// framebuffers, render pass, swapchain, and surface. The shared instance and
/// device come last as `Arc`s — their actual destruction happens when the
/// last referrer (usually the world's [`RenderDevice`] resource) drops, so
/// the swapchain and surface are always destroyed while the device and
/// instance are still alive.
pub struct WindowRenderer {
    image_available: Vec<Semaphore>,
    render_finished: Vec<Semaphore>,
    in_flight: Vec<Fence>,
    command_buffers: Vec<CommandBuffer>,
    /// Held for drop order only: the pool must outlive its command buffers.
    #[allow(dead_code)]
    command_pool: CommandPool,
    framebuffers: Vec<Framebuffer>,
    render_pass: RenderPass,
    swapchain: Swapchain,
    surface: Surface,
    device: Arc<Device>,
    instance: Arc<Instance>,
    current_frame: usize,
    current_image: Option<u32>,
    needs_recreate: bool,
}

impl WindowRenderer {
    /// Create a renderer presenting to the given window, on the shared
    /// [`RenderDevice`]'s instance and device.
    ///
    /// The shared device is created without a surface, so its graphics queue
    /// family's presentation support is validated against this window's
    /// surface here; creation fails if the device cannot present to it.
    pub fn new(
        render_device: &RenderDevice,
        window: &(impl HasWindowHandle + HasDisplayHandle),
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let instance = render_device.instance().clone();
        let device = render_device.device().clone();

        let surface = Surface::from_window(instance.entry(), &instance, window)?;
        let queue_families = device.queue_family_indices();
        if !instance.get_physical_device_surface_support(
            device.physical_device(),
            queue_families.graphics,
            surface.raw(),
        ) {
            return Err(Error::Backend(
                "the shared render device cannot present to this window's surface".to_string(),
            ));
        }
        let swapchain = Swapchain::new(&instance, &device, &surface, [width, height], None)?;
        let render_pass_format =
            Format::from_vk(swapchain.format().format).ok_or(Error::Unsupported)?;
        let render_pass = RenderPass::new(&device, render_pass_format)?;
        let framebuffers = create_framebuffers(&device, &render_pass, &swapchain)?;

        let command_pool = CommandPool::new(&device, queue_families.graphics)?;
        let mut command_buffers = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut image_available = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut in_flight = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            command_buffers.push(command_pool.allocate_command_buffer()?);
            image_available.push(Semaphore::new(&device)?);
            render_finished.push(Semaphore::new(&device)?);
            in_flight.push(Fence::new(&device, true)?);
        }

        Ok(Self {
            image_available,
            render_finished,
            in_flight,
            command_buffers,
            command_pool,
            framebuffers,
            render_pass,
            swapchain,
            surface,
            device,
            instance,
            current_frame: 0,
            current_image: None,
            needs_recreate: false,
        })
    }

    /// Begin a frame: wait for the frame-in-flight fence, acquire the next
    /// swapchain image, and begin recording the frame's command buffer.
    ///
    /// Returns `false` when the swapchain is out of date and no frame was
    /// started; call [`recreate`](Self::recreate) and try again.
    pub fn begin_frame(&mut self) -> Result<bool> {
        if self.current_image.is_some() {
            return Err(Error::Validation(
                "begin_frame called while a frame is in progress".to_string(),
            ));
        }

        let frame = self.current_frame;
        self.in_flight[frame].wait(u64::MAX)?;

        let (image_index, suboptimal) = match self
            .swapchain
            .acquire_next_image(u64::MAX, self.image_available[frame].raw())
        {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.needs_recreate = true;
                return Ok(false);
            }
            Err(e) => return Err(e.into()),
        };
        if suboptimal {
            self.needs_recreate = true;
        }

        self.in_flight[frame].reset()?;
        self.current_image = Some(image_index);

        let command_buffer = &mut self.command_buffers[frame];
        command_buffer.begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)?;
        Ok(true)
    }

    /// The command buffer recording the current frame.
    ///
    /// Panics if called outside `begin_frame`/`end_frame`.
    pub fn command_buffer(&mut self) -> &mut CommandBuffer {
        assert!(
            self.current_image.is_some(),
            "no frame in progress; call begin_frame first"
        );
        &mut self.command_buffers[self.current_frame]
    }

    /// The render pass targeting the swapchain images.
    pub fn render_pass(&self) -> &RenderPass {
        &self.render_pass
    }

    /// The framebuffer of the currently acquired swapchain image.
    ///
    /// Panics if called outside `begin_frame`/`end_frame`.
    pub fn framebuffer(&self) -> &Framebuffer {
        let image_index = self
            .current_image
            .expect("no frame in progress; call begin_frame first");
        &self.framebuffers[image_index as usize]
    }

    /// End the frame: finish recording, submit to the graphics queue, and
    /// present the acquired image.
    pub fn end_frame(&mut self) -> Result<()> {
        let image_index = self
            .current_image
            .take()
            .expect("no frame in progress; call begin_frame first");
        let frame = self.current_frame;

        self.command_buffers[frame].end()?;

        let wait_semaphores = [self.image_available[frame].raw()];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal_semaphores = [self.render_finished[frame].raw()];
        let command_buffers = [self.command_buffers[frame].raw()];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers)
            .signal_semaphores(&signal_semaphores);

        // SAFETY: the command buffer is fully recorded; the semaphores and
        // fence are valid and follow the in-flight contract.
        unsafe {
            self.device
                .raw()
                .queue_submit(
                    self.device.graphics_queue(),
                    std::slice::from_ref(&submit_info),
                    self.in_flight[frame].raw(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit frame: {:?}", e)))?;
        }

        match self.swapchain.queue_present(
            self.device.present_queue(),
            &signal_semaphores,
            image_index,
        ) {
            Ok(suboptimal) => {
                if suboptimal {
                    self.needs_recreate = true;
                }
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.needs_recreate = true;
            }
            Err(e) => return Err(e.into()),
        }

        self.current_frame = (frame + 1) % MAX_FRAMES_IN_FLIGHT;
        Ok(())
    }

    /// Whether the swapchain should be recreated (resize, suboptimal, or
    /// out-of-date was observed).
    pub fn needs_recreate(&self) -> bool {
        self.needs_recreate
    }

    /// Recreate the swapchain and its framebuffers for a new window size.
    ///
    /// Waits for the device to go idle first. Zero dimensions are ignored
    /// (e.g. a minimized window).
    pub fn recreate(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        // SAFETY: the device is valid; waiting for idle guarantees no frame
        // still references the old swapchain images.
        unsafe {
            self.device
                .raw()
                .device_wait_idle()
                .map_err(|e| Error::Backend(format!("failed to wait for device idle: {:?}", e)))?;
        }

        // Pass the current swapchain as `oldSwapchain` so the driver recycles
        // the surface's images; MoltenVK rejects creation unless the new
        // swapchain names the one currently in use by the surface. The old
        // swapchain is dropped after the new one is created and the device is
        // idle.
        let old_swapchain = self.swapchain.raw();
        self.swapchain = Swapchain::new(
            &self.instance,
            &self.device,
            &self.surface,
            [width, height],
            Some(old_swapchain),
        )?;
        self.framebuffers = create_framebuffers(&self.device, &self.render_pass, &self.swapchain)?;
        self.needs_recreate = false;
        Ok(())
    }

    /// The current swapchain extent.
    pub fn extent(&self) -> vk::Extent2D {
        self.swapchain.extent()
    }

    /// The swapchain surface format.
    pub fn format(&self) -> vk::SurfaceFormatKHR {
        self.swapchain.format()
    }

    /// Number of frames that may be in flight concurrently; per-slot GPU
    /// resources (buffers, deferred frees) key off this.
    pub fn frames_in_flight(&self) -> usize {
        MAX_FRAMES_IN_FLIGHT
    }

    /// The current frame slot (0..frames_in_flight). Per-slot GPU resources
    /// key off this so writers don't race a frame still on the GPU.
    pub fn current_frame_index(&self) -> usize {
        self.current_frame
    }

    /// Access the logical device (e.g. to hand to a UI renderer).
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Access the Vulkan instance (e.g. to hand to a UI renderer).
    pub fn instance(&self) -> &Instance {
        &self.instance
    }
}

impl Drop for WindowRenderer {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so no swapchain image or command buffer is
        // destroyed while still in use by the GPU.
        unsafe {
            let _ = self.device.raw().device_wait_idle();
        }
    }
}

fn create_framebuffers(
    device: &Device,
    render_pass: &RenderPass,
    swapchain: &Swapchain,
) -> Result<Vec<Framebuffer>> {
    swapchain
        .image_views()
        .iter()
        .map(|view| Framebuffer::new(device, render_pass, &[*view], swapchain.extent()))
        .collect()
}
