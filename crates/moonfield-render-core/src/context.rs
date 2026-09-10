//! The frame's recording surface: typed doors over the frame command buffer.
//!
//! Systems never touch the raw `CommandBuffer`. [`RenderContext`] — a
//! [`SystemParam`] for function systems, and directly constructible inside an
//! exclusive system via [`RenderContext::get`] — exposes three doors:
//!
//! - [`RenderContext::begin_rendering`] returns a [`TrackedRenderPass`], the
//!   raster recording surface (skips redundant state binds);
//! - [`RenderContext::compute`] returns a [`ComputeRecording`], which
//!   separates consecutive dispatches with automatic `COMPUTE → COMPUTE`
//!   memory barriers;
//! - [`RenderContext::barrier`] records a manual barrier as given.
//!
//! A stage state machine ([`RecordingState`], reset when the frame begins)
//! inserts the rhi's global, resource-less barriers automatically on door
//! switches — the one sync rule every pass shares. A manual `barrier` marks
//! the hazard handled, so the next door switch records nothing.

use crate::window::FrameContext;
use moonfield_ecs::{SystemParam, World};
use moonfield_rhi::{
    BarrierHazard, BlendMode, CommandBuffer, ComputePipeline, CullState, DepthState, GpuPtr,
    Rect2d, RenderPassDesc, Stage, Viewport,
};
use std::cell::{Ref, RefMut};

/// A raster-pass recording surface over the frame command buffer that skips
/// redundant state binds. The tracking resets every time
/// [`TrackedRenderPass::begin_rendering`] runs; pipelines are immutable
/// resources while a pass records (rebuilds happen in `PrepareViews`), so a
/// pipeline's raw Vulkan handle identifies it for the tracking window.
pub struct TrackedRenderPass<'a> {
    command_buffer: &'a CommandBuffer,
    graphics_pipeline: Option<u64>,
    viewport: Option<Viewport>,
    depth_state: Option<DepthState>,
    cull_state: Option<CullState>,
}

impl<'a> TrackedRenderPass<'a> {
    /// Wrap `command_buffer` with fresh (empty) tracking.
    pub fn new(command_buffer: &'a CommandBuffer) -> Self {
        Self {
            command_buffer,
            graphics_pipeline: None,
            viewport: None,
            depth_state: None,
            cull_state: None,
        }
    }

    /// Begin a render pass; resets the bind tracking.
    pub fn begin_rendering(&mut self, desc: &RenderPassDesc) {
        self.graphics_pipeline = None;
        self.viewport = None;
        self.depth_state = None;
        self.cull_state = None;
        self.command_buffer.begin_rendering(desc);
    }

    /// End the render pass.
    pub fn end_rendering(&self) {
        self.command_buffer.end_rendering();
    }

    /// Set the viewport, skipping the call when it is already set.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if self.viewport == Some(viewport) {
            return;
        }
        self.viewport = Some(viewport);
        self.command_buffer.set_viewport(viewport);
    }

    /// Set the scissor rectangle (recording-surface passthrough; UI passes
    /// change it per draw).
    pub fn set_scissor(&self, scissor: Rect2d) {
        self.command_buffer.set_scissor(scissor);
    }

    /// Set the depth state, skipping the call when it is already set.
    pub fn set_depth_state(&mut self, state: DepthState) {
        if self.depth_state == Some(state) {
            return;
        }
        self.depth_state = Some(state);
        self.command_buffer.set_depth_state(state);
    }

    /// Set the cull state, skipping the call when it is already set.
    pub fn set_cull_state(&mut self, state: CullState) {
        if self.cull_state == Some(state) {
            return;
        }
        self.cull_state = Some(state);
        self.command_buffer.set_cull_state(state);
    }

    /// Set the color blend state (recording-surface passthrough; UI passes
    /// blend, scene passes do not).
    pub fn set_blend_state(&self, blend: BlendMode) {
        self.command_buffer.set_blend_state(blend);
    }

    /// Bind `pipeline`, skipping the bind when it is already bound.
    pub fn set_graphics_pipeline(&mut self, pipeline: &moonfield_rhi::GraphicsPipeline) {
        let key = pipeline.id();
        if self.graphics_pipeline == Some(key) {
            return;
        }
        self.command_buffer.bind_graphics_pipeline(pipeline);
        self.graphics_pipeline = Some(key);
    }

    /// Push root data at `offset` (recording-surface passthrough).
    pub fn push_data(&self, offset: u32, data: &[u8]) {
        self.command_buffer.push_data(offset, data);
    }

    /// Record a non-indexed draw (recording-surface passthrough).
    pub fn draw(
        &self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        self.command_buffer
            .draw(vertex_count, instance_count, first_vertex, first_instance);
    }
}

/// A compute recording surface over the frame command buffer. Consecutive
/// [`ComputeRecording::dispatch`] calls are separated by automatic
/// `COMPUTE → COMPUTE` memory barriers — the read-after-write chain every
/// compute pass has; passes needing nothing coarser than that record no
/// barriers of their own.
pub struct ComputeRecording<'a> {
    command_buffer: &'a CommandBuffer,
    dispatches: u32,
}

impl<'a> ComputeRecording<'a> {
    /// Wrap `command_buffer` with a fresh (empty) dispatch count. Public for
    /// tests that own their command buffer; frame systems go through
    /// [`RenderContext::compute`].
    pub fn new(command_buffer: &'a CommandBuffer) -> Self {
        Self {
            command_buffer,
            dispatches: 0,
        }
    }

    /// Bind a compute pipeline (recording-surface passthrough).
    pub fn bind_pipeline(&self, pipeline: &ComputePipeline) {
        self.command_buffer.bind_compute_pipeline(pipeline);
    }

    /// Push the two bindless root pointers for the next dispatch.
    pub fn set_bindless_root(&self, input: GpuPtr, output: GpuPtr) {
        self.command_buffer.set_bindless_root(input, output);
    }

    /// Push root data at `offset` (recording-surface passthrough).
    pub fn push_data(&self, offset: u32, data: &[u8]) {
        self.command_buffer.push_data(offset, data);
    }

    /// Launch a compute kernel. When a dispatch was already recorded into
    /// this surface, a `COMPUTE → COMPUTE` memory barrier is inserted first.
    pub fn dispatch(&mut self, x: u32, y: u32, z: u32) {
        if self.dispatches > 0 {
            self.command_buffer
                .barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
        }
        self.dispatches += 1;
        self.command_buffer.dispatch(x, y, z);
    }
}

/// The door-switch barrier machine's phase.
#[derive(Default, PartialEq)]
enum RecPhase {
    #[default]
    Idle,
    Rendering,
    Compute,
}

/// Render-world resource: the recording state machine, advanced by
/// [`RenderContext`]'s doors. The frame sequencer resets it to idle when a
/// frame begins.
#[derive(Default)]
pub struct RecordingState {
    phase: RecPhase,
}

/// The frame's typed recording doors; see the [module docs](self).
pub struct RenderContext<'a> {
    frame: Option<Ref<'a, FrameContext>>,
    state: Option<RefMut<'a, RecordingState>>,
}

impl<'a> RenderContext<'a> {
    /// Build the context from the render world: the in-progress frame's
    /// command buffer and the recording state. Both doors read as absent when
    /// the underlying resource is missing (no device, frame not begun), so a
    /// pass system no-ops on headless machines and outside the frame.
    pub fn get(world: &'a World) -> Self {
        Self {
            frame: world.get_resource::<FrameContext>(),
            state: world.get_resource_mut::<RecordingState>(),
        }
    }

    /// The raster door: switch-barrier, then begin the render pass.
    /// Returns `None` when there is nothing to record into.
    pub fn begin_rendering(&mut self, desc: &RenderPassDesc) -> Option<TrackedRenderPass<'_>> {
        // Field-disjoint borrows: the command buffer lives in `self.frame`,
        // the machine phase in `self.state`.
        let command_buffer = self.frame.as_ref()?.current_command_buffer()?;
        let state = self.state.as_mut()?;
        match state.phase {
            RecPhase::Idle => {}
            // No `COLOR_ATTACHMENT_OUTPUT` constant exists in `Stage`, so
            // raster-involving switches use the broadest stage pair: the
            // conservative over-synchronization this design accepts.
            RecPhase::Rendering | RecPhase::Compute => {
                command_buffer.barrier(Stage::ALL, Stage::ALL, BarrierHazard::Memory);
            }
        }
        state.phase = RecPhase::Rendering;
        let mut pass = TrackedRenderPass::new(command_buffer);
        pass.begin_rendering(desc);
        Some(pass)
    }

    /// The compute door: switch-barrier, then open a dispatch surface.
    /// Returns `None` when there is nothing to record into.
    pub fn compute(&mut self) -> Option<ComputeRecording<'_>> {
        let command_buffer = self.frame.as_ref()?.current_command_buffer()?;
        let state = self.state.as_mut()?;
        match state.phase {
            RecPhase::Idle => {}
            RecPhase::Rendering => {
                command_buffer.barrier(Stage::ALL, Stage::COMPUTE, BarrierHazard::Memory);
            }
            RecPhase::Compute => {
                command_buffer.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
            }
        }
        state.phase = RecPhase::Compute;
        Some(ComputeRecording::new(command_buffer))
    }

    /// The manual door: record the barrier as given. The machine marks the
    /// hazard handled, so the next door switch records nothing extra.
    pub fn barrier(&mut self, before: Stage, after: Stage, hazard: BarrierHazard) {
        let Some(command_buffer) = self
            .frame
            .as_ref()
            .and_then(|frame| frame.current_command_buffer())
        else {
            return;
        };
        command_buffer.barrier(before, after, hazard);
        if let Some(state) = self.state.as_mut() {
            state.phase = RecPhase::Idle;
        }
    }
}

impl SystemParam for RenderContext<'_> {
    type State = ();
    type Item<'w, 's> = RenderContext<'w>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        RenderContext::get(world)
    }
}
