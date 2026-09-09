//! The `Render` schedule's set chain: the public ordering anchors plugins
//! attach to. [`RenderPlugin`](crate::RenderPlugin) registers the chain;
//! [`RenderPlugin`]'s window frame systems (acquire, submit) sit outside it
//! — acquire before [`PrepareAssets`], submit after [`Submit`].

use moonfield_ecs::SystemSet;

/// GPU asset preparation: meshes, shaders, window surfaces.
pub struct PrepareAssets;
/// Per-view phase components are filled.
pub struct Queue;
/// Every phase is sorted.
pub struct PhaseSort;
/// Per-view resources: pipelines, view-target attachments, per-frame arenas.
pub struct PrepareViews;
/// The camera driver runs each view's schedule.
pub struct CameraDriver;
/// Post-view work on the frame's targets (UI overlays).
pub struct PostViews;
/// The frame is submitted and presented.
pub struct Submit;

impl SystemSet for PrepareAssets {}
impl SystemSet for Queue {}
impl SystemSet for PhaseSort {}
impl SystemSet for PrepareViews {}
impl SystemSet for CameraDriver {}
impl SystemSet for PostViews {}
impl SystemSet for Submit {}
