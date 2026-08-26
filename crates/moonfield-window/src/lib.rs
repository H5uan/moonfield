//! Abstract windowing types for Moonfield.
//!
//! A window is an ECS entity carrying the [`Window`] component (plus the
//! [`PrimaryWindow`] marker on the primary window and a
//! [`RawHandleWrapper`] for graphics-API surface creation), so other crates
//! (render, winit, editor, etc.) can communicate about windows without
//! depending on a specific windowing backend. This crate additionally
//! provides the backend-agnostic [`InputState`] resource / [`InputEvent`]
//! types, the strongly-typed [`KeyCode`] / [`MouseButton`] mirror enums, and
//! the [`WindowEventKind`] lifecycle events (delivered through the
//! `Messages<WindowEventKind>` channel).

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

pub mod events;
pub mod input;
pub mod keyboard;
pub mod modifiers;
pub mod mouse;
pub mod window;

pub use events::{WindowControl, WindowEventKind};
pub use input::{CursorMode, InputEvent, InputState, MOUSE_SCROLL_PIXELS_PER_LINE};
pub use keyboard::{KeyCode, NativeKeyCode};
pub use modifiers::Modifiers;
pub use mouse::{MouseButton, MouseScrollUnit};
pub use window::{PrimaryWindow, Window, WindowResolution};

/// Raw window and display handles, suitable for graphics API surface creation.
///
/// Attached as a **component** to the window entity by the windowing
/// backend (e.g. `moonfield-winit`). Renderers (e.g. `moonfield-rhi`)
/// use this to create a Vulkan surface without depending on any specific
/// windowing library.
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
