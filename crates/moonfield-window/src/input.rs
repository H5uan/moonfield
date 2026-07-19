//! Backend-agnostic input state and event types.
//!
//! A windowing backend (e.g. `moonfield-winit`) translates raw OS events
//! into [`InputEvent`]s and applies them to the [`InputState`] resource as
//! they arrive; once per frame (after the app update has consumed them) it
//! calls [`InputState::end_frame`]. Consumers — script runtimes, ECS
//! systems — read the resource during the update.
//!
//! The model follows Bevy's `ButtonInput` contract: pressed state persists
//! across frames, while `just_pressed`/`just_released` edges, the event
//! queue, and the mouse accumulators are frame-scoped.

use std::collections::HashSet;

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
/// Key and button codes are strings matching winit's `KeyCode` /
/// `MouseButton` debug names (e.g. `"Space"`, `"KeyW"`, `"ArrowLeft"`,
/// `"Left"`) so scripts and backends share one vocabulary without depending
/// on winit.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    /// A key was pressed (auto-repeat events are filtered by the backend).
    KeyPressed { code: String },
    /// A key was released.
    KeyReleased { code: String },
    /// A mouse button was pressed.
    MouseButtonPressed { button: String },
    /// A mouse button was released.
    MouseButtonReleased { button: String },
    /// The cursor moved, in logical pixels since the previous event.
    MouseMotion { dx: f64, dy: f64 },
    /// The mouse wheel scrolled, in lines (pixel deltas converted at ~16px
    /// per line).
    MouseWheel { dx: f64, dy: f64 },
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
    pressed_keys: HashSet<String>,
    pressed_buttons: HashSet<String>,
    just_pressed_keys: HashSet<String>,
    just_released_keys: HashSet<String>,
    just_pressed_buttons: HashSet<String>,
    just_released_buttons: HashSet<String>,
    /// This frame's events, in arrival order.
    events: Vec<InputEvent>,
    /// Cursor motion accumulated this frame, in logical pixels.
    mouse_delta: (f64, f64),
    /// Scroll accumulated this frame, in lines.
    mouse_scroll: (f64, f64),
    /// Last reported absolute cursor position, in logical pixels.
    mouse_position: (f64, f64),
    /// Current cursor visibility/grab mode.
    cursor_mode: CursorMode,
}

impl InputState {
    /// Apply one event, updating pressed state, frame edges, and
    /// accumulators. The event is also queued for event-replay consumers.
    pub fn apply_event(&mut self, event: InputEvent) {
        match &event {
            InputEvent::KeyPressed { code } => {
                // `insert` returns false for auto-repeat on a held key.
                if self.pressed_keys.insert(code.clone()) {
                    self.just_pressed_keys.insert(code.clone());
                }
            }
            InputEvent::KeyReleased { code } => {
                if self.pressed_keys.remove(code) {
                    self.just_released_keys.insert(code.clone());
                }
            }
            InputEvent::MouseButtonPressed { button } => {
                if self.pressed_buttons.insert(button.clone()) {
                    self.just_pressed_buttons.insert(button.clone());
                }
            }
            InputEvent::MouseButtonReleased { button } => {
                if self.pressed_buttons.remove(button) {
                    self.just_released_buttons.insert(button.clone());
                }
            }
            InputEvent::MouseMotion { dx, dy } => {
                self.mouse_delta.0 += dx;
                self.mouse_delta.1 += dy;
            }
            InputEvent::MouseWheel { dx, dy } => {
                self.mouse_scroll.0 += dx;
                self.mouse_scroll.1 += dy;
            }
            InputEvent::FocusLost => {
                self.pressed_keys.clear();
                self.pressed_buttons.clear();
                self.just_pressed_keys.clear();
                self.just_released_keys.clear();
                self.just_pressed_buttons.clear();
                self.just_released_buttons.clear();
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
        self.mouse_scroll = (0.0, 0.0);
    }

    /// Keys currently held down.
    pub fn pressed_keys(&self) -> &HashSet<String> {
        &self.pressed_keys
    }

    /// Keys pressed this frame.
    pub fn just_pressed_keys(&self) -> &HashSet<String> {
        &self.just_pressed_keys
    }

    /// Keys released this frame.
    pub fn just_released_keys(&self) -> &HashSet<String> {
        &self.just_released_keys
    }

    /// Mouse buttons currently held down.
    pub fn pressed_buttons(&self) -> &HashSet<String> {
        &self.pressed_buttons
    }

    /// Mouse buttons pressed this frame.
    pub fn just_pressed_buttons(&self) -> &HashSet<String> {
        &self.just_pressed_buttons
    }

    /// Mouse buttons released this frame.
    pub fn just_released_buttons(&self) -> &HashSet<String> {
        &self.just_released_buttons
    }

    /// This frame's events, in arrival order.
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Cursor motion accumulated this frame, in logical pixels.
    pub fn mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }

    /// Scroll accumulated this frame, in lines.
    pub fn mouse_scroll(&self) -> (f64, f64) {
        self.mouse_scroll
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

    fn key(code: &str) -> InputEvent {
        InputEvent::KeyPressed {
            code: code.to_string(),
        }
    }

    #[test]
    fn pressed_and_edges_tracked_per_frame() {
        let mut input = InputState::default();
        input.apply_event(key("Space"));
        assert!(input.pressed_keys().contains("Space"));
        assert!(input.just_pressed_keys().contains("Space"));

        // Auto-repeat on a held key does not re-arm the edge.
        input.apply_event(key("Space"));
        assert_eq!(input.just_pressed_keys().len(), 1);

        // Frame boundary: edges clear, pressed state persists.
        input.end_frame();
        assert!(input.pressed_keys().contains("Space"));
        assert!(input.just_pressed_keys().is_empty());
        assert!(input.events().is_empty());
    }

    #[test]
    fn tap_within_one_frame_records_both_edges() {
        let mut input = InputState::default();
        input.apply_event(key("Space"));
        input.apply_event(InputEvent::KeyReleased {
            code: "Space".to_string(),
        });
        assert!(input.just_pressed_keys().contains("Space"));
        assert!(input.just_released_keys().contains("Space"));
        assert!(!input.pressed_keys().contains("Space"));
    }

    #[test]
    fn focus_lost_clears_all_state() {
        let mut input = InputState::default();
        input.apply_event(key("Space"));
        input.apply_event(InputEvent::MouseButtonPressed {
            button: "Left".to_string(),
        });
        input.apply_event(InputEvent::FocusLost);
        assert!(input.pressed_keys().is_empty());
        assert!(input.pressed_buttons().is_empty());
        assert!(input.just_pressed_keys().is_empty());
        assert!(input.just_pressed_buttons().is_empty());
        // The event itself is still queued so consumers can react (pause).
        assert_eq!(input.events().last(), Some(&InputEvent::FocusLost));
    }

    #[test]
    fn mouse_accumulators_reset_each_frame() {
        let mut input = InputState::default();
        input.apply_event(InputEvent::MouseMotion { dx: 1.5, dy: -2.0 });
        input.apply_event(InputEvent::MouseMotion { dx: 0.5, dy: 1.0 });
        input.apply_event(InputEvent::MouseWheel { dx: 0.0, dy: 3.0 });
        assert_eq!(input.mouse_delta(), (2.0, -1.0));
        assert_eq!(input.mouse_scroll(), (0.0, 3.0));
        input.end_frame();
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
        assert_eq!(input.mouse_scroll(), (0.0, 0.0));
    }
}
