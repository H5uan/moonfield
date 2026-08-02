//! Windowed rendering: swapchain frame loop.
//!
//! Provides [`WindowRenderer`], which owns the full windowed Vulkan setup —
//! instance with surface extensions, surface, device, swapchain, per-image
//! framebuffers, and per-frame-in-flight synchronization — and drives the
//! acquire → record → submit → present cycle. A UI renderer (e.g.
//! egui-ash-renderer) records its draw commands into the frame's command
//! buffer between [`WindowRenderer::begin_frame`] and
//! [`WindowRenderer::end_frame`].

use crate::device::Device;
use crate::error::{Error, Result};
use crate::framebuffer::Framebuffer;
use crate::instance::Instance;
use crate::render_pass::RenderPass;
use crate::swapchain::{Surface, Swapchain};
use crate::{CommandBuffer, CommandPool, Fence, Semaphore};
use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::ffi::CStr;

/// Number of frames that may be in flight concurrently.
const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// Swapchain frame loop for a window.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: per-frame sync and command objects first, then
/// framebuffers, render pass, swapchain, device, surface, and finally the
/// instance.
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
    device: Device,
    surface: Surface,
    instance: Instance,
    current_frame: usize,
    current_image: Option<u32>,
    needs_recreate: bool,
}

impl WindowRenderer {
    /// Create a renderer presenting to the given window.
    pub fn new(
        window: &(impl HasWindowHandle + HasDisplayHandle),
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let display_handle = window
            .display_handle()
            .map_err(|e| Error::Backend(format!("failed to get display handle: {e}")))?;
        let extension_ptrs = ash_window::enumerate_required_extensions(display_handle.as_raw())
            .map_err(|e| Error::Backend(format!("failed to enumerate extensions: {e}")))?;
        // SAFETY: ash-window returns pointers to static extension name strings.
        let extensions: Vec<&CStr> = extension_ptrs
            .iter()
            .map(|ptr| unsafe { CStr::from_ptr(*ptr) })
            .collect();

        let instance = Instance::new(&extensions)?;
        let surface = Surface::from_window(instance.entry(), &instance, window)?;
        let device = Device::new(&instance, Some(surface.raw()))?;
        let swapchain = Swapchain::new(&instance, &device, &surface, [width, height])?;
        let render_pass = RenderPass::new(&device, swapchain.format().format)?;
        let framebuffers = create_framebuffers(&device, &render_pass, &swapchain)?;

        let command_pool = CommandPool::new(&device, device.queue_family_indices().graphics)?;
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
            device,
            surface,
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

        // The old swapchain is dropped after the new one is created; multiple
        // swapchains per surface are legal, and the device is idle.
        self.swapchain =
            Swapchain::new(&self.instance, &self.device, &self.surface, [width, height])?;
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
