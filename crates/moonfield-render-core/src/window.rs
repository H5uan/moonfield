//! Windowed rendering: surfaces and the swapchain frame loop as ECS data
//! plus systems.
//!
//! Bevy-style, there is no window "renderer" object. Per-frame window
//! snapshots arrive as [`ExtractedWindow`] components ([`extract_windows`]);
//! persistent per-window GPU state lives in the [`WindowSurfaces`] resource
//! keyed by the source [`MainEntity`]; the frame itself — command buffer
//! ring, timeline semaphore, slot sequencing — is the [`FrameContext`]
//! resource. The frame loop is three public systems that other plugins order
//! against:
//!
//! - [`create_window_surfaces`] (`PrepareAssets` set): creates/recreates
//!   surfaces
//!   and swapchains to match the extracted windows.
//! - [`acquire_window_frames`] (`Render`, first): begins the frame (waits the
//!   in-flight timeline counter, drains the frame slot's retirements, begins
//!   the frame's command buffer) and acquires the next swapchain image for
//!   every window with [`WindowFrameDemand`].
//! - [`submit_window_frames`] (`Render`, last): flushes the shared uploader,
//!   ends recording, submits to the graphics queue once, presents every
//!   acquired window, and advances the frame slot.
//!
//! The frame exists every `Render` tick a device exists — offscreen passes
//! need no window. Everything that records into the frame fetches the
//! in-progress command buffer from [`FrameContext`] between acquire and
//! submit. The device-level singletons stay on the shared [`RenderDevice`]
//! resource.

use crate::MainEntity;
use crate::extract::Extract;
use moonfield_app::prelude::World;
use moonfield_ecs::{Commands, Query};
use moonfield_log::error;
use moonfield_rhi::{
    CommandBuffer, CommandBufferUsage, CommandPool, DepthBuffer, Device, Error, Extent2d, Format,
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
///
/// Alias of [`moonfield_rhi::RETIRE_RING`]: the RHI owns the value so the
/// retirement ring depth and the frame loop cannot drift apart.
pub const MAX_FRAMES_IN_FLIGHT: usize = moonfield_rhi::RETIRE_RING;

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
pub fn extract_windows(windows: Extract<Query<(&Window, &RawHandleWrapper)>>, commands: Commands) {
    for (entity, (window, handle)) in windows.iter() {
        commands.spawn((ExtractedWindow {
            main_entity: MainEntity(entity),
            handle: handle.clone(),
            physical_width: window.resolution.physical_width(),
            physical_height: window.resolution.physical_height(),
        },));
    }
}

/// Pure frame-sequencing state: which frame slot to record into, which
/// timeline value to wait on before reusing a slot, and whether a frame is
/// in progress.
///
/// Extracted from [`FrameContext`] so the arithmetic is unit-testable
/// without a GPU; [`FrameContext`] interleaves the Vulkan calls between
/// these state transitions. Per-window state (the acquired image, the
/// recreate flag) lives in [`WindowSurfaceData`] as plain fields.
struct FrameSequencer {
    /// Number of the next frame to submit; the first frame is 1.
    frame_submitted: u64,
    /// A frame has been begun and not yet submitted.
    in_progress: bool,
    /// Frames submitted to the queue (present itself may still have failed).
    presented_frames: u64,
}

/// What [`FrameSequencer::plan_acquire`] computed for the next frame.
struct FramePlan {
    /// Timeline value to wait on before the slot may be reused, if the slot
    /// still has a previous submission in flight.
    wait: Option<u64>,
    /// The frame slot (0..[`MAX_FRAMES_IN_FLIGHT`]) to record into.
    slot: usize,
}

impl FrameSequencer {
    fn new() -> Self {
        Self {
            frame_submitted: 1,
            in_progress: false,
            presented_frames: 0,
        }
    }

    /// Plan the next frame: the slot to record into and the timeline value
    /// to wait on before reusing it. Returns `None` while a frame is in
    /// progress (double acquire).
    fn plan_acquire(&self) -> Option<FramePlan> {
        if self.in_progress {
            return None;
        }
        let wait = (self.frame_submitted > MAX_FRAMES_IN_FLIGHT as u64)
            .then(|| self.frame_submitted - MAX_FRAMES_IN_FLIGHT as u64);
        Some(FramePlan {
            wait,
            slot: self.current_slot(),
        })
    }

    /// The frame was begun; it is now in progress.
    fn note_begin(&mut self) {
        self.in_progress = true;
    }

    /// Take the in-progress frame for submission: its slot and the timeline
    /// value to signal (the frame number).
    ///
    /// Panics when no frame is in progress.
    fn take_for_submit(&mut self) -> (usize, u64) {
        assert!(
            self.in_progress,
            "no frame in progress; acquire must run first"
        );
        self.in_progress = false;
        (self.current_slot(), self.frame_submitted)
    }

    /// The frame's timeline was signaled: advance the counters. Runs right
    /// after the queue submission — present may still fail afterwards, but
    /// the timeline value is spent and must not be re-signaled.
    fn finish_submit(&mut self) {
        self.frame_submitted += 1;
        self.presented_frames = self.presented_frames.saturating_add(1);
    }

    /// The slot the next (or in-progress) frame records into.
    fn current_slot(&self) -> usize {
        ((self.frame_submitted - 1) % MAX_FRAMES_IN_FLIGHT as u64) as usize
    }

    fn frame_in_progress(&self) -> bool {
        self.in_progress
    }

    fn presented_frames(&self) -> u64 {
        self.presented_frames
    }
}

/// The frame as a first-class object: the frame-level GPU objects (command
/// pool, per-slot command buffer ring, timeline semaphore) plus the frame
/// sequencing state.
///
/// A render-world resource created lazily by [`acquire_window_frames`] once
/// a [`RenderDevice`] exists. Windows are acquire/present targets of the
/// frame, not its owner: [`WindowSurfaceData`] keeps only per-window state.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: command objects and the timeline first, the Arc'd
/// device last — its actual destruction happens when the last referrer
/// (usually the world's [`RenderDevice`] resource) drops, so the command
/// pool and semaphore are always destroyed while the device is still alive.
pub struct FrameContext {
    command_buffers: Vec<CommandBuffer>,
    /// Held for drop order only: the pool must outlive its command buffers.
    #[allow(dead_code)]
    command_pool: CommandPool,
    timeline: Semaphore,
    device: Arc<Device>,
    /// Frame sequencing state (slot, timeline values); plain data, no
    /// drop-order relevance.
    sequencer: FrameSequencer,
}

impl FrameContext {
    /// Create the frame context on the shared device's graphics queue family.
    fn new(device: &Arc<Device>) -> Result<Self> {
        let queue_families = device.queue_family_indices();
        let command_pool = CommandPool::new(device, queue_families.graphics)?;
        let mut command_buffers = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            command_buffers.push(command_pool.allocate_command_buffer()?);
        }
        let timeline = Semaphore::new_timeline(device, 0)?;
        Ok(Self {
            command_buffers,
            command_pool,
            timeline,
            device: device.clone(),
            sequencer: FrameSequencer::new(),
        })
    }

    /// Begin the frame: wait for the in-flight timeline counter
    /// (`frame_submitted - MAX_FRAMES_IN_FLIGHT`), drain the frame slot's
    /// retirements, and begin recording the slot's command buffer. Runs every
    /// `Render` tick a device exists, with or without windows; returns the
    /// frame slot.
    fn begin_frame(&mut self) -> Result<usize> {
        let Some(plan) = self.sequencer.plan_acquire() else {
            return Err(Error::Validation(
                "frame begun while a frame is still in progress".to_string(),
            ));
        };

        if let Some(wait) = plan.wait {
            self.timeline.wait(wait, u64::MAX)?;
        }
        // The wait above guarantees this slot's previous submission
        // completed, so its retirements are safe to run; the slot also
        // becomes the ring's push target for this frame's drops. Once per
        // frame, for the whole device.
        self.device.begin_gpu_frame(plan.slot);
        let command_buffer = &mut self.command_buffers[plan.slot];
        command_buffer.begin(CommandBufferUsage::ONE_TIME_SUBMIT)?;
        // Bind the descriptor heaps once per frame command buffer (heap
        // binding is command-buffer scoped): every heap-indexed shader
        // access in the frame reads them. Direct command-buffer owners
        // (tests) bind their own.
        self.device.descriptor_heap().cmd_bind(command_buffer)?;
        self.sequencer.note_begin();
        Ok(plan.slot)
    }

    /// End the frame: finish recording and submit the slot's command buffer
    /// to the graphics queue, waiting on `waits` (the acquired windows'
    /// `image_available`), signaling `signals` (their `render_finished`) plus
    /// the timeline with the frame number. An offscreen-only frame passes
    /// empty slices.
    fn end_frame(&mut self, waits: &[&Semaphore], signals: &[&Semaphore]) -> Result<()> {
        let (slot, signal) = self.sequencer.take_for_submit();
        self.command_buffers[slot].end()?;
        self.device.submit_frame_timeline(
            &self.command_buffers[slot],
            waits,
            signals,
            &self.timeline,
            signal,
        )?;
        // The timeline is now signaled, so the frame is submitted no matter
        // how present turns out: advance the counters here, or the next
        // submit would re-signal the same timeline value (illegal for
        // timeline semaphores).
        self.sequencer.finish_submit();
        Ok(())
    }

    /// Whether a frame has been begun and is open for recording.
    pub fn frame_in_progress(&self) -> bool {
        self.sequencer.frame_in_progress()
    }

    /// The current frame slot (0..[`MAX_FRAMES_IN_FLIGHT`]). Per-slot GPU
    /// resources key off this so writers don't race a frame still on the GPU.
    pub fn current_slot(&self) -> usize {
        self.sequencer.current_slot()
    }

    /// The command buffer recording the current frame, if a frame is in
    /// progress (between [`acquire_window_frames`] and
    /// [`submit_window_frames`]).
    pub fn current_command_buffer(&self) -> Option<&CommandBuffer> {
        if !self.sequencer.frame_in_progress() {
            return None;
        }
        Some(&self.command_buffers[self.sequencer.current_slot()])
    }

    /// Frames submitted to the graphics queue (present itself may still have
    /// failed).
    pub fn presented_frames(&self) -> u64 {
        self.sequencer.presented_frames()
    }
}

impl Drop for FrameContext {
    fn drop(&mut self) {
        // Best-effort wait so no command buffer is destroyed while still in
        // use by the GPU.
        let _ = self.device.wait_idle();
    }
}

/// Persistent GPU state for one window: surface, swapchain, per-frame-in-flight
/// present synchronization, and the per-window acquire state. The frame-level
/// objects (command buffers, timeline, sequencing) live in [`FrameContext`].
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: the present semaphores first, then the swapchain and
/// surface. The shared instance and device come last as `Arc`s — their actual
/// destruction happens when the last referrer (usually the world's
/// [`RenderDevice`] resource) drops, so the swapchain and surface are always
/// destroyed while the device and instance are still alive.
pub struct WindowSurfaceData {
    image_available: Vec<Semaphore>,
    render_finished: Vec<Semaphore>,
    swapchain: Swapchain,
    /// Per-window reverse-Z depth attachment, sized to the swapchain and
    /// resized with it (the window-targeted 3D pass draws depth-tested).
    depth: Option<DepthBuffer>,
    surface: Surface,
    device: Arc<Device>,
    instance: Arc<Instance>,
    /// The acquired swapchain image of the in-progress frame, if any.
    current_image: Option<u32>,
    /// The swapchain reported itself out of date (or suboptimal). Plain
    /// data, no drop-order relevance.
    needs_recreate: bool,
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

        let surface = Surface::from_window(&instance, window)?;
        if !instance.supports_present(&device, &surface) {
            return Err(Error::Backend(
                "the shared render device cannot present to this window's surface".to_string(),
            ));
        }
        let swapchain = Swapchain::new(
            &instance,
            &device,
            &surface,
            [window.physical_width, window.physical_height],
        )?;
        // The window-targeted 3D pass draws depth-tested; the depth buffer
        // tracks the swapchain extent (resized in `recreate`).
        let depth = DepthBuffer::new(&device, window.physical_width, window.physical_height)?;

        let mut image_available = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            image_available.push(Semaphore::new(&device)?);
            render_finished.push(Semaphore::new(&device)?);
        }

        Ok(Self {
            image_available,
            render_finished,
            swapchain,
            depth: Some(depth),
            surface,
            device,
            instance,
            current_image: None,
            needs_recreate: false,
        })
    }

    /// Acquire the next swapchain image for the in-progress frame, signaling
    /// this window's `image_available[slot]`.
    ///
    /// Returns `false` when the swapchain is out of date and no image was
    /// acquired; the surface is flagged for recreation on the next
    /// [`create_window_surfaces`] run.
    fn acquire_image(&mut self, slot: usize) -> Result<bool> {
        match self
            .swapchain
            .acquire_next_image(u64::MAX, &self.image_available[slot])
        {
            Ok((image_index, suboptimal)) => {
                self.current_image = Some(image_index);
                self.needs_recreate |= suboptimal;
                Ok(true)
            }
            Err(Error::SurfaceOutOfDate) => {
                self.needs_recreate = true;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Present the acquired image, waiting on this window's
    /// `render_finished[slot]`. Suboptimal and out-of-date results flag the
    /// surface for recreation.
    fn present(&mut self, slot: usize) -> Result<()> {
        let Some(image_index) = self.current_image.take() else {
            return Ok(());
        };
        let render_finished = [&self.render_finished[slot]];
        match self
            .swapchain
            .queue_present(&self.device, &render_finished, image_index)
        {
            Ok(suboptimal) => {
                self.needs_recreate |= suboptimal;
            }
            Err(Error::SurfaceOutOfDate) => {
                self.needs_recreate = true;
            }
            Err(e) => return Err(e),
        }
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

        // The device is idle; the swapchain passes its own current handle as
        // `oldSwapchain` so the driver recycles the surface's images.
        self.swapchain
            .recreate(&self.instance, &self.device, &self.surface, [width, height])?;
        if let Some(depth) = &mut self.depth {
            depth.resize(&self.device, width, height)?;
        }
        self.needs_recreate = false;
        Ok(())
    }

    /// The current swapchain extent.
    pub fn extent(&self) -> Extent2d {
        self.swapchain.extent()
    }

    /// The swapchain color format in the crate's vocabulary, plus whether the
    /// framebuffer is sRGB-encoded (an sRGB target needs the UI shader's
    /// linearizing fragment entry).
    pub fn format(&self) -> Result<(Format, bool)> {
        self.swapchain.format_srgb()
    }

    /// Whether this window acquired an image for the in-progress frame.
    pub fn frame_in_progress(&self) -> bool {
        self.current_image.is_some()
    }

    /// The image view of the currently acquired swapchain image, for use as
    /// the color attachment of a [`moonfield_rhi::RenderPassDesc`]. `None` when no
    /// frame is in progress. The returned view borrows the swapchain's; it
    /// must not outlive the surface data.
    pub fn current_image_view(&self) -> Option<TextureView> {
        let image_index = self.current_image?;
        Some(self.swapchain.image_view(image_index))
    }

    /// Borrow the window's depth attachment view, for the depth attachment of
    /// a [`moonfield_rhi::RenderPassDesc`]. The view borrows the depth
    /// buffer's; it must not outlive the surface data.
    pub fn depth_view(&self) -> Option<TextureView> {
        self.depth.as_ref().map(|depth| depth.view())
    }
}

impl Drop for WindowSurfaceData {
    fn drop(&mut self) {
        // Best-effort wait so no swapchain image is destroyed while still in
        // use by the GPU.
        let _ = self.device.wait_idle();
    }
}

/// Render-world flag: a consumer has window content to present this tick
/// (e.g. a window-targeted camera, or the editor's UI pass). Demand gates
/// swapchain acquire/present only — the frame itself ([`FrameContext`])
/// exists every `Render` tick a device exists. Written by extraction each
/// frame.
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

/// `PrepareAssets` set system: create or recreate surface data to match the
/// extracted windows, and drop surface data whose window disappeared.
///
/// No-ops when no [`RenderDevice`] exists (headless machines without a
/// Vulkan driver).
pub fn create_window_surfaces(world: &mut World) {
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|device| (*device).clone())
    else {
        return;
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
                if (data.needs_recreate
                    || (extent.width != window.physical_width
                        || extent.height != window.physical_height))
                    && let Err(e) = data.recreate(window.physical_width, window.physical_height)
                {
                    error!("failed to recreate window surface: {e}");
                }
            }
        }
    }
}

/// `Render` system (ordering anchor; pass systems run `.after()` it): begin
/// the frame, then acquire the next swapchain image for every window with
/// [`WindowFrameDemand`]. The frame begins every tick a [`RenderDevice`]
/// exists — offscreen passes record into it whether or not any window
/// acquired. Windows whose swapchain is out of date skip this frame and are
/// recreated by the next [`create_window_surfaces`] run; minimized
/// (zero-size) windows are skipped entirely.
pub fn acquire_window_frames(world: &mut World) {
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|device| (*device).clone())
    else {
        return;
    };
    if !world.contains_resource::<FrameContext>() {
        match FrameContext::new(render_device.device()) {
            Ok(frame) => world.insert_resource(frame),
            Err(e) => {
                error!("failed to create frame context: {e}");
                return;
            }
        }
    }
    let slot = {
        let mut frame = world
            .get_resource_mut::<FrameContext>()
            .expect("FrameContext was just ensured");
        match frame.begin_frame() {
            Ok(slot) => slot,
            Err(e) => {
                error!("failed to begin frame: {e}");
                return;
            }
        }
    };
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
        match data.acquire_image(slot) {
            Ok(true) => {}
            Ok(false) => {} // out of date; recreated next frame
            Err(e) => error!("failed to acquire window frame: {e}"),
        }
    }
}

/// `Render` system (ordering anchor; pass systems run `.before()` it): end
/// recording, submit the frame's command buffer once — waiting on every
/// acquired window's `image_available`, signaling every acquired window's
/// `render_finished` plus the frame timeline — and present each acquired
/// window. A frame with no acquired window (offscreen-only) submits
/// timeline-only.
pub fn submit_window_frames(world: &mut World) {
    // Flush uploads recorded during this frame's preparation (texture
    // deltas, image transitions) ahead of the frame command buffers:
    // same-queue submission order executes them first. Idempotent — a
    // frame with no uploads submits nothing.
    if let Some(render_device) = world.get_resource::<RenderDevice>()
        && let Err(e) = render_device
            .device()
            .uploader()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .end_frame()
    {
        error!("failed to flush frame uploads: {e}");
    }
    let Some(mut frame) = world.get_resource_mut::<FrameContext>() else {
        return;
    };
    if !frame.frame_in_progress() {
        return;
    }
    let slot = frame.current_slot();
    let mut surfaces = world.get_resource_mut::<WindowSurfaces>();
    let (waits, signals): (Vec<&Semaphore>, Vec<&Semaphore>) = surfaces
        .as_deref()
        .map(|surfaces| {
            surfaces
                .surfaces
                .values()
                .filter(|data| data.frame_in_progress())
                .map(|data| (&data.image_available[slot], &data.render_finished[slot]))
                .unzip()
        })
        .unwrap_or_default();
    if let Err(e) = frame.end_frame(&waits, &signals) {
        error!("failed to submit frame: {e}");
        return;
    }
    let Some(surfaces) = surfaces.as_deref_mut() else {
        return;
    };
    for data in surfaces.surfaces.values_mut() {
        if !data.frame_in_progress() {
            continue;
        }
        if let Err(e) = data.present(slot) {
            error!("failed to present window frame: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive one full acquire → submit cycle at the sequencer level,
    /// returning (slot, wait, signal) as observed by the Vulkan layer.
    fn run_frame(seq: &mut FrameSequencer) -> (usize, Option<u64>, u64) {
        let plan = seq.plan_acquire().expect("no frame in progress");
        seq.note_begin();
        let (slot, signal) = seq.take_for_submit();
        seq.finish_submit();
        (slot, plan.wait, signal)
    }

    #[test]
    fn test_first_frames_fill_the_ring_without_waiting() {
        let mut seq = FrameSequencer::new();
        for (expected_slot, expected_signal) in [(0, 1), (1, 2)] {
            let (slot, wait, signal) = run_frame(&mut seq);
            assert_eq!((slot, wait, signal), (expected_slot, None, expected_signal));
        }
    }

    #[test]
    fn test_wait_catches_up_with_the_ring_depth() {
        let mut seq = FrameSequencer::new();
        run_frame(&mut seq);
        run_frame(&mut seq);
        // Frame 3 reuses slot 0: wait for frame 1's timeline value.
        let (slot, wait, signal) = run_frame(&mut seq);
        assert_eq!((slot, wait, signal), (0, Some(1), 3));
        // Frame 4 reuses slot 1: wait for frame 2.
        let (slot, wait, signal) = run_frame(&mut seq);
        assert_eq!((slot, wait, signal), (1, Some(2), 4));
        // And the ring keeps cycling.
        let (slot, wait, _) = run_frame(&mut seq);
        assert_eq!((slot, wait), (0, Some(3)));
    }

    #[test]
    fn test_double_acquire_is_rejected() {
        let mut seq = FrameSequencer::new();
        assert!(seq.plan_acquire().is_some());
        seq.note_begin();
        assert!(seq.plan_acquire().is_none());
        assert!(seq.frame_in_progress());
    }

    #[test]
    #[should_panic(expected = "no frame in progress")]
    fn test_submit_without_acquire_panics() {
        let mut seq = FrameSequencer::new();
        seq.take_for_submit();
    }

    #[test]
    fn test_slot_and_progress_track_the_cycle() {
        let mut seq = FrameSequencer::new();
        assert_eq!(seq.current_slot(), 0);
        assert!(!seq.frame_in_progress());

        seq.plan_acquire().expect("no frame in progress");
        seq.note_begin();
        assert!(seq.frame_in_progress());

        let (slot, signal) = seq.take_for_submit();
        assert_eq!((slot, signal), (0, 1));
        assert!(!seq.frame_in_progress());

        seq.finish_submit();
        assert_eq!(seq.current_slot(), 1);
        assert_eq!(seq.presented_frames(), 1);
    }
}
