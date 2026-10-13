//! Mouse button and scroll types, backend-agnostic.
//!
//! Mirrors `bevy_input::mouse` (which mirrors winit) so the backend
//! converter is a mechanical 1:1 match.

/// A mouse button.
#[derive(Debug, Hash, Ord, PartialOrd, PartialEq, Eq, Clone, Copy)]
pub enum MouseButton {
    /// The left mouse button.
    Left,
    /// The right mouse button.
    Right,
    /// The middle mouse button.
    Middle,
    /// The back mouse button.
    Back,
    /// The forward mouse button.
    Forward,
    /// Another mouse button, identified by a platform-specific number.
    Other(u16),
}

/// The unit of a mouse scroll delta.
///
/// The value of a scroll event can be interpreted either as lines or as
/// pixels, depending on what the backend / OS reported. Precision touchpads
/// report pixel deltas; classic wheel mice report line deltas. Consumers
/// that need one uniform unit convert with
/// [`MOUSE_SCROLL_PIXELS_PER_LINE`](crate::input::MOUSE_SCROLL_PIXELS_PER_LINE).
#[derive(Debug, Hash, Clone, Copy, Eq, PartialEq)]
pub enum MouseScrollUnit {
    /// The scroll delta is expressed in number of lines.
    Line,
    /// The scroll delta is expressed in number of pixels.
    Pixel,
}
