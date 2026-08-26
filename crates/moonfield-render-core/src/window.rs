//! Windowed rendering: surfaces and the swapchain frame loop as ECS data
//! plus systems.
//!
//! Bevy-style, there is no window "renderer" object. Per-frame window
//! snapshots arrive as [`ExtractedWindow`] components ([`extract_windows`]);
//! persistent per-window GPU state lives in the [`WindowSurfaces`] resource
//! keyed by the source [`MainEntity`]; the frame loop is three public systems
//! that other plugins order against:
//!
//! - [`create_window_surfaces`] (`RenderPrepare`): creates/recreates surfaces
//!   and swapchains to match the extracted windows.
//! - [`acquire_window_frames`] (`Render`, first): waits the in-flight fence,
//!   acquires the next swapchain image, and begins the frame's command buffer.
//! - [`submit_window_frames`] (`Render`, last): ends recording, submits to the
//!   graphics queue, presents, and advances the frame slot.
//!
//! Everything that records into a window frame fetches the in-progress
//! command buffer from [`WindowSurfaces`] between acquire and submit. The
//! device-level singletons stay on the shared [`RenderDevice`] resource.

use crate::MainEntity;
use moonfield_app::prelude::World;
use moonfield_log::error;
use moonfield_rhi::{
    CommandBuffer, CommandBufferUsage, CommandPool, Device, Error, Extent2d, Fence, Format,
    Instance, RenderDevice, Result, Semaphore, Surface, Swapchain, TextureView,
};
use moonfield_window::{RawHandleWrapper, Window};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Number of frames that may be in flight concurrently. Per-slot GPU
/// resources (buffers, deferred frees) key off the frame slot index.
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;

/// Per-frame snapshot of a main-world window, extracted into the render world.
///
/// Render-world entities are rebuilt every frame, so this component is pure
/// data; the persistent surface/swapchain it drives lives in
/// [`WindowSurfaces`], keyed by [`ExtractedWindow::main_entity`].
pub struct ExtractedWindow {
    /// Source window entity in the main world.
    pub main_entity: MainEntity,
    /// Raw handles for surface creation.
    pub handle: RawHandleWrapper,
    /// Physical size in pixels (what the swapchain reports).
    pub physical_width: u32,
    /// Physical height in pixels.
    pub physical_height: u32,
}

impl HasWindowHandle for ExtractedWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle was captured from a live OS window owned by the
        // windowing backend; surfaces created from it are dropped (via
        // `WindowSurfaces`) before the window itself.
        Ok(unsafe { WindowHandle::borrow_raw(self.handle.window_handle) })
    }
}

impl HasDisplayHandle for ExtractedWindow {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: see `window_handle`.
        Ok(unsafe { DisplayHandle::borrow_raw(self.handle.display_handle) })
    }
}

/// Copy every main-world window (`Window` + `RawHandleWrapper`) into the
/// render world as an [`ExtractedWindow`] component.
pub fn extract_windows(world: &World, render_world: &mut World) {
    for (entity, (window, handle)) in world.query::<(&Window, &RawHandleWrapper)>() {
        render_world.spawn((ExtractedWindow {
            main_entity: MainEntity(entity),
            handle: handle.clone(),
            physical_width: window.resolution.physical_width(),
            physical_height: window.resolution.physical_height(),
        },));
    }
}

/// Persistent GPU state for one window: surface, swapchain, per-frame-in-flight
/// synchronization, and command buffers.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: per-frame sync and command objects first, then the
/// swapchain and surface. The shared instance and device come last as `Arc`s —
/// their actual destruction happens when the last referrer (usually the
/// world's [`RenderDevice`] resource) drops, so the swapchain and surface are
/// always destroyed while the device and instance are still alive.
pub struct WindowSurfaceData {
    image_available: Vec<Semaphore>,
    render_finished: Vec<Semaphore>,
    in_flight: Vec<Fence>,
    command_buffers: Vec<CommandBuffer>,
    /// Held for drop order only: the pool must outlive its command buffers.
    #[allow(dead_code)]
    command_pool: CommandPool,
    swapchain: Swapchain,
    surface: Surface,
    device: Arc<Device>,
    instance: Arc<Instance>,
    current_frame: usize,
    current_image: Option<u32>,
    needs_recreate: bool,
    /// Frames successfully presented; consumers (e.g. the editor's feedback
    /// channel) read this to count rendered frames.
    presented_frames: u64,
}

impl WindowSurfaceData {
    /// Create the surface and swapchain for an extracted window, on the
    /// shared [`RenderDevice`]'s instance and device.
    ///
    /// The shared device is created without a surface, so its graphics queue
    /// family's presentation support is validated against this window's
    /// surface here; creation fails if the device cannot present to it.
    fn new(render_device: &RenderDevice, window: &ExtractedWindow) -> Result<Self> {
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
        let swapchain = Swapchain::new(
            &instance,
            &device,
            &surface,
            [window.physical_width, window.physical_height],
            None,
        )?;

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
            swapchain,
            surface,
            device,
            instance,
            current_frame: 0,
            current_image: None,
            needs_recreate: false,
            presented_frames: 0,
        })
    }

    /// Begin a frame: wait for the frame-in-flight fence, acquire the next
    /// swapchain image, and begin recording the frame's command buffer.
    ///
    /// Returns `false` when the swapchain is out of date and no frame was
    /// started; the surface is flagged for recreation on the next
    /// [`create_window_surfaces`] run.
    fn acquire(&mut self) -> Result<bool> {
        if self.current_image.is_some() {
            return Err(Error::Validation(
                "acquire called while a frame is in progress".to_string(),
            ));
        }

        let frame = self.current_frame;
        self.in_flight[frame].wait(u64::MAX)?;

        let (image_index, suboptimal) = match self
            .swapchain
            .acquire_next_image(u64::MAX, &self.image_available[frame])
        {
            Ok(result) => result,
            Err(Error::SurfaceOutOfDate) => {
                self.needs_recreate = true;
                return Ok(false);
            }
            Err(e) => return Err(e),
        };
        if suboptimal {
            self.needs_recreate = true;
        }

        self.in_flight[frame].reset()?;
        self.current_image = Some(image_index);

        let command_buffer = &mut self.command_buffers[frame];
        command_buffer.begin(CommandBufferUsage::ONE_TIME_SUBMIT)?;
        Ok(true)
    }

    /// End the frame: finish recording, submit to the graphics queue, and
    /// present the acquired image.
    fn submit(&mut self) -> Result<()> {
        let image_index = self
            .current_image
            .take()
            .expect("no frame in progress; acquire must run first");
        let frame = self.current_frame;

        self.command_buffers[frame].end()?;

        self.device.submit_frame(
            &self.command_buffers[frame],
            &self.image_available[frame],
            &self.render_finished[frame],
            &self.in_flight[frame],
        )?;

        let render_finished = [&self.render_finished[frame]];
        match self
            .swapchain
            .queue_present(&self.device, &render_finished, image_index)
        {
            Ok(suboptimal) => {
                if suboptimal {
                    self.needs_recreate = true;
                }
            }
            Err(Error::SurfaceOutOfDate) => {
                self.needs_recreate = true;
            }
            Err(e) => return Err(e),
        }

        self.current_frame = (frame + 1) % MAX_FRAMES_IN_FLIGHT;
        self.presented_frames = self.presented_frames.saturating_add(1);
        Ok(())
    }

    /// Recreate the swapchain for a new window size.
    ///
    /// Waits for the device to go idle first. Zero dimensions are ignored
    /// (e.g. a minimized window).
    fn recreate(&mut self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        self.device.wait_idle()?;

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
        self.needs_recreate = false;
        Ok(())
    }

    /// The current swapchain extent.
    pub fn extent(&self) -> Extent2d {
        let extent = self.swapchain.extent();
        Extent2d {
            width: extent.width,
            height: extent.height,
        }
    }

    /// The swapchain color format in the crate's vocabulary, plus whether the
    /// framebuffer is sRGB-encoded (an sRGB target needs the UI shader's
    /// linearizing fragment entry).
    pub fn format(&self) -> Result<(Format, bool)> {
        self.swapchain.format_srgb()
    }

    /// The current frame slot (0..[`MAX_FRAMES_IN_FLIGHT`]). Per-slot GPU
    /// resources key off this so writers don't race a frame still on the GPU.
    pub fn frame_index(&self) -> usize {
        self.current_frame
    }

    /// Frames successfully presented on this surface.
    pub fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    /// Whether a frame has been acquired and is open for recording.
    pub fn frame_in_progress(&self) -> bool {
        self.current_image.is_some()
    }

    /// The command buffer recording the current frame, if a frame is in
    /// progress (between [`acquire_window_frames`] and
    /// [`submit_window_frames`]).
    pub fn current_command_buffer(&mut self) -> Option<&mut CommandBuffer> {
        self.current_image?;
        Some(&mut self.command_buffers[self.current_frame])
    }

    /// The image view of the currently acquired swapchain image, for use as
    /// the color attachment of a [`moonfield_rhi::RenderPassDesc`]. `None` when no
    /// frame is in progress. The returned view borrows the swapchain's; it
    /// must not outlive the surface data.
    pub fn current_image_view(&self) -> Option<TextureView> {
        let image_index = self.current_image?;
        Some(TextureView::borrow_raw(
            self.swapchain.image_views()[image_index as usize],
            self.device.raw().clone(),
        ))
    }
}

impl Drop for WindowSurfaceData {
    fn drop(&mut self) {
        // Best-effort wait so no swapchain image or command buffer is
        // destroyed while still in use by the GPU.
        let _ = self.device.wait_idle();
    }
}

/// Render-world flag: a consumer has frame content to present this tick
/// (e.g. the editor's UI pass). Window frames are acquired only when demand
/// is set — presenting an image nothing recorded into is undefined content.
/// Written by extraction each frame.
#[derive(Default)]
pub struct WindowFrameDemand(pub bool);

/// Persistent window GPU state, keyed by the main-world window entity.
///
/// A resource in the render world (entities there are rebuilt every frame,
/// so surfaces cannot live on them). Single-window apps observe exactly one
/// entry; the map shape is the multi-window upgrade path.
#[derive(Default)]
pub struct WindowSurfaces {
    surfaces: HashMap<MainEntity, WindowSurfaceData>,
}

impl WindowSurfaces {
    /// The surface data for a main-world window entity, if created.
    pub fn get(&self, window: MainEntity) -> Option<&WindowSurfaceData> {
        self.surfaces.get(&window)
    }

    /// Mutable access to a window's surface data.
    pub fn get_mut(&mut self, window: MainEntity) -> Option<&mut WindowSurfaceData> {
        self.surfaces.get_mut(&window)
    }

    /// Iterate all live surface data (e.g. to record into every window frame).
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut WindowSurfaceData> {
        self.surfaces.values_mut()
    }
}

/// `RenderPrepare` system: create or recreate surface data to match the
/// extracted windows, and drop surface data whose window disappeared.
///
/// No-ops when no [`RenderDevice`] exists (headless machines without a
/// Vulkan driver).
pub fn create_window_surfaces(world: &mut World) {
    let render_device = match world.get_resource::<RenderDevice>() {
        Some(device) => device.clone(),
        None => return,
    };

    let windows: Vec<ExtractedWindow> = world
        .query::<&ExtractedWindow>()
        .map(|(_, window)| ExtractedWindow {
            main_entity: window.main_entity,
            handle: window.handle.clone(),
            physical_width: window.physical_width,
            physical_height: window.physical_height,
        })
        .collect();
    if !world.contains_resource::<WindowSurfaces>() {
        world.insert_resource(WindowSurfaces::default());
    }
    let mut surfaces = world
        .get_resource_mut::<WindowSurfaces>()
        .expect("WindowSurfaces was just inserted");

    surfaces
        .surfaces
        .retain(|entity, _| windows.iter().any(|w| &w.main_entity == entity));

    for window in &windows {
        match surfaces.surfaces.entry(window.main_entity) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                if window.physical_width == 0 || window.physical_height == 0 {
                    continue;
                }
                match WindowSurfaceData::new(&render_device, window) {
                    Ok(data) => {
                        entry.insert(data);
                    }
                    Err(e) => error!("failed to create window surface: {e}"),
                }
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let data = entry.get_mut();
                let extent = data.extent();
                if data.needs_recreate
                    || (extent.width != window.physical_width
                        || extent.height != window.physical_height)
                {
                    if let Err(e) = data.recreate(window.physical_width, window.physical_height) {
                        error!("failed to recreate window surface: {e}");
                    }
                }
            }
        }
    }
}

/// `Render` system (ordering anchor; pass systems run `.after()` it): acquire
/// the next swapchain image for every window and begin its command buffer.
/// Windows whose swapchain is out of date skip this frame and are recreated
/// by the next [`create_window_surfaces`] run; minimized (zero-size) windows
/// and frames with no [`WindowFrameDemand`] are skipped entirely.
pub fn acquire_window_frames(world: &mut World) {
    if !world
        .get_resource::<WindowFrameDemand>()
        .is_some_and(|demand| demand.0)
    {
        return;
    }
    let zero_size: Vec<MainEntity> = world
        .query::<&ExtractedWindow>()
        .filter(|(_, window)| window.physical_width == 0 || window.physical_height == 0)
        .map(|(_, window)| window.main_entity)
        .collect();
    let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
        return;
    };
    for (entity, data) in surfaces.surfaces.iter_mut() {
        if zero_size.contains(entity) {
            continue;
        }
        if data.frame_in_progress() {
            error!("window frame acquired while a frame is still in progress");
            continue;
        }
        match data.acquire() {
            Ok(true) => {}
            Ok(false) => {} // out of date; recreated next frame
            Err(e) => error!("failed to acquire window frame: {e}"),
        }
    }
}

/// `Render` system (ordering anchor; pass systems run `.before()` it): end
/// recording, submit, and present every window frame that was acquired.
pub fn submit_window_frames(world: &mut World) {
    let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
        return;
    };
    for data in surfaces.values_mut() {
        if !data.frame_in_progress() {
            continue;
        }
        if let Err(e) = data.submit() {
            error!("failed to submit window frame: {e}");
        }
    }
}
