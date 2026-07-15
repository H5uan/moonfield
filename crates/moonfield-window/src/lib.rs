//! Abstract windowing types for Moonfield.
//!
//! This crate defines the [`Window`] resource and [`RawHandleWrapper`] that
//! other crates (render, winit, etc.) use to communicate about windows
//! without depending on a specific windowing backend.

use moonfield_app::App;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Plugin that registers the default [`Window`] resource.
///
/// Adding this plugin is optional — [`Window`] is usually created and inserted
/// by a windowing backend (e.g. `moonfield-winit`). This plugin only provides
/// a sensible default so that consumers can read the resource without a
/// hard dependency on any backend.
pub struct WindowPlugin;

/// Abstract window properties.
///
/// This resource is created and updated by a windowing backend (e.g.
/// `moonfield-winit`) and read by renderers and other systems.
#[derive(Debug, Clone)]
pub struct Window {
    /// Window title.
    pub title: String,
    /// Inner width in logical pixels.
    pub width: u32,
    /// Inner height in logical pixels.
    pub height: u32,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
        }
    }
}

/// Raw window and display handles, suitable for graphics API surface creation.
///
/// Created by a windowing backend from the platform-native window handle.
/// Renderers (e.g. `moonfield-render`) use this to create a Vulkan surface
/// without depending on any specific windowing library.
///
/// # Safety
///
/// `RawHandleWrapper` is `Send + Sync` even though the underlying
/// `raw-window-handle` types may not be, because the handles are only used
/// to create Vulkan surfaces and are never accessed concurrently in a way
/// that would cause undefined behaviour.
#[derive(Debug, Clone)]
pub struct RawHandleWrapper {
    pub window_handle: RawWindowHandle,
    pub display_handle: RawDisplayHandle,
}

// SAFETY: The handles are only passed to Vulkan surface creation and are
// never concurrently mutated in a way that would cause UB.
unsafe impl Send for RawHandleWrapper {}
unsafe impl Sync for RawHandleWrapper {}

impl moonfield_app::Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        if app.get_resource::<Window>().is_none() {
            app.insert_resource(Window::default());
        }
    }

    fn name(&self) -> &str {
        "moonfield_window::WindowPlugin"
    }
}
