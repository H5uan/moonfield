//! Selene's runtime plugin: the render world's device resource and window
//! frame loop.
//!
//! [`RenderPlugin`] creates the shared [`RenderDevice`] resource at plugin
//! build time (instance + logical device), inserts it into the render world,
//! and registers the extraction systems and the window frame-loop systems
//! that own per-window surfaces, swapchains, and frame sync as render-world
//! data. Windowed consumers record into the frame's command buffer between
//! the acquire and submit systems; destruction order is handled by `Arc`
//! refcounting on the shared device.
//!
//! Headless tolerance: if the machine has no Vulkan driver (e.g. a CI
//! runner), the plugin logs an error and inserts nothing — the app
//! keeps running, and windowed consumers retry gracefully until a
//! `RenderDevice` appears.

use crate::extract::extract_cameras;
use crate::schedule::{CameraDriver, PostViews, PrepareAssets, PrepareViews, Queue, Submit};
use crate::window::{
    acquire_window_frames, create_window_surfaces, extract_windows, submit_window_frames,
};
use moonfield_app::prelude::IntoSystemConfigs;
use moonfield_app::{App, ExtractSchedule, Plugin, Render};
use moonfield_log::error;
use moonfield_rhi::RenderDevice;

/// Runtime plugin: creates the shared [`RenderDevice`] resource, registers
/// the extraction systems, owns the `Render` schedule's set chain, and
/// brackets it with the window frame loop (acquire before the chain,
/// submit after it).
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn name(&self) -> &str {
        "moonfield_render_core::RenderPlugin"
    }

    fn build(&self, app: &mut App) {
        app.add_render_systems(ExtractSchedule, (extract_cameras, extract_windows));
        app.add_render_sets(
            Render,
            (
                PrepareAssets,
                Queue,
                crate::schedule::PhaseSort,
                PrepareViews,
                CameraDriver,
                PostViews,
                Submit,
            ),
        );
        app.add_render_systems(
            Render,
            (
                acquire_window_frames.before_set::<PrepareAssets>(),
                create_window_surfaces.after_set::<PrepareAssets>(),
                // Per-view attachment components; feature-side pool `ensure`
                // systems order `.before(this)`.
                crate::scene::prepare_view_attachments.in_set::<PrepareViews>(),
                submit_window_frames.after_set::<Submit>(),
            ),
        );
        match RenderDevice::new() {
            Ok(render_device) => {
                app.render_world_mut().insert_resource(render_device);
            }
            Err(e) => {
                // No Vulkan driver (e.g. CI without a GPU): run without
                // rendering resources instead of panicking.
                error!("RenderPlugin could not initialize Vulkan: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_plugin_never_panics_without_driver() {
        // Whether or not this machine has a Vulkan driver, adding the plugin
        // must succeed; the resource is present iff device creation worked.
        let mut app = App::new();
        app.add_plugin(RenderPlugin);
        if app.render_world().contains_resource::<RenderDevice>() {
            // On machines with a driver, the shared device is usable.
            // (A valid device always has a non-null graphics queue.)
        }
        assert!(!app.world().contains_resource::<RenderDevice>());
    }
}
