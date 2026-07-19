//! Windowing plugin for Moonfield, built on `winit`.
//!
//! Provides a [`WinitPlugin`] that creates a window and runs the winit event
//! loop, driving the application's update cycle.

use moonfield_app::{App, Plugin};
use moonfield_log::error;
use moonfield_window::{
    InputEvent, InputState, RawHandleWrapper, WindowControl, WindowEventKind, WindowEvents,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::collections::HashMap;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowAttributes, WindowId},
};

/// Plugin that creates a winit window and runs the winit event loop.
///
/// The plugin stores the window as a [`WinitWindow`] resource, creates the
/// abstract [`moonfield_window::Window`] resource, and replaces the app's
/// runner with a winit-based event loop. On each `about_to_wait` event the
/// app's update systems are invoked.
///
/// # Example
///
/// ```ignore
/// use moonfield_app::App;
/// use moonfield_winit::WinitPlugin;
///
/// App::new()
///     .add_plugins(WinitPlugin::default())
///     .run();
/// ```
pub struct WinitPlugin {
    /// Window title.
    pub title: String,
    /// Initial window width in logical pixels.
    pub width: u32,
    /// Initial window height in logical pixels.
    pub height: u32,
    /// Whether to poll or wait for events.
    pub wait_mode: WaitMode,
    /// Window control signals shared with scripts (exit policy).
    pub window_control: WindowControl,
}

/// Control-flow strategy for the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaitMode {
    /// Poll as fast as possible (no waiting).
    Poll,
    /// Wait for the next event, then wake up.
    #[default]
    Wait,
}

impl Default for WinitPlugin {
    fn default() -> Self {
        Self {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            wait_mode: WaitMode::Wait,
            window_control: WindowControl::default(),
        }
    }
}

/// Share the [`WindowControl`] handle with the event loop (and, via the
/// composition root, with scripts).
impl WinitPlugin {
    pub fn with_window_control(mut self, window_control: WindowControl) -> Self {
        self.window_control = window_control;
        self
    }
}

/// A resource holding the raw winit [`Window`].
///
/// Other plugins (e.g. `moonfield-render`) can access this resource to create
/// a Vulkan surface from the window handle via `raw-window-handle`.
#[derive(Clone)]
pub struct WinitWindow(pub Arc<Window>);

/// Internal configuration resource stored by [`WinitPlugin`].
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub wait_mode: WaitMode,
    /// Window control signals shared with scripts (exit policy).
    pub window_control: WindowControl,
}

impl Plugin for WinitPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowConfig {
            title: self.title.clone(),
            width: self.width,
            height: self.height,
            wait_mode: self.wait_mode,
            window_control: self.window_control.clone(),
        });
        app.insert_resource(InputState::default());
        app.insert_resource(WindowEvents::default());
    }

    fn finish(&self, app: &mut App) {
        app.set_runner(winit_runner);
    }

    fn name(&self) -> &str {
        "moonfield_winit::WinitPlugin"
    }
}

/// Default winit runner: creates an [`EventLoop`] + [`Window`] and drives the
/// app via winit events.
///
/// Stored as the app's runner by [`WinitPlugin::finish`].
pub fn winit_runner(app: &mut App) {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");

    let config = app
        .get_resource::<WindowConfig>()
        .map(|c| WindowConfig {
            title: c.title.clone(),
            width: c.width,
            height: c.height,
            wait_mode: c.wait_mode,
            window_control: c.window_control.clone(),
        })
        .unwrap_or(WindowConfig {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            wait_mode: WaitMode::Wait,
            window_control: WindowControl::default(),
        });

    let mut handler = WinitHandler {
        app,
        window: None,
        config,
        last_cursor: None,
        key_names: HashMap::new(),
        button_names: HashMap::new(),
    };

    if let Err(e) = event_loop.run_app(&mut handler) {
        error!("event loop exited with error: {e}");
    }
}

/// Bridge between winit's [`ApplicationHandler`] and moonfield's [`App`].
struct WinitHandler<'a> {
    app: &'a mut App,
    window: Option<Arc<Window>>,
    config: WindowConfig,
    /// Last cursor position, used to compute motion deltas.
    last_cursor: Option<(f64, f64)>,
    /// Interned key/button debug names, so repeated events for the same
    /// key reuse one cached `String` instead of re-running
    /// `format!("{:?}", ..)` on every OS event.
    key_names: HashMap<KeyCode, String>,
    button_names: HashMap<MouseButton, String>,
}

impl ApplicationHandler for WinitHandler<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);

                // — Store the raw winit window for direct access (e.g. surface creation) —
                self.app.insert_resource(WinitWindow(window.clone()));

                // — Create the abstract moonfield Window resource —
                self.app.insert_resource(moonfield_window::Window {
                    title: self.config.title.clone(),
                    width: self.config.width,
                    height: self.config.height,
                });

                // — Create raw handle wrapper for surface creation —
                match (
                    window.as_ref().window_handle(),
                    window.as_ref().display_handle(),
                ) {
                    (Ok(w_handle), Ok(d_handle)) => {
                        self.app.insert_resource(RawHandleWrapper {
                            window_handle: w_handle.into(),
                            display_handle: d_handle.into(),
                        });
                    }
                    _ => {
                        error!("failed to get window handles");
                    }
                }

                self.window = Some(window);
            }
            Err(e) => {
                error!("failed to create window: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        // Translate input events into the shared InputState resource
        // (consumed during the next app update).
        let input_event = match &event {
            WindowEvent::KeyboardInput { event, .. } => {
                if event.repeat {
                    None
                } else if let PhysicalKey::Code(code) = event.physical_key {
                    let code = self
                        .key_names
                        .entry(code)
                        .or_insert_with(|| format!("{:?}", code))
                        .clone();
                    Some(match event.state {
                        ElementState::Pressed => InputEvent::KeyPressed { code },
                        ElementState::Released => InputEvent::KeyReleased { code },
                    })
                } else {
                    None
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = self
                    .button_names
                    .entry(*button)
                    .or_insert_with(|| format!("{:?}", button))
                    .clone();
                Some(match state {
                    ElementState::Pressed => InputEvent::MouseButtonPressed { button },
                    ElementState::Released => InputEvent::MouseButtonReleased { button },
                })
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = (position.x, position.y);
                let (dx, dy) = self
                    .last_cursor
                    .map(|last| (pos.0 - last.0, pos.1 - last.1))
                    .unwrap_or((0.0, 0.0));
                self.last_cursor = Some(pos);
                Some(InputEvent::MouseMotion { dx, dy })
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x as f64, *y as f64),
                    // Convert pixel deltas (precision touchpads) at ~16px/line.
                    MouseScrollDelta::PixelDelta(pos) => (pos.x / 16.0, pos.y / 16.0),
                };
                Some(InputEvent::MouseWheel { dx, dy })
            }
            WindowEvent::Focused(false) => Some(InputEvent::FocusLost),
            _ => None,
        };
        if let Some(event) = input_event {
            if let Some(mut input) = self.app.get_resource_mut::<InputState>() {
                input.apply_event(event);
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                if let Some(mut events) = self.app.get_resource_mut::<WindowEvents>() {
                    events.push(WindowEventKind::CloseRequested);
                }
                // Godot's auto_accept_quit: exit immediately unless scripts
                // have taken over close handling via
                // `app_set_auto_exit_on_close(false)`.
                if self.config.window_control.auto_exit_on_close() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(mut events) = self.app.get_resource_mut::<WindowEvents>() {
                    events.push(WindowEventKind::Resized {
                        width: size.width,
                        height: size.height,
                    });
                }
                // Update the abstract Window resource with the new size.
                if let Some(mut win) = self.app.get_resource_mut::<moonfield_window::Window>() {
                    win.width = size.width;
                    win.height = size.height;
                }
                // Rebuild raw handles when the window is resized (the handles
                // themselves are still valid, but the size is updated above).
                if let Some(window) = &self.window {
                    match (
                        window.as_ref().window_handle(),
                        window.as_ref().display_handle(),
                    ) {
                        (Ok(w_handle), Ok(d_handle)) => {
                            self.app.insert_resource(RawHandleWrapper {
                                window_handle: w_handle.into(),
                                display_handle: d_handle.into(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                if let Some(mut events) = self.app.get_resource_mut::<WindowEvents>() {
                    events.push(if focused {
                        WindowEventKind::FocusGained
                    } else {
                        WindowEventKind::FocusLost
                    });
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.config.wait_mode {
            WaitMode::Poll => event_loop.set_control_flow(ControlFlow::Poll),
            WaitMode::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
        self.app.update();
        // The frame's input has been consumed — clear frame-scoped state.
        if let Some(mut input) = self.app.get_resource_mut::<InputState>() {
            input.end_frame();
        }
        if let Some(mut events) = self.app.get_resource_mut::<WindowEvents>() {
            events.end_frame();
        }
        // A script asked us to quit via `app_exit()`.
        if self.config.window_control.exit_requested() {
            event_loop.exit();
        }
    }
}
