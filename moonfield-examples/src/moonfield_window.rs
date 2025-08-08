use moonfield::engine::{
    Engine, EngineInitParams, GraphicsBackendConstructor, GraphicsContext,
    GraphicsContextParams,
};
use moonfield::rhi::{
    buffer::BufferKind,
    geometry_buffer::{
        ElementsDescriptor, GeometryBufferDescriptor, GeometryBufferWarpper,
        VertexAttributeDefinition, VertexAttributeKind, VertexBufferData,
        VertexBufferDescriptor,
    },
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
    triangle_geometry: Option<GeometryBufferWarpper>,
}

impl MoonfieldApp {
    fn new() -> Self {
        Self { engine: None, triangle_geometry: None }
    }

    fn create_triangle_geometry(
        &mut self,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(engine) = &self.engine {
            if let GraphicsContext::Initilized(ctx) = &engine.graphics_context {
                // Create a simple triangle
                let vertices: Vec<f32> = vec![
                    // x, y, z
                    0.0, 0.5, 0.0, // Top vertex
                    -0.5, -0.5, 0.0, // Bottom left
                    0.5, -0.5, 0.0, // Bottom right
                ];

                let vertex_data = vertices.as_ptr() as *const u8;
                let vertex_bytes = unsafe {
                    std::slice::from_raw_parts(
                        vertex_data,
                        vertices.len() * std::mem::size_of::<f32>(),
                    )
                };

                // Define vertex attributes for position (x, y, z)
                let position_attribute = VertexAttributeDefinition {
                    location: 0,
                    kind: VertexAttributeKind::Float32x3,
                    component_count: 3,
                    normalized: false,
                    divisor: 0,
                };

                let geometry_desc = GeometryBufferDescriptor {
                    name: "triangle",
                    kind: BufferKind::Vertex,
                    buffers: &[VertexBufferDescriptor {
                        kind: BufferKind::Vertex,
                        attributes: &[position_attribute],
                        data: VertexBufferData {
                            bytes: Some(vertex_bytes),
                            element_size: 3 * std::mem::size_of::<f32>(),
                        },
                    }],
                    element: ElementsDescriptor::Triangles(&[]),
                };

                self.triangle_geometry = Some(
                    ctx.renderer
                        .graphics_backend()
                        .create_geometry_buffer(geometry_desc)?,
                );
            }
        }
        Ok(())
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

                        // Set a blue clear color to show that Metal rendering is working
                        if let GraphicsContext::Initilized(ref mut ctx) =
                            engine.graphics_context
                        {
                            use moonfield::core::math::color;
                            ctx.renderer.set_clear_color(color::BLUE);
                            ctx.window.request_redraw();
                        }
                        self.engine = Some(engine);

                        // Create triangle geometry after engine is initialized
                        if let Err(e) = self.create_triangle_geometry() {
                            error!("Failed to create triangle geometry: {}", e);
                        }
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
                    // Add triangle to renderer if we have one
                    if let Some(triangle_geometry) = &self.triangle_geometry {
                        if let GraphicsContext::Initilized(ctx) =
                            &mut engine.graphics_context
                        {
                            ctx.renderer
                                .draw_geometry(triangle_geometry.clone());
                        }
                    }

                    match engine.render() {
                        Ok(()) => {
                            // Clear geometry buffers for next frame
                            if let GraphicsContext::Initilized(ctx) =
                                &mut engine.graphics_context
                            {
                                ctx.renderer.clear_geometry();
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
        // Fallback to basic tracing setup if optimized logging fails
        tracing_subscriber::fmt::init();
        error!("Failed to initialize optimized logging: {}", e);
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
