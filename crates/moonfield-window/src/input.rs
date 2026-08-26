//! Backend-agnostic input state and event types.
//!
//! A windowing backend (e.g. `moonfield-winit`) translates raw OS events
//! into [`InputEvent`]s and applies them to the [`InputState`] resource as
//! they arrive; once per frame (after the app update has consumed them) it
//! calls [`InputState::end_frame`]. Consumers — ECS systems — read the
//! resource during the update.
//!
//! The model follows Bevy's `ButtonInput` contract: pressed state persists
//! across frames, while `just_pressed`/`just_released` edges, the event
//! queue, and the mouse accumulators are frame-scoped. Keys and buttons are
//! strongly typed ([`KeyCode`] / [`MouseButton`]), mirroring
//! `bevy_input`'s enums.

use std::collections::HashSet;

use crate::{KeyCode, Modifiers, MouseButton, MouseScrollUnit};

/// Approximate number of pixels per line when converting
/// [`MouseScrollUnit::Pixel`] deltas into [`MouseScrollUnit::Line`] deltas.
///
/// Matches Bevy's `MouseScrollPixelsPerLine` default (100.0), itself a best
/// guess for Microsoft Edge; platform-true ratios are not broadly
/// standardized.
pub const MOUSE_SCROLL_PIXELS_PER_LINE: f64 = 100.0;

/// Cursor visibility / grab mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorMode {
    /// Normal visible cursor.
    #[default]
    Normal,
    /// Cursor is hidden but not constrained.
    Hidden,
    /// Cursor is hidden and locked to the window center.
    Locked,
}

/// A single input event, translated from the windowing backend's OS event.
///
/// Keys are identified by **physical** position ([`KeyCode`]), matching
/// Bevy's `KeyboardInput.key_code`. Unlike Bevy, Moonfield currently has no
/// logical-key / IME layer — text input is owned by egui in the editor path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// A key was pressed. `repeat` is true for OS auto-repeat events; they
    /// maintain pressed state but never re-arm the `just_pressed` edge.
    KeyPressed { code: KeyCode, repeat: bool },
    /// A key was released.
    KeyReleased { code: KeyCode },
    /// A mouse button was pressed.
    MouseButtonPressed { button: MouseButton },
    /// A mouse button was released.
    MouseButtonReleased { button: MouseButton },
    /// The cursor moved, in logical pixels since the previous event.
    MouseMotion { dx: f64, dy: f64 },
    /// The mouse wheel scrolled. The `unit` says whether `x`/`y` are lines
    /// or pixels; no conversion is applied (convert with
    /// [`MOUSE_SCROLL_PIXELS_PER_LINE`] if you need one).
    MouseWheel {
        unit: MouseScrollUnit,
        x: f64,
        y: f64,
    },
    /// The window lost keyboard focus; all pressed state was cleared so
    /// keys cannot get stuck (Alt-Tab between press and release).
    FocusLost,
}

/// Frame-latched input state resource.
///
/// `pressed_*` sets persist across frames until the corresponding release;
/// the `just_*` edge sets, the event queue, and the mouse accumulators are
/// frame-scoped and cleared by [`InputState::end_frame`].
///
/// A press and release landing in the same frame record both edges (taps
/// are never lost). Auto-repeat press events on an already-pressed key do
/// not re-arm the edge.
#[derive(Debug, Default)]
pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    pressed_buttons: HashSet<MouseButton>,
    just_pressed_keys: HashSet<KeyCode>,
    just_released_keys: HashSet<KeyCode>,
    just_pressed_buttons: HashSet<MouseButton>,
    just_released_buttons: HashSet<MouseButton>,
    /// Currently held keyboard modifiers (maintained by the backend from
    /// the OS modifiers-changed event).
    modifiers: Modifiers,
    /// This frame's events, in arrival order.
    events: Vec<InputEvent>,
    /// Cursor motion accumulated this frame, in logical pixels.
    mouse_delta: (f64, f64),
    /// Line-unit scroll accumulated this frame.
    scroll_lines: (f64, f64),
    /// Pixel-unit scroll accumulated this frame.
    scroll_pixels: (f64, f64),
    /// Last reported absolute cursor position, in logical pixels.
    mouse_position: (f64, f64),
    /// Current cursor visibility/grab mode.
    cursor_mode: CursorMode,
}

impl InputState {
    /// Apply one event, updating pressed state, frame edges, and
    /// accumulators. The event is also queued for event-replay consumers.
    pub fn apply_event(&mut self, event: InputEvent) {
        match event {
            InputEvent::KeyPressed { code, repeat } => {
                // A repeat event on a key that lost its pressed record (e.g.
                // across a focus loss) still restores pressed state, but a
                // repeat never arms the `just_pressed` edge.
                if self.pressed_keys.insert(code) && !repeat {
                    self.just_pressed_keys.insert(code);
                }
            }
            InputEvent::KeyReleased { code } => {
                if self.pressed_keys.remove(&code) {
                    self.just_released_keys.insert(code);
                }
            }
            InputEvent::MouseButtonPressed { button } => {
                if self.pressed_buttons.insert(button) {
                    self.just_pressed_buttons.insert(button);
                }
            }
            InputEvent::MouseButtonReleased { button } => {
                if self.pressed_buttons.remove(&button) {
                    self.just_released_buttons.insert(button);
                }
            }
            InputEvent::MouseMotion { dx, dy } => {
                self.mouse_delta.0 += dx;
                self.mouse_delta.1 += dy;
            }
            InputEvent::MouseWheel { unit, x, y } => match unit {
                MouseScrollUnit::Line => {
                    self.scroll_lines.0 += x;
                    self.scroll_lines.1 += y;
                }
                MouseScrollUnit::Pixel => {
                    self.scroll_pixels.0 += x;
                    self.scroll_pixels.1 += y;
                }
            },
            InputEvent::FocusLost => {
                self.pressed_keys.clear();
                self.pressed_buttons.clear();
                self.just_pressed_keys.clear();
                self.just_released_keys.clear();
                self.just_pressed_buttons.clear();
                self.just_released_buttons.clear();
                self.modifiers = Modifiers::empty();
            }
        }
        self.events.push(event);
    }

    /// Clear frame-scoped data: edge sets, the event queue, and the mouse
    /// accumulators. Pressed state persists. Called by the windowing
    /// backend once per frame, after the app update has consumed the frame.
    pub fn end_frame(&mut self) {
        self.just_pressed_keys.clear();
        self.just_released_keys.clear();
        self.just_pressed_buttons.clear();
        self.just_released_buttons.clear();
        self.events.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll_lines = (0.0, 0.0);
        self.scroll_pixels = (0.0, 0.0);
    }

    /// Keys currently held down.
    pub fn pressed_keys(&self) -> &HashSet<KeyCode> {
        &self.pressed_keys
    }

    /// True if `code` is currently held down.
    pub fn pressed(&self, code: KeyCode) -> bool {
        self.pressed_keys.contains(&code)
    }

    /// Keys pressed this frame.
    pub fn just_pressed_keys(&self) -> &HashSet<KeyCode> {
        &self.just_pressed_keys
    }

    /// True if `code` was pressed this frame.
    pub fn just_pressed(&self, code: KeyCode) -> bool {
        self.just_pressed_keys.contains(&code)
    }

    /// Keys released this frame.
    pub fn just_released_keys(&self) -> &HashSet<KeyCode> {
        &self.just_released_keys
    }

    /// True if `code` was released this frame.
    pub fn just_released(&self, code: KeyCode) -> bool {
        self.just_released_keys.contains(&code)
    }

    /// Mouse buttons currently held down.
    pub fn pressed_buttons(&self) -> &HashSet<MouseButton> {
        &self.pressed_buttons
    }

    /// True if `button` is currently held down.
    pub fn button_pressed(&self, button: MouseButton) -> bool {
        self.pressed_buttons.contains(&button)
    }

    /// Mouse buttons pressed this frame.
    pub fn just_pressed_buttons(&self) -> &HashSet<MouseButton> {
        &self.just_pressed_buttons
    }

    /// Mouse buttons released this frame.
    pub fn just_released_buttons(&self) -> &HashSet<MouseButton> {
        &self.just_released_buttons
    }

    /// Currently held keyboard modifiers.
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Update the modifier state (called by the backend on the OS
    /// modifiers-changed event).
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// This frame's events, in arrival order.
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Cursor motion accumulated this frame, in logical pixels.
    pub fn mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }

    /// Line-unit scroll accumulated this frame.
    pub fn scroll_lines(&self) -> (f64, f64) {
        self.scroll_lines
    }

    /// Pixel-unit scroll accumulated this frame.
    pub fn scroll_pixels(&self) -> (f64, f64) {
        self.scroll_pixels
    }

    /// Total scroll this frame expressed in lines, converting any
    /// pixel-unit accumulation at [`MOUSE_SCROLL_PIXELS_PER_LINE`].
    pub fn scroll_in_lines(&self) -> (f64, f64) {
        (
            self.scroll_lines.0 + self.scroll_pixels.0 / MOUSE_SCROLL_PIXELS_PER_LINE,
            self.scroll_lines.1 + self.scroll_pixels.1 / MOUSE_SCROLL_PIXELS_PER_LINE,
        )
    }

    /// Set the absolute cursor position, in logical pixels.
    pub fn set_mouse_position(&mut self, position: (f64, f64)) {
        self.mouse_position = position;
    }

    /// Last reported absolute cursor position, in logical pixels.
    pub fn mouse_position(&self) -> (f64, f64) {
        self.mouse_position
    }

    /// Set the cursor visibility / grab mode.
    pub fn set_cursor_mode(&mut self, mode: CursorMode) {
        self.cursor_mode = mode;
    }

    /// Current cursor visibility / grab mode.
    pub fn cursor_mode(&self) -> CursorMode {
        self.cursor_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::KeyPressed {
            code,
            repeat: false,
        }
    }

    #[test]
    fn pressed_and_edges_tracked_per_frame() {
        let mut input = InputState::default();
        input.apply_event(key(KeyCode::Space));
        assert!(input.pressed(KeyCode::Space));
        assert!(input.just_pressed(KeyCode::Space));

        // Auto-repeat on a held key does not re-arm the edge.
        input.apply_event(InputEvent::KeyPressed {
            code: KeyCode::Space,
            repeat: true,
        });
        assert_eq!(input.just_pressed_keys().len(), 1);

        // Frame boundary: edges clear, pressed state persists.
        input.end_frame();
        assert!(input.pressed(KeyCode::Space));
        assert!(input.just_pressed_keys().is_empty());
        assert!(input.events().is_empty());
    }

    #[test]
    fn repeat_on_unpressed_key_restores_state_without_edge() {
        // Focus loss clears pressed state while the OS keeps auto-repeating;
        // the repeat must restore `pressed` but not arm `just_pressed`.
        let mut input = InputState::default();
        input.apply_event(InputEvent::KeyPressed {
            code: KeyCode::KeyW,
            repeat: true,
        });
        assert!(input.pressed(KeyCode::KeyW));
        assert!(!input.just_pressed(KeyCode::KeyW));
    }

    #[test]
    fn tap_within_one_frame_records_both_edges() {
        let mut input = InputState::default();
        input.apply_event(key(KeyCode::Space));
        input.apply_event(InputEvent::KeyReleased {
            code: KeyCode::Space,
        });
        assert!(input.just_pressed(KeyCode::Space));
        assert!(input.just_released(KeyCode::Space));
        assert!(!input.pressed(KeyCode::Space));
    }

    #[test]
    fn focus_lost_clears_all_state() {
        let mut input = InputState::default();
        input.apply_event(key(KeyCode::Space));
        input.apply_event(InputEvent::MouseButtonPressed {
            button: MouseButton::Left,
        });
        input.set_modifiers(Modifiers::SHIFT);
        input.apply_event(InputEvent::FocusLost);
        assert!(input.pressed_keys().is_empty());
        assert!(input.pressed_buttons().is_empty());
        assert!(input.just_pressed_keys().is_empty());
        assert!(input.just_pressed_buttons().is_empty());
        assert!(input.modifiers().is_empty());
        // The event itself is still queued so consumers can react (pause).
        assert_eq!(input.events().last(), Some(&InputEvent::FocusLost));
    }

    #[test]
    fn mouse_accumulators_reset_each_frame() {
        let mut input = InputState::default();
        input.apply_event(InputEvent::MouseMotion { dx: 1.5, dy: -2.0 });
        input.apply_event(InputEvent::MouseMotion { dx: 0.5, dy: 1.0 });
        input.apply_event(InputEvent::MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 3.0,
        });
        input.apply_event(InputEvent::MouseWheel {
            unit: MouseScrollUnit::Pixel,
            x: 0.0,
            y: 200.0,
        });
        assert_eq!(input.mouse_delta(), (2.0, -1.0));
        assert_eq!(input.scroll_lines(), (0.0, 3.0));
        assert_eq!(input.scroll_pixels(), (0.0, 200.0));
        assert_eq!(input.scroll_in_lines(), (0.0, 5.0));
        input.end_frame();
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
        assert_eq!(input.scroll_lines(), (0.0, 0.0));
        assert_eq!(input.scroll_pixels(), (0.0, 0.0));
    }
}
