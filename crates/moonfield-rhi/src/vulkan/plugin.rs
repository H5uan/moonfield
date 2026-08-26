//! The shared device-level Vulkan singletons.
//!
//! Provides [`RenderDevice`], which pairs the instance and logical device
//! created for the best available physical device. It is a plain resource
//! type with no ECS dependency: the engine layer (`moonfield-render-core`'s
//! `RenderPlugin`) inserts it into the render world, and headless one-shot
//! consumers call [`RenderDevice::new`] directly.

use crate::{Device, Instance};
use ash::vk;
use moonfield_log::{info, warn};
use std::ffi::CStr;
use std::sync::Arc;

/// The shared device-level Vulkan singletons: one [`Instance`] and one
/// logical [`Device`] for the whole app. The engine layer's `RenderPlugin`
/// inserts this resource into the render world.
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

        let mut props = vk::PhysicalDeviceProperties2::default();
        instance.physical_device_properties2(device.physical_device(), &mut props);
        // SAFETY: device_name is a NUL-terminated C string in the props struct.
        let device_name =
            unsafe { CStr::from_ptr(props.properties.device_name.as_ptr()) }.to_string_lossy();
        info!("Lunar Mare initialized Vulkan on device: {device_name}");

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
        &[
            ash::khr::surface::NAME,
            ash::khr::win32_surface::NAME,
            ash::khr::get_surface_capabilities2::NAME,
        ]
    }
    #[cfg(target_os = "linux")]
    {
        &[
            ash::khr::surface::NAME,
            ash::khr::xlib_surface::NAME,
            ash::khr::xcb_surface::NAME,
            ash::khr::wayland_surface::NAME,
            ash::khr::get_surface_capabilities2::NAME,
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            ash::khr::surface::NAME,
            ash::ext::metal_surface::NAME,
            ash::khr::portability_enumeration::NAME,
            ash::khr::get_surface_capabilities2::NAME,
        ]
    }
}
