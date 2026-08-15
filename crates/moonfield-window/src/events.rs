//! Window lifecycle events and control signals (backend-agnostic).
//!
//! Distinct from [`crate::input`]: these are app-level lifecycle signals
//! (close requests, resize, focus) rather than gameplay input. Follows the
//! same frame-scoped pattern — the windowing backend (e.g. `moonfield-winit`)
//! pushes events into the [`WindowEvents`] resource as they arrive, the app
//! consumes them during the update, and the backend clears the queue after.
//!
//! Exit policy mirrors Godot's `auto_accept_quit`: by default the backend
//! exits immediately on `CloseRequested`; a caller can take over by setting
//! [`WindowControl::set_auto_exit_on_close`] to `false` and later calling
//! [`WindowControl::request_exit`] (which sets [`WindowControl::exit_requested`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use moonfield_ecs::Entity;

/// A window lifecycle event, translated from the backend's OS event.
///
/// Every variant carries the `window` [`Entity`] it fired for — the type is
/// shaped for multi-window; single-window builds always report the primary
/// window entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowEventKind {
    /// The user asked to close the window (title-bar ×, Alt-F4).
    CloseRequested { window: Entity },
    /// The window was resized (physical pixels).
    Resized {
        window: Entity,
        width: u32,
        height: u32,
    },
    /// The window's scale factor changed (DPI change, moved across
    /// monitors).
    ScaleFactorChanged { window: Entity, scale_factor: f64 },
    /// The window gained keyboard focus.
    FocusGained { window: Entity },
    /// The window lost keyboard focus.
    FocusLost { window: Entity },
}

/// Frame-scoped queue of window lifecycle events, stored as a world
/// resource by the backend and replayed to consumers during the update.
#[derive(Debug, Default)]
pub struct WindowEvents {
    events: Vec<WindowEventKind>,
}

impl WindowEvents {
    /// Queue one event.
    pub fn push(&mut self, event: WindowEventKind) {
        self.events.push(event);
    }

    /// This frame's events, in arrival order.
    pub fn events(&self) -> &[WindowEventKind] {
        &self.events
    }

    /// Clear the queue. Called by the backend once per frame, after the app
    /// update has consumed the frame's events.
    pub fn end_frame(&mut self) {
        self.events.clear();
    }
}

/// Window control signals shared between the windowing backend and other
/// callers. Cheap to clone (atomics behind an `Arc`).
#[derive(Debug, Clone)]
pub struct WindowControl {
    /// When true (the default), the backend exits the event loop
    /// immediately on `CloseRequested` — Godot's `auto_accept_quit`. A caller
    /// sets this false via [`WindowControl::set_auto_exit_on_close`] to
    /// receive `close_requested` events and decide themselves.
    pub auto_exit_on_close: Arc<AtomicBool>,
    /// Set by a caller via [`WindowControl::request_exit`]; the backend exits
    /// the event loop at the next frame boundary.
    pub exit_requested: Arc<AtomicBool>,
}

impl Default for WindowControl {
    fn default() -> Self {
        Self {
            auto_exit_on_close: Arc::new(AtomicBool::new(true)),
            exit_requested: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl WindowControl {
    /// Read `auto_exit_on_close`.
    pub fn auto_exit_on_close(&self) -> bool {
        self.auto_exit_on_close.load(Ordering::Relaxed)
    }

    /// Read `exit_requested`.
    pub fn exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::Relaxed)
    }

    /// Set `auto_exit_on_close`.
    pub fn set_auto_exit_on_close(&self, enabled: bool) {
        self.auto_exit_on_close.store(enabled, Ordering::Relaxed);
    }

    /// Set `exit_requested`.
    pub fn request_exit(&self) {
        self.exit_requested.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_queue_is_frame_scoped() {
        let mut events = WindowEvents::default();
        let window = Entity::DANGLING;
        events.push(WindowEventKind::CloseRequested { window });
        events.push(WindowEventKind::Resized {
            window,
            width: 1024,
            height: 768,
        });
        assert_eq!(events.events().len(), 2);
        events.end_frame();
        assert!(events.events().is_empty());
    }

    #[test]
    fn control_defaults_to_auto_exit() {
        let control = WindowControl::default();
        assert!(control.auto_exit_on_close());
        assert!(!control.exit_requested());
        control.set_auto_exit_on_close(false);
        assert!(!control.auto_exit_on_close());
        control.request_exit();
        assert!(control.exit_requested());
    }
}
