//! Update-rate configuration for the winit event loop.
//!
//! Mirrors `bevy_winit`'s `WinitSettings` / `UpdateMode` (verified against
//! bevy main, where `reactive_low_power` is a `Reactive` constructor with
//! `react_to_device_events: false` rather than a separate variant).

use std::time::Duration;

/// Settings for the [`WinitPlugin`](crate::WinitPlugin).
///
/// Stored as a world resource; the backend re-reads it on every frame
/// decision, so systems may mutate it at runtime (e.g. drop to a low-power
/// mode when a game pauses).
#[derive(Debug, Clone, Copy)]
pub struct WinitSettings {
    /// How frequently the app may update while any window has focus.
    pub focused_mode: UpdateMode,
    /// How frequently the app may update while unfocused.
    pub unfocused_mode: UpdateMode,
}

impl WinitSettings {
    /// Default settings for games: [`Continuous`](UpdateMode::Continuous)
    /// while focused, [`reactive_low_power`](UpdateMode::reactive_low_power)
    /// at 60 Hz otherwise.
    pub fn game() -> Self {
        Self {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs_f64(1.0 / 60.0)),
        }
    }

    /// Default settings for desktop applications (editors): reactive to
    /// events with a 5 s fallback tick while focused, 60 s while unfocused.
    ///
    /// Use the [`EventLoopProxyWrapper`](crate::EventLoopProxyWrapper) to
    /// wake the loop from outside the event loop (e.g. a UI toolkit's
    /// repaint request).
    pub fn desktop_app() -> Self {
        Self {
            focused_mode: UpdateMode::reactive(Duration::from_secs(5)),
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs(60)),
        }
    }

    /// Update as fast as possible regardless of focus.
    pub fn continuous() -> Self {
        Self {
            focused_mode: UpdateMode::Continuous,
            unfocused_mode: UpdateMode::Continuous,
        }
    }

    /// The current [`UpdateMode`], depending on focus.
    pub fn update_mode(&self, focused: bool) -> UpdateMode {
        match focused {
            true => self.focused_mode,
            false => self.unfocused_mode,
        }
    }
}

impl Default for WinitSettings {
    fn default() -> Self {
        WinitSettings::game()
    }
}

/// How frequently the app should update.
///
/// Independent of VSync: VSync is controlled by the swapchain's present
/// mode; this setting only governs how often the app is woken to produce a
/// frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateMode {
    /// Update over and over, as fast as possible.
    Continuous,
    /// Update in response to:
    /// - `wait` having elapsed since the previous update,
    /// - new window / device / user events (each gated by its `react_to_*`
    ///   flag),
    /// - a wake-up requested through the
    ///   [`EventLoopProxyWrapper`](crate::EventLoopProxyWrapper).
    Reactive {
        /// Approximate time from the start of one update to the next. Has no
        /// upper limit — the loop waits indefinitely on `Duration::MAX`.
        wait: Duration,
        /// React to device events (raw input) by waking the loop.
        react_to_device_events: bool,
        /// React to user events ([`WinitUserEvent`](crate::WinitUserEvent))
        /// by waking the loop.
        react_to_user_events: bool,
        /// React to window events by waking the loop.
        react_to_window_events: bool,
    },
}

impl UpdateMode {
    /// Reactive mode that wakes for any kind of event.
    pub fn reactive(wait: Duration) -> Self {
        Self::Reactive {
            wait,
            react_to_device_events: true,
            react_to_user_events: true,
            react_to_window_events: true,
        }
    }

    /// Low-power mode: like [`reactive`](UpdateMode::reactive), but ignores
    /// device events (e.g. raw mouse motion), so the app only updates when
    /// interacting with a window. Considerably reduces idle power draw.
    pub fn reactive_low_power(wait: Duration) -> Self {
        Self::Reactive {
            wait,
            react_to_device_events: false,
            react_to_user_events: true,
            react_to_window_events: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_focus_split() {
        let game = WinitSettings::game();
        assert_eq!(game.update_mode(true), UpdateMode::Continuous);
        assert!(matches!(
            game.update_mode(false),
            UpdateMode::Reactive {
                react_to_device_events: false,
                ..
            }
        ));

        let continuous = WinitSettings::continuous();
        assert_eq!(continuous.update_mode(false), UpdateMode::Continuous);

        let desktop = WinitSettings::desktop_app();
        assert!(matches!(
            desktop.update_mode(true),
            UpdateMode::Reactive {
                react_to_window_events: true,
                ..
            }
        ));
    }
}
