//! Window components (backend-agnostic).
//!
//! A window is an ECS entity carrying a [`Window`] component; the primary
//! window additionally carries the [`PrimaryWindow`] marker. A windowing
//! backend (e.g. `moonfield-winit`) owns the OS window: it creates the
//! entity when the event loop resumes, writes OS-side changes (resize, DPI)
//! back into the component, and applies component-side changes (title,
//! cursor mode) to the OS window at the frame boundary.

use crate::CursorMode;

/// Abstract window properties, as an ECS component.
///
/// Written by both sides: the backend updates [`Window::resolution`] when
/// the OS reports resize / DPI changes; gameplay and editor code may mutate
/// any field, and the backend applies the diff to the OS window once per
/// frame (CachedWindow-style field comparison, Bevy's `changed_windows`
/// pattern).
#[derive(Debug, Clone)]
pub struct Window {
    /// Window title.
    pub title: String,
    /// Physical size and scale factor.
    pub resolution: WindowResolution,
    /// Cursor visibility / grab mode.
    pub cursor_mode: CursorMode,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            title: "Moonfield".to_string(),
            resolution: WindowResolution::new(800, 600, 1.0),
            cursor_mode: CursorMode::default(),
        }
    }
}

/// Physical size and scale factor of a window.
///
/// Sizes are stored in **physical** pixels (what the OS / GPU swapchain
/// reports); logical sizes divide by [`WindowResolution::scale_factor`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowResolution {
    physical_width: u32,
    physical_height: u32,
    scale_factor: f64,
}

impl WindowResolution {
    /// Create a resolution from physical size and scale factor.
    pub fn new(physical_width: u32, physical_height: u32, scale_factor: f64) -> Self {
        Self {
            physical_width,
            physical_height,
            scale_factor,
        }
    }

    /// Physical width in pixels.
    pub fn physical_width(&self) -> u32 {
        self.physical_width
    }

    /// Physical height in pixels.
    pub fn physical_height(&self) -> u32 {
        self.physical_height
    }

    /// Logical width (physical / scale factor).
    pub fn width(&self) -> f32 {
        self.physical_width as f32 / self.scale_factor as f32
    }

    /// Logical height (physical / scale factor).
    pub fn height(&self) -> f32 {
        self.physical_height as f32 / self.scale_factor as f32
    }

    /// The window's scale factor (DPI ratio).
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Set the physical size (backend, on OS resize).
    pub fn set_physical(&mut self, width: u32, height: u32) {
        self.physical_width = width;
        self.physical_height = height;
    }

    /// Set the scale factor (backend, on DPI change).
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
    }
}

/// Marker component for the primary window entity.
///
/// Single-window builds have exactly one entity with this marker. The type
/// is shaped for the multi-window architecture (any number of [`Window`]
/// entities, exactly one primary), but the current backend ignores
/// additional window entities.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrimaryWindow;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_size_divides_by_scale_factor() {
        let mut res = WindowResolution::new(1600, 1200, 2.0);
        assert_eq!(res.width(), 800.0);
        assert_eq!(res.height(), 600.0);
        res.set_scale_factor(1.0);
        assert_eq!(res.width(), 1600.0);
        res.set_physical(1920, 1080);
        assert_eq!(res.physical_width(), 1920);
        assert_eq!(res.physical_height(), 1080);
    }
}
