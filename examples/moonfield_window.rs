use moonfield::engine::{
    Engine, EngineInitParams, GraphicsBackendConstructor, GraphicsContext,
    GraphicsContextParams,
};
use tracing::{debug, error, info};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{WindowAttributes, WindowId},
};

struct MoonfieldApp {
    engine: Option<Engine>,
}

impl MoonfieldApp {
    fn new() -> Self {
        Self { engine: None }
    }
}

impl ApplicationHandler for MoonfieldApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.engine.is_none() {
            let window_attributes = WindowAttributes::default()
                .with_title("Moonfield Engine Demo")
                .with_inner_size(winit::dpi::LogicalSize::new(800, 600));

            let graphics_context_params = GraphicsContextParams {
                window_attributes,
                vsync: true,
                msaa_sample_count: Some(4),
                graphics_backend_constructor:
                    GraphicsBackendConstructor::default(),
                named_objects: true,
            };

            let engine_init_params =
                EngineInitParams { graphics_context_params };

            match Engine::new(engine_init_params) {
                Ok(mut engine) => match engine
                    .initialize_graphics_context(event_loop)
                {
                    Ok(()) => {
                        info!("Moonfield engine initialized successfully!");
                        // Request initial redraw to start the rendering loop
                        if let GraphicsContext::Initilized(ref ctx) =
                            engine.graphics_context
                        {
                            ctx.window.request_redraw();
                        }
                        self.engine = Some(engine);
                    }
                    Err(e) => {
                        error!("Failed to initialize graphics context: {}", e);
                        event_loop.exit();
                    }
                },
                Err(e) => {
                    error!("Failed to create engine: {}", e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self, event_loop: &ActiveEventLoop, _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                info!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(engine) = &mut self.engine {
                    match engine.render() {
                        Ok(()) => {
                            if let GraphicsContext::Initilized(ctx) =
                                &engine.graphics_context
                            {
                                ctx.window.request_redraw();
                            }
                        }
                        Err(e) => {
                            error!("Render error: {}", e);
                        }
                    }
                }
            }
            WindowEvent::Resized(physical_size) => {
                debug!("Window resized to: {:?}", physical_size);
            }
            _ => (),
        }
    }
}

fn main() {
    // Initialize tracing logging system with optimized configuration
    if let Err(e) = moonfield::core::logging::init_optimized_logging() {
        // Use eprintln! here since tracing is not yet initialized
        eprintln!("Failed to initialize logging: {}", e);
        return;
    }

    info!("Starting Moonfield Engine Demo");

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = MoonfieldApp::new();

    if let Err(e) = event_loop.run_app(&mut app) {
        error!("Event loop error: {}", e);
    }
}
