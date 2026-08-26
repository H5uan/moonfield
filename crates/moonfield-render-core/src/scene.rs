//! Render-world camera snapshots and resolved view targets.

use moonfield_camera::{view_matrix, Camera, RenderTarget};
use moonfield_log::error;
use moonfield_math::{GlobalTransform, Mat4};
use moonfield_rhi::{Format, OffscreenTarget, RenderDevice};

use crate::MainEntity;
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

/// One depth-tested offscreen target per logical [`RenderTarget`], as a
/// render-world resource (Bevy's `ViewTargetAttachments` counterpart).
/// Reverse-Z: depth clear is 0.0 with `GREATER_OR_EQUAL`.
#[derive(Default)]
pub struct ViewTargets {
    targets: HashMap<RenderTarget, OffscreenTarget>,
}

impl ViewTargets {
    /// The target for a logical render target, if created.
    pub fn get(&self, target: RenderTarget) -> Option<&OffscreenTarget> {
        self.targets.get(&target)
    }

    /// Mutable access to a target (e.g. the editor resizing its viewport).
    pub fn get_mut(&mut self, target: RenderTarget) -> Option<&mut OffscreenTarget> {
        self.targets.get_mut(&target)
    }

    /// Iterate `(logical target, attachment)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&RenderTarget, &OffscreenTarget)> {
        self.targets.iter()
    }

    /// Create `target` at `width` x `height`, or resize it when its extent
    /// changed. Zero dimensions are ignored (e.g. a hidden editor viewport).
    pub fn ensure(
        &mut self,
        target: RenderTarget,
        width: u32,
        height: u32,
        format: Format,
        render_device: &RenderDevice,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        match self.targets.entry(target) {
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
                if entry.get().extent() != (width, height) {
                    if let Err(e) = entry
                        .get_mut()
                        .resize(render_device.device(), width, height)
                    {
                        error!("failed to resize view target: {e}");
                    }
                }
            }
        }
    }
}
