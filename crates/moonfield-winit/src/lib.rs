//! Windowing plugin for Moonfield, built on `winit`.
//!
//! Provides a [`WinitPlugin`] that creates a window and runs the winit event
//! loop, driving the application's update cycle.
//!
//! A window is an ECS entity carrying a [`Window`] component (plus
//! [`PrimaryWindow`] and [`RawHandleWrapper`]); the backend owns the OS
//! window, writes resize/DPI changes back into the component, and applies
//! component-side mutations (title, cursor mode) once per frame via a
//! [`CachedWindow`] field diff (Bevy's `changed_windows` pattern).

use converters::{convert_modifiers, convert_mouse_button, convert_physical_key_code};
use moonfield_app::{App, AppExit, Last, Plugin};
use moonfield_ecs::{Entity, Messages, World};
use moonfield_log::error;
use moonfield_window::{
    InputEvent, InputState, MouseScrollUnit, PrimaryWindow, RawHandleWrapper, Window,
    WindowControl, WindowEventKind, WindowResolution,
};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window as WinitWindowHandle, WindowAttributes, WindowId},
};

mod converters;
mod windows;
mod winit_config;

pub use windows::{CachedWindow, WinitWindows};
pub use winit_config::{UpdateMode, WinitSettings};

/// Events that can be sent into the winit event loop from outside it.
///
/// Sent via the [`EventLoopProxyWrapper`] resource (mirrors bevy's
/// `WinitUserEvent`).
#[derive(Debug, Clone, Copy)]
pub enum WinitUserEvent {
    /// Dummy event that just wakes up the event loop (e.g. a UI toolkit's
    /// repaint request, or a background thread asking for a frame).
    WakeUp,
}

/// A wrapper around [`winit::event_loop::EventLoopProxy`], stored as a world
/// resource so any system (or external thread) can wake the event loop while
/// it idles in a [`Reactive`](UpdateMode::Reactive) update mode.
#[derive(Clone)]
pub struct EventLoopProxyWrapper(winit::event_loop::EventLoopProxy<WinitUserEvent>);

impl EventLoopProxyWrapper {
    /// Wake the event loop, requesting a new frame.
    pub fn wake_up(&self) {
        let _ = self.0.send_event(WinitUserEvent::WakeUp);
    }
}

impl std::ops::Deref for EventLoopProxyWrapper {
    type Target = winit::event_loop::EventLoopProxy<WinitUserEvent>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Plugin that creates a winit window and runs the winit event loop.
///
/// The plugin stores the raw window as a [`WinitWindow`] resource, spawns
/// the primary window entity ([`Window`] + [`PrimaryWindow`] +
/// [`RawHandleWrapper`] components) when the event loop resumes, and
/// replaces the app's runner with a winit-based event loop. On each
/// `about_to_wait` event the app's update systems are invoked.
///
/// Event delivery uses message channels registered via `App::add_message`:
/// [`WindowEventKind`] (translated lifecycle events) and raw
/// [`WindowEvent`]s (for consumers like `egui_winit` that need the original
/// winit events). Buffers swap once per frame in the `First` schedule.
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
    /// Update-rate settings (stored as a [`WinitSettings`] resource,
    /// re-read every frame decision).
    pub settings: WinitSettings,
    /// Window control signals (exit policy).
    pub window_control: WindowControl,
}

impl Default for WinitPlugin {
    fn default() -> Self {
        Self {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            settings: WinitSettings::default(),
            window_control: WindowControl::default(),
        }
    }
}

/// Share the [`WindowControl`] handle with the event loop.
impl WinitPlugin {
    pub fn with_window_control(mut self, window_control: WindowControl) -> Self {
        self.window_control = window_control;
        self
    }

    /// Set the update-rate settings (e.g. [`WinitSettings::desktop_app`]
    /// for an editor, [`WinitSettings::continuous`] for a game).
    pub fn with_settings(mut self, settings: WinitSettings) -> Self {
        self.settings = settings;
        self
    }
}

/// A resource holding the raw winit [`Window`].
///
/// Other plugins (e.g. `moonfield-render-core`) can access this resource to create
/// a Vulkan surface from the window handle via `raw-window-handle`.
#[derive(Clone)]
pub struct WinitWindow(pub Arc<WinitWindowHandle>);

/// Internal configuration resource stored by [`WinitPlugin`].
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Window control signals (exit policy).
    pub window_control: WindowControl,
}

impl Plugin for WinitPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WindowConfig {
            title: self.title.clone(),
            width: self.width,
            height: self.height,
            window_control: self.window_control.clone(),
        });
        app.insert_resource(self.settings);
        app.insert_resource(InputState::default());
        // Window lifecycle + raw winit events are delivered through message
        // channels (two-frame retention; swapped in the `First` schedule).
        app.add_message::<WindowEventKind>();
        app.add_message::<WindowEvent>();
        app.insert_resource(WinitWindows::default());
        // Frame-end bookkeeping as systems, not runner glue (Bevy's
        // `changed_windows` runs in `Last` the same way): apply ECS-side
        // window mutations and clear the frame-scoped input state.
        app.add_systems(Last, (windows::sync_windows, input_end_frame));
        // The shared exit-policy handle, readable by other plugins (e.g. the
        // editor's MOONFIELD_EDITOR_AUTO_CLOSE helper calls request_exit on
        // this same handle).
        app.insert_resource(self.window_control.clone());
    }

    fn finish(&self, app: &mut App) {
        // Set the Runner so App::run() delegates to the winit event loop
        // instead of the run_once default. The runner drives app.update() per
        // frame; rendering and the frame's other bookkeeping are part of the
        // tick and of the `Last` systems registered in `build`.
        app.set_runner(|app: &mut App| winit_run(app));
    }

    fn name(&self) -> &str {
        "moonfield_winit::WinitPlugin"
    }
}

/// Creates an [`EventLoop`] + [`Window`] and drives the app via winit events.
/// Called from the [`moonfield_app::Runner`] set by [`WinitPlugin`]. Returns
/// the exit code requested via the [`AppExit`] resource, or success when no
/// request was made (e.g. the window was closed through `WindowControl`).
pub fn winit_run(app: &mut App) -> AppExit {
    let event_loop = EventLoop::<WinitUserEvent>::with_user_event()
        .build()
        .expect("failed to create winit event loop");

    // Expose the proxy so systems and external threads can wake the loop
    // while it idles in a Reactive update mode.
    app.insert_resource(EventLoopProxyWrapper(event_loop.create_proxy()));

    let config = app
        .get_resource::<WindowConfig>()
        .map(|c| WindowConfig {
            title: c.title.clone(),
            width: c.width,
            height: c.height,
            window_control: c.window_control.clone(),
        })
        .unwrap_or(WindowConfig {
            title: "Moonfield".to_string(),
            width: 800,
            height: 600,
            window_control: WindowControl::default(),
        });

    let mut handler = WinitHandler {
        app,
        window: None,
        window_entity: None,
        config,
        last_cursor: None,
        focused: true,
        last_frame: std::time::Instant::now(),
        redraw_pending: false,
        window_event_received: false,
        device_event_received: false,
        user_event_received: false,
    };

    if let Err(e) = event_loop.run_app(&mut handler) {
        error!("event loop exited with error: {e}");
        return AppExit::error();
    }

    // Read the exit request (if any) after the loop has fully drained.
    app.world()
        .get_resource::<AppExit>()
        .map(|exit| *exit)
        .unwrap_or(AppExit::SUCCESS)
}

/// Bridge between winit's [`ApplicationHandler`] and moonfield's [`App`].
struct WinitHandler<'a> {
    app: &'a mut App,
    window: Option<Arc<WinitWindowHandle>>,
    /// The primary window entity; `None` until `resumed` creates/adopts it.
    window_entity: Option<Entity>,
    config: WindowConfig,
    /// Last cursor position, used to compute motion deltas.
    last_cursor: Option<(f64, f64)>,
    /// Whether any window currently has focus (drives focused/unfocused
    /// [`UpdateMode`] selection).
    focused: bool,
    /// Start time of the previous frame (drives `Reactive` wait deadlines).
    last_frame: std::time::Instant,
    /// A redraw was requested but not yet delivered by the OS.
    redraw_pending: bool,
    /// Event-kind flags accumulated since the last `about_to_wait`, gating
    /// `Reactive` wake-ups by their `react_to_*` switches.
    window_event_received: bool,
    device_event_received: bool,
    user_event_received: bool,
}

impl ApplicationHandler<WinitUserEvent> for WinitHandler<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Adopt a pre-spawned window entity (Bevy-style: user code may spawn
        // a `Window` component at startup), or spawn the primary window
        // entity from the plugin config.
        let existing = <&Window as moonfield_ecs::WorldQuery>::fetch(self.app.world())
            .next()
            .map(|(e, _)| e);
        let entity = match existing {
            Some(e) => e,
            None => {
                let e = self.app.world_mut().spawn_empty();
                self.app.world_mut().insert_component(
                    e,
                    Window {
                        title: self.config.title.clone(),
                        // Physical size is written back with real values once
                        // the OS window exists; start at scale factor 1.0.
                        resolution: WindowResolution::new(
                            self.config.width,
                            self.config.height,
                            1.0,
                        ),
                        ..Default::default()
                    },
                );
                e
            }
        };

        let attrs = match self.app.world().get_component::<Window>(entity) {
            Some(w) => WindowAttributes::default()
                .with_title(&w.title)
                .with_inner_size(LogicalSize::new(
                    w.resolution.width() as f64,
                    w.resolution.height() as f64,
                )),
            None => WindowAttributes::default()
                .with_title(&self.config.title)
                .with_inner_size(LogicalSize::new(self.config.width, self.config.height)),
        };

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);

                // — Write the real physical size / DPI back into the component —
                if let Some(mut w) = self.app.world_mut().get_component_mut::<Window>(entity) {
                    let size = window.inner_size();
                    w.resolution.set_physical(size.width, size.height);
                    w.resolution.set_scale_factor(window.scale_factor());
                }

                // — Attach the window-side components —
                let world = self.app.world_mut();
                if world.get_component::<PrimaryWindow>(entity).is_none() {
                    world.insert_component(entity, PrimaryWindow);
                }
                match (
                    window.as_ref().window_handle(),
                    window.as_ref().display_handle(),
                ) {
                    (Ok(w_handle), Ok(d_handle)) => {
                        world.insert_component(
                            entity,
                            RawHandleWrapper {
                                window_handle: w_handle.into(),
                                display_handle: d_handle.into(),
                            },
                        );
                    }
                    _ => {
                        error!("failed to get window handles");
                    }
                }
                if let Some(w) = world.get_component::<Window>(entity) {
                    let cache = CachedWindow::new(w);
                    world.insert_component(entity, cache);
                }

                // — Register the mapping and the raw-window escape hatch —
                if let Some(mut windows) = self.app.get_resource_mut::<WinitWindows>() {
                    windows.insert(entity, window.clone());
                }
                self.app.insert_resource(WinitWindow(window.clone()));

                self.window = Some(window);
                self.window_entity = Some(entity);
                // Kick the first frame deterministically instead of relying
                // on the platform to deliver an initial RedrawRequested.
                self.request_redraw();
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
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Track the event kind for Reactive-mode wake-up gating; the redraw
        // request itself is not a wake-up reason.
        if !matches!(event, WindowEvent::RedrawRequested) {
            self.window_event_received = true;
        }

        // Broadcast the raw event to consumers that need the original winit
        // event (e.g. egui_winit), through the raw-event message channel.
        if let Some(mut raw) = self.app.get_resource_mut::<Messages<WindowEvent>>() {
            raw.write(event.clone());
        }

        // Translate input events into the shared InputState resource
        // (consumed during the next app update). Auto-repeat presses are
        // passed through (flagged `repeat`); InputState keeps them from
        // re-arming the just_pressed edge.
        let input_event = match &event {
            WindowEvent::KeyboardInput { event, .. } => {
                let code = convert_physical_key_code(event.physical_key);
                Some(match event.state {
                    ElementState::Pressed => InputEvent::KeyPressed {
                        code,
                        repeat: event.repeat,
                    },
                    ElementState::Released => InputEvent::KeyReleased { code },
                })
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = convert_mouse_button(*button);
                Some(match state {
                    ElementState::Pressed => InputEvent::MouseButtonPressed { button },
                    ElementState::Released => InputEvent::MouseButtonReleased { button },
                })
            }
            WindowEvent::CursorMoved { position, .. } => {
                let pos = (position.x, position.y);
                if let Some(mut input) = self.app.get_resource_mut::<InputState>() {
                    input.set_mouse_position(pos);
                }
                let (dx, dy) = self
                    .last_cursor
                    .map(|last| (pos.0 - last.0, pos.1 - last.1))
                    .unwrap_or((0.0, 0.0));
                self.last_cursor = Some(pos);
                Some(InputEvent::MouseMotion { dx, dy })
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Keep the original unit: LineDelta stays lines, PixelDelta
                // (precision touchpads) stays pixels. Consumers convert via
                // MOUSE_SCROLL_PIXELS_PER_LINE if they need one unit.
                let (unit, x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        (MouseScrollUnit::Line, *x as f64, *y as f64)
                    }
                    MouseScrollDelta::PixelDelta(pos) => (MouseScrollUnit::Pixel, pos.x, pos.y),
                };
                Some(InputEvent::MouseWheel { unit, x, y })
            }
            WindowEvent::Focused(false) => Some(InputEvent::FocusLost),
            _ => None,
        };
        if let Some(event) = input_event
            && let Some(mut input) = self.app.get_resource_mut::<InputState>()
        {
            input.apply_event(event);
        }

        // Resolve the window entity this event fired for (multi-window
        // shape; single-window builds always resolve to the primary).
        let window_entity = self
            .app
            .get_resource::<WinitWindows>()
            .and_then(|w| w.get_entity(window_id))
            .or(self.window_entity);

        match event {
            WindowEvent::CloseRequested => {
                if let Some(window) = window_entity
                    && let Some(mut events) =
                        self.app.get_resource_mut::<Messages<WindowEventKind>>()
                {
                    events.write(WindowEventKind::CloseRequested { window });
                }
                // Godot's auto_accept_quit: exit immediately by default, unless
                // `auto_exit_on_close` was turned off to take over close
                // handling.
                if self.config.window_control.auto_exit_on_close() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(window) = window_entity {
                    if let Some(mut events) =
                        self.app.get_resource_mut::<Messages<WindowEventKind>>()
                    {
                        events.write(WindowEventKind::Resized {
                            window,
                            width: size.width,
                            height: size.height,
                        });
                    }
                    // Write the OS-side change back into the component.
                    if let Some(mut w) = self.app.world_mut().get_component_mut::<Window>(window) {
                        w.resolution.set_physical(size.width, size.height);
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(window) = window_entity {
                    if let Some(mut events) =
                        self.app.get_resource_mut::<Messages<WindowEventKind>>()
                    {
                        events.write(WindowEventKind::ScaleFactorChanged {
                            window,
                            scale_factor,
                        });
                    }
                    if let Some(mut w) = self.app.world_mut().get_component_mut::<Window>(window) {
                        w.resolution.set_scale_factor(scale_factor);
                        // The scale factor change comes with a new physical
                        // size; keep resolution consistent.
                        if let Some(os_window) = &self.window {
                            let size = os_window.inner_size();
                            w.resolution.set_physical(size.width, size.height);
                        }
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if let Some(window) = window_entity
                    && let Some(mut events) =
                        self.app.get_resource_mut::<Messages<WindowEventKind>>()
                {
                    events.write(if focused {
                        WindowEventKind::FocusGained { window }
                    } else {
                        WindowEventKind::FocusLost { window }
                    });
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                // Modifier keys also arrive as ordinary key presses; this
                // maintains the convenience bitflags view.
                if let Some(mut input) = self.app.get_resource_mut::<InputState>() {
                    input.set_modifiers(convert_modifiers(modifiers.state()));
                }
            }
            // The OS asks for a frame: this is where the frame actually
            // runs (Bevy's redraw_requested-driven model). Frame pacing is
            // paced by the compositor, not by event-loop idle spinning.
            WindowEvent::RedrawRequested => self.run_frame(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // An exit requested from outside the event loop (e.g. a caller on
        // another thread, or a script) can arrive while the loop idles in a
        // Reactive mode; the WakeUp that accompanies it lands here.
        if self.config.window_control.exit_requested() {
            event_loop.exit();
            return;
        }

        let settings = self
            .app
            .get_resource::<WinitSettings>()
            .map(|s| *s)
            .unwrap_or_default();
        match settings.update_mode(self.focused) {
            UpdateMode::Continuous => {
                event_loop.set_control_flow(ControlFlow::Poll);
                self.request_redraw();
            }
            UpdateMode::Reactive {
                wait,
                react_to_device_events,
                react_to_user_events,
                react_to_window_events,
            } => {
                let next_tick = self.last_frame + wait;
                event_loop.set_control_flow(ControlFlow::WaitUntil(next_tick));
                let woke = (self.window_event_received && react_to_window_events)
                    || (self.device_event_received && react_to_device_events)
                    || (self.user_event_received && react_to_user_events);
                if woke || std::time::Instant::now() >= next_tick {
                    self.request_redraw();
                }
            }
        }
        self.window_event_received = false;
        self.device_event_received = false;
        self.user_event_received = false;
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: WinitUserEvent) {
        match event {
            WinitUserEvent::WakeUp => self.user_event_received = true,
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        _event: winit::event::DeviceEvent,
    ) {
        // Raw device input (e.g. `DeviceEvent::MouseMotion` for FPS-style
        // pointer-locked cameras) is deliberately not translated yet — it
        // only participates in Reactive-mode wake-up gating.
        self.device_event_received = true;
    }
}

impl WinitHandler<'_> {
    /// Request that the OS deliver `RedrawRequested` (coalesced).
    fn request_redraw(&mut self) {
        if self.redraw_pending {
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
            self.redraw_pending = true;
        }
    }

    /// Run one frame: a single full `App` tick. Called from `RedrawRequested`;
    /// time advance (a `First` system), rendering, window-diff application,
    /// and input clearing are all part of the tick or of `Last` systems, so
    /// the runner only decides *when* to tick.
    fn run_frame(&mut self, event_loop: &ActiveEventLoop) {
        self.app.update();
        self.last_frame = std::time::Instant::now();
        self.redraw_pending = false;
        // `app_exit()`-style request: a non-event-loop caller asked us to quit.
        if self.config.window_control.exit_requested() {
            event_loop.exit();
        }
    }
}

/// `Last` system: clear the frame-scoped input state. Previously called by
/// the winit runner after rendering; now part of the schedule so any runner
/// (and headless apps) get it automatically.
fn input_end_frame(world: &mut World) {
    if let Some(mut input) = world.get_resource_mut::<InputState>() {
        input.end_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_registers_window_event_message_channels() {
        let mut app = App::new();
        app.add_plugin(WinitPlugin::default());
        assert!(app.world().contains_resource::<Messages<WindowEventKind>>());
        assert!(app.world().contains_resource::<Messages<WindowEvent>>());
    }
}
