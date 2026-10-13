//! Selene — the moonfield render engine layer.
//!
//! The engine framework between the Lunar Mare RHI (`moonfield-rhi`) and the
//! feature crates (`moonfield-render-feature`): per-frame extraction
//! ([`extract`]), camera snapshots and view targets ([`scene`]), the window
//! frame loop ([`window`]), and the [`RenderPlugin`] that wires them into the
//! app's render world. Low-level Vulkan objects stay in `moonfield-rhi`.

pub mod arena;
pub mod context;
pub mod extract;
pub mod plugin;
pub mod render_phase;
pub mod scene;
pub mod schedule;
pub mod view;
pub mod window;

pub use arena::{FrameDrawArena, begin_frame_draw_arena};
pub use context::{ComputeRecording, RecordingState, RenderContext, TrackedRenderPass};
pub use extract::{Extract, MainEntity, extract_cameras, extract_with_transform};
pub use plugin::RenderPlugin;
pub use render_phase::{
    DrawFunction, DrawFunctionId, DrawFunctions, OrderedFloat, PhaseItem, RenderCommand,
    RenderCommandState, RenderPhase, SortedPhasePlugin, prepare_phase, sort_phase,
};
pub use scene::{
    ExtractedView, RenderTargetSizes, ViewAttachments, ViewTarget, ViewTargets,
    prepare_view_attachments,
};
pub use view::{CurrentView, camera_driver};
pub use window::{
    ExtractedWindow, FrameContext, MAX_FRAMES_IN_FLIGHT, WindowFrameDemand, WindowSurfaceData,
    WindowSurfaces, acquire_window_frames, create_window_surfaces, extract_windows,
    submit_window_frames,
};
