//! Bevy-style plugin for the rendering crate.
//!
//! Provides [`RenderPlugin`], which creates the device-level Vulkan singletons
//! (instance + logical device) at plugin build time and inserts them into the
//! world as the shared [`RenderDevice`] resource. Window-bound objects
//! (surface, swapchain, frame sync) stay with the windowed consumer — e.g. the
//! editor's `WindowRenderer` — which borrows the shared device through
//! `Arc`s, so destruction order is handled by refcounting rather than by who
//! owns the world.
//!
//! Headless tolerance: if the machine has no Vulkan driver (CI on
//! Windows/macOS), the plugin logs an error and inserts nothing — the app
//! keeps running, and windowed consumers retry gracefully until a
//! `RenderDevice` appears.

use crate::{Device, Instance};
use moonfield_app::{App, Plugin};
use moonfield_log::{error, info, warn};
use std::ffi::CStr;
use std::sync::Arc;

/// The shared device-level Vulkan singletons: one [`Instance`] and one
/// logical [`Device`] for the whole app, owned by the world (inserted by
/// [`RenderPlugin`]).
///
/// Mirrors Bevy's `RenderDevice` (+ `RenderInstance`) as a single resource.
/// The instance is created with the platform's surface extensions, so
/// windowed consumers can create surfaces from it later; the device is
/// created without a surface (presentation support is validated per window,
/// when the surface exists).
///
/// Cloneable (cheap `Arc` clones) so windowed renderers can hold the device
/// alive independently of the resource's lifetime.
///
/// Field order matters: `device` drops before `instance` (a Vulkan instance
/// must not be destroyed while its logical devices are still alive).
#[derive(Clone)]
pub struct RenderDevice {
    device: Arc<Device>,
    instance: Arc<Instance>,
}

impl RenderDevice {
    /// Create the shared instance + device for the best available physical
    /// device (discrete GPU preferred).
    ///
    /// If an instance with the platform's surface extensions cannot be
    /// created, falls back to a headless instance (windowed rendering will
    /// fail later at surface creation, but GPU compute still works).
    pub fn new() -> crate::Result<Self> {
        let instance = match Instance::new(platform_surface_extensions()) {
            Ok(instance) => instance,
            Err(e) => {
                warn!(
                    "Vulkan instance with surface extensions unavailable ({e}); \
                     falling back to a headless instance — windowed rendering disabled"
                );
                Instance::new_headless()?
            }
        };
        let device = Device::new(&instance, None)?;

        let props = instance.physical_device_properties(device.physical_device());
        // SAFETY: device_name is a NUL-terminated C string in the props struct.
        let device_name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy();
        info!("RenderPlugin initialized Vulkan on device: {device_name}");

        Ok(Self {
            instance: Arc::new(instance),
            device: Arc::new(device),
        })
    }

    /// The shared Vulkan instance.
    pub fn instance(&self) -> &Arc<Instance> {
        &self.instance
    }

    /// The shared logical device (graphics/present queues, GPU allocator).
    pub fn device(&self) -> &Arc<Device> {
        &self.device
    }
}

/// The surface extensions a window on this platform may need, known without a
/// display handle so the shared instance can be created before any window.
fn platform_surface_extensions() -> &'static [&'static CStr] {
    #[cfg(target_os = "windows")]
    {
        &[ash::khr::surface::NAME, ash::khr::win32_surface::NAME]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            ash::khr::surface::NAME,
            ash::khr::xcb_surface::NAME,
            ash::khr::wayland_surface::NAME,
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            ash::khr::surface::NAME,
            ash::ext::metal_surface::NAME,
            ash::khr::portability_enumeration::NAME,
        ]
    }
}

/// Runtime plugin: creates the shared [`RenderDevice`] resource.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn name(&self) -> &str {
        "moonfield_render::RenderPlugin"
    }

    fn build(&self, app: &mut App) {
        match RenderDevice::new() {
            Ok(render_device) => {
                app.insert_resource(render_device);
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
        let device_usable = app
            .world()
            .get_resource::<RenderDevice>()
            .map(|render_device| render_device.device().graphics_queue() != ash::vk::Queue::null());
        if app.world().contains_resource::<RenderDevice>() {
            // On machines with a driver, the shared device is usable.
            assert_eq!(device_usable, Some(true));
        }
    }
}
