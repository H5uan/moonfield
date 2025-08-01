use moonfield_impl::engine::{
    Engine, EngineInitParams, GraphicsBackendConstructor, GraphicsContextParams,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{WindowAttributes, WindowId};

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
                graphics_backend_constructor: GraphicsBackendConstructor::default(),
                named_objects: true,
            };

            let engine_init_params = EngineInitParams {
                graphics_context_params,
            };

            match Engine::new(engine_init_params) {
                Ok(mut engine) => match engine.initialize_graphics_context(event_loop) {
                    Ok(()) => {
                        println!("Moonfield engine initialized successfully!");
                        self.engine = Some(engine);
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize graphics context: {}", e);
                        event_loop.exit();
                    }
                },
                Err(e) => {
                    eprintln!("Failed to create engine: {}", e);
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(_engine) = &self.engine {
                }

                if let Some(engine) = &self.engine {
                    if let moonfield_impl::engine::GraphicsContext::Initilized(ctx) =
                        &engine.graphics_context
                    {
                        ctx.window.request_redraw();
                    }
                }
            }
            WindowEvent::Resized(physical_size) => {
                println!("Window resized to: {:?}", physical_size);
            }
            _ => (),
        }
    }
}

fn main() {
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = MoonfieldApp::new();

    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("Event loop error: {}", e);
    }
}
