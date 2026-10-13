//! Render-world camera snapshots and resolved view targets.

use crate::MainEntity;
use moonfield_camera::{Camera, RenderTarget, view_matrix};
use moonfield_ecs::{Entity, World};
use moonfield_log::error;
use moonfield_math::{GlobalTransform, Mat4};
use moonfield_rhi::{
    AttachmentLayout, ClearValue, Format, LoadOp, OffscreenTarget, RenderAttachment, RenderDevice,
    StoreOp,
};
use std::collections::HashMap;

/// Render-world camera snapshot linked to its source entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtractedView {
    /// Main-world camera entity.
    pub main_entity: MainEntity,
    /// Projection and clear settings copied from the camera.
    pub camera: Camera,
    /// Camera transform copied after transform propagation.
    pub world_from_view: GlobalTransform,
    /// Logical destination selected for the camera.
    pub target: ViewTarget,
}

impl ExtractedView {
    /// Projection multiplied by the inverse camera transform for a target
    /// aspect ratio.
    pub fn clip_from_world(&self, aspect: f32) -> Mat4 {
        self.camera.projection_matrix(aspect) * view_matrix(&self.world_from_view)
    }

    /// The logical target selected by this view.
    pub fn target(&self) -> ViewTarget {
        self.target
    }
}

/// Render-world target selected for an [`ExtractedView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewTarget(pub RenderTarget);

/// One depth-tested offscreen target per viewport camera, keyed by the
/// camera's [`MainEntity`] (stable across frames, unlike render-world view
/// entities which rebuild every frame), as a render-world resource.
/// Reverse-Z: depth clear is 0.0 with `GREATER_OR_EQUAL`.
#[derive(Default)]
pub struct ViewTargets {
    targets: HashMap<MainEntity, OffscreenTarget>,
}

impl ViewTargets {
    /// The target for a camera's main entity, if created.
    pub fn get(&self, camera: MainEntity) -> Option<&OffscreenTarget> {
        self.targets.get(&camera)
    }

    /// Create `target` at `width` x `height`, or resize it when its extent
    /// changed. Zero dimensions are ignored (e.g. a hidden editor viewport).
    pub fn ensure(
        &mut self,
        camera: MainEntity,
        width: u32,
        height: u32,
        format: Format,
        render_device: &RenderDevice,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        match self.targets.entry(camera) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                match OffscreenTarget::new_with_depth(render_device.device(), width, height, format)
                {
                    Ok(target) => {
                        entry.insert(target);
                    }
                    Err(e) => error!("failed to create view target: {e}"),
                }
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().extent() != (width, height)
                    && let Err(e) = entry
                        .get_mut()
                        .resize(render_device.device(), width, height)
                {
                    error!("failed to resize view target: {e}");
                }
            }
        }
    }

    /// Drop the targets of cameras with no view this frame.
    pub fn retain_cameras(&mut self, claimed: impl Fn(MainEntity) -> bool) {
        self.targets.retain(|camera, _| claimed(*camera));
    }
}

/// Physical sizes requested for logical render targets, written by consumers
/// each frame (the editor writes its viewport camera's entry from the panel
/// size). Keyed by the requesting camera's [`MainEntity`].
#[derive(Default)]
pub struct RenderTargetSizes(pub HashMap<MainEntity, (u32, u32)>);

/// The per-frame GPU attachments resolved for one view, as a view-entity
/// component (the redesign's "attachments are view-entity components over
/// persistent maps": the allocations live in [`ViewTargets`] and
/// `WindowSurfaces`, this record is the linkage). Built by
/// [`prepare_view_attachments`]; the pass reads it instead of matching on
/// logical targets.
#[derive(Clone)]
pub struct ViewAttachments {
    /// Color attachment record: view, layout, load/store, clear value.
    pub color: RenderAttachment,
    /// Depth attachment record, when the pass is depth-tested.
    pub depth: Option<RenderAttachment>,
    /// The target's `(width, height)`.
    pub extent: (u32, u32),
    /// The color format — the pipeline specialization key.
    pub color_format: Format,
}

/// `PrepareViews` system: resolve every extracted view's logical target into
/// a [`ViewAttachments`] component.
///
/// Viewport views draw into their camera's pooled offscreen target — always
/// cleared, nothing else renders into it. Window views share the window's
/// swapchain image and depth buffer, so the first camera in camera-order
/// clears and the rest load — the composite semantics for multiple cameras on
/// one target. Views whose target cannot be resolved (no acquired image, no
/// pooled target) get no component, and their pass no-ops.
///
/// Ordering: the offscreen pool's `ensure` (render-feature) must run first in
/// `PrepareViews`; feature plugins order their ensure `.before(this)`.
pub fn prepare_view_attachments(world: &mut World) {
    // (order, entity bits, entity, main, target, clear color) in the camera
    // driver's sort order, so "first camera per target" is deterministic.
    let mut views: Vec<(f32, u64, Entity, MainEntity, ViewTarget, [f32; 4])> = world
        .query::<&ExtractedView>()
        .map(|(entity, view)| {
            (
                view.camera.order,
                entity.to_bits().get(),
                entity,
                view.main_entity,
                view.target,
                view.camera.clear_color,
            )
        })
        .collect();
    views.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    if !world.contains_resource::<ViewTargets>() {
        world.insert_resource(ViewTargets::default());
    }

    // Resolve the window surface into an owned record before mutating
    // components: (color view, depth view, extent, color format), all `None`
    // when the window has no acquired image or an unreadable format.
    let window = world
        .get_resource::<crate::window::WindowSurfaces>()
        .and_then(|surfaces| {
            let data = surfaces.primary()?;
            let (format, _) = data.format().ok()?;
            Some((
                data.current_image_view()?,
                data.depth_view(),
                (data.extent().width, data.extent().height),
                format,
            ))
        });
    let targets = world.get_resource::<ViewTargets>();

    let mut resolved: Vec<(Entity, ViewAttachments)> = Vec::new();

    // Window views: the first camera clears, the rest composite over it;
    // depth survives across the passes only when there is more than one.
    let window_views: Vec<_> = views
        .iter()
        .filter(|(_, _, _, _, target, _)| target.0 == RenderTarget::PrimaryWindow)
        .collect();
    if let (Some((color, depth, extent, format)), true) = (window, !window_views.is_empty()) {
        for (index, (_, _, entity, _, _, clear_color)) in window_views.iter().enumerate() {
            let load = if index == 0 {
                LoadOp::Clear
            } else {
                LoadOp::Load
            };
            resolved.push((
                *entity,
                ViewAttachments {
                    color: RenderAttachment {
                        view: color.clone(),
                        layout: AttachmentLayout::Present,
                        load,
                        store: StoreOp::Store,
                        clear: ClearValue::Color(*clear_color),
                    },
                    depth: depth.clone().map(|view| RenderAttachment {
                        view,
                        layout: AttachmentLayout::DepthStencil,
                        load,
                        store: if window_views.len() > 1 {
                            StoreOp::Store
                        } else {
                            StoreOp::Discard
                        },
                        clear: ClearValue::DepthStencil {
                            depth: 0.0,
                            stencil: 0,
                        },
                    }),
                    extent,
                    color_format: format,
                },
            ));
        }
    }

    // Viewport views: each camera's own pooled target, always cleared.
    for (_, _, entity, main, _, clear_color) in
        views.iter().filter(|v| v.4.0 == RenderTarget::Viewport)
    {
        let Some(offscreen) = targets.as_ref().and_then(|targets| targets.get(*main)) else {
            continue;
        };
        resolved.push((
            *entity,
            ViewAttachments {
                color: RenderAttachment {
                    view: offscreen.view(),
                    layout: AttachmentLayout::ShaderRead,
                    load: LoadOp::Clear,
                    store: StoreOp::Store,
                    clear: ClearValue::Color(*clear_color),
                },
                depth: offscreen.depth_view().map(|view| RenderAttachment {
                    view,
                    layout: AttachmentLayout::DepthStencil,
                    load: LoadOp::Clear,
                    store: StoreOp::Discard,
                    clear: ClearValue::DepthStencil {
                        depth: 0.0,
                        stencil: 0,
                    },
                }),
                extent: offscreen.extent(),
                color_format: offscreen.format(),
            },
        ));
    }
    drop(targets);

    // Retire the pooled targets of cameras with no viewport view this frame.
    if let Some(mut targets) = world.get_resource_mut::<ViewTargets>() {
        targets.retain_cameras(|main| {
            views.iter().any(|(_, _, _, view_main, target, _)| {
                *view_main == main && target.0 == RenderTarget::Viewport
            })
        });
    }

    for (entity, attachments) in resolved {
        world.insert_component(entity, attachments);
    }
}
