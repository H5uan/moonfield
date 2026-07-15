//! Windowing plugin for Moonfield, built on `winit`.
//!
//! Provides a [`WinitPlugin`] that creates a window and runs the winit event
//! loop, driving the application's update cycle.

use moonfield_app::{App, Plugin};
use moonfield_log::error;
use moonfield_window::RawHandleWrapper;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
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
}

/// Control-flow strategy for the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    /// Poll as fast as possible (no waiting).
    Poll,
    /// Wait for the next event, then wake up.
    Wait,
}

impl Default for WaitMode {
    fn default() -> Self {
        Self::Wait
    }
}

impl Default for WinitPlugin {
    fn default() -> Self {
        Self {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            wait_mode: WaitMode::Wait,
        }
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
}

impl Plugin for WinitPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowConfig {
            title: self.title.clone(),
            width: self.width,
            height: self.height,
            wait_mode: self.wait_mode,
        });
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
        })
        .unwrap_or(WindowConfig {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            wait_mode: WaitMode::Wait,
        });

    let mut handler = WinitHandler {
        app,
        window: None,
        config,
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
        event: winit::event::WindowEvent,
    ) {
        match event {
            winit::event::WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            winit::event::WindowEvent::Resized(size) => {
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
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.config.wait_mode {
            WaitMode::Poll => event_loop.set_control_flow(ControlFlow::Poll),
            WaitMode::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
        self.app.update();
    }
}
