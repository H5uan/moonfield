//! Windowing plugin for Moonfield, built on `winit`.
//!
//! Provides a [`WinitPlugin`] that creates a window and runs the winit event
//! loop, driving the application's update cycle.

use moonfield_app::{App, Plugin};
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

/// Plugin that creates a winit window and runs the winit event loop.
///
/// The plugin stores the window as a [`WinitWindow`] resource and replaces the
/// app's runner with a winit-based event loop. On each `about_to_wait` event
/// the app's update systems are invoked.
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

/// A resource holding the winit [`Window`].
///
/// Other plugins (e.g. `moonfield-lunaris`) can access this resource to create
/// a Vulkan surface from the window handle.
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
        eprintln!("[moonfield-winit] event loop exited with error: {e}");
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
                self.app.insert_resource(WinitWindow(window.clone()));
                self.window = Some(window);
            }
            Err(e) => {
                eprintln!("[moonfield-winit] failed to create window: {e}");
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
                let _ = size;
                // TODO: forward resize to swapchain / renderer
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Apply control-flow preference each cycle.
        match self.config.wait_mode {
            WaitMode::Poll => event_loop.set_control_flow(ControlFlow::Poll),
            WaitMode::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        }
        // Run exactly one frame per event-loop tick, matching Bevy's
        // App::update() semantics.
        self.app.update();
    }
}