use std::sync::Arc;

use moonfield_core::asset::{AssetHandle, AssetServer, ShaderAsset};
use moonfield_rhi::{types::Backend, *};
use shader_slang::Stage;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 3],
}

struct TriangleApp {
    window: Option<Arc<Window>>,
    _instance: Option<Arc<dyn Instance>>,
    _surface: Option<Arc<dyn Surface>>,
    device: Option<Arc<dyn Device>>,
    swapchain: Option<Arc<dyn Swapchain>>,
    pipeline: Option<Arc<dyn Pipeline>>,
    vertex_buffer: Option<Arc<dyn Buffer>>,
    command_pool: Option<Arc<dyn CommandPool>>,
    queue: Option<Arc<dyn Queue>>,
    asset_server: Option<AssetServer>,
    vertex_shader_handle: Option<AssetHandle<ShaderAsset>>,
    fragment_shader_handle: Option<AssetHandle<ShaderAsset>>,
}

impl Default for TriangleApp {
    fn default() -> Self {
        Self {
            window: None,
            _instance: None,
            _surface: None,
            device: None,
            swapchain: None,
            pipeline: None,
            vertex_buffer: None,
            command_pool: None,
            queue: None,
            asset_server: None,
            vertex_shader_handle: None,
            fragment_shader_handle: None,
        }
    }
}

impl Drop for TriangleApp {
    fn drop(&mut self) {
        self.fragment_shader_handle = None;
        self.vertex_shader_handle = None;
        self.asset_server = None;
        self.command_pool = None;
        self.queue = None;
        self.vertex_buffer = None;
        self.pipeline = None;
        self.swapchain = None;
        self.device = None;
        self._surface = None;
        self._instance = None;
        self.window = None;
    }
}

impl ApplicationHandler for TriangleApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = WindowAttributes::default()
            .with_title("Triangle Example")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600));

        let window = Arc::new(event_loop.create_window(window_attrs).unwrap());

        let backend = Backend::Vulkan;

        let instance = create_instance_with_window(backend, &window).unwrap();
        let surface = instance.create_surface(&window).unwrap();

        let adapters = instance.enumerate_adapters();
        let adapter = adapters.first().expect("No adapters found");

        tracing::info!("Using adapter: {:?}", adapter.get_properties());

        let device = adapter.request_device().unwrap();

        let capabilities = surface.get_capabilities(adapter.as_ref());
        let format = capabilities.formats[0];

        let window_size = window.inner_size();
        let extent = types::Extent2D {
            width: window_size.width,
            height: window_size.height,
        };

        let swapchain = device
            .create_swapchain(&types::SwapchainDescriptor {
                surface: surface.clone(),
                format,
                extent,
                present_mode: types::PresentMode::Fifo,
                image_count: capabilities.min_image_count.max(2),
            })
            .unwrap();

        // Initialize asset server and register loaders
        let mut asset_server = AssetServer::new();
        asset_server
            .register_loader(Box::new(moonfield_core::asset::ShaderLoader));

        // Load shaders using the asset system
        // Build path relative to the project root
        // CARGO_MANIFEST_DIR is the directory of the Cargo.toml file for this example
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let project_root = manifest_dir.parent().unwrap().parent().unwrap(); // Go up twice to reach project root

        // Use SPIR-V files for Vulkan/Metal backends
        let vertex_shader_path =
            project_root.join("assets/shaders/out/spir-v/triangle.vert.spv");
        let fragment_shader_path =
            project_root.join("assets/shaders/out/spir-v/triangle.frag.spv");

        let vertex_shader_handle = asset_server
            .load::<ShaderAsset>(vertex_shader_path.to_str().unwrap())
            .unwrap();
        let fragment_shader_handle = asset_server
            .load::<ShaderAsset>(fragment_shader_path.to_str().unwrap())
            .unwrap();

        let vertices = vec![
            Vertex { position: [0.0, -0.5], color: [1.0, 0.0, 0.0] },
            Vertex { position: [0.5, 0.5], color: [0.0, 1.0, 0.0] },
            Vertex { position: [-0.5, 0.5], color: [0.0, 0.0, 1.0] },
        ];

        let vertex_buffer = device
            .create_buffer(&types::BufferDescriptor {
                size: (std::mem::size_of::<Vertex>() * vertices.len()) as u64,
                usage: types::BufferUsage::Vertex,
                memory_location: types::MemoryLocation::CpuToGpu,
            })
            .unwrap();

        unsafe {
            let ptr = vertex_buffer.map().unwrap();
            std::ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                ptr,
                std::mem::size_of::<Vertex>() * vertices.len(),
            );
            vertex_buffer.unmap();
        }

        // Get shader data from asset system
        let vertex_shader_asset =
            asset_server.get(&vertex_shader_handle).unwrap();
        let fragment_shader_asset =
            asset_server.get(&fragment_shader_handle).unwrap();

        let vertex_shader = device
            .create_shader_module(&types::ShaderModuleDescriptor {
                code: &vertex_shader_asset.source,
                stage: Stage::Vertex,
            })
            .unwrap();

        let fragment_shader = device
            .create_shader_module(&types::ShaderModuleDescriptor {
                code: &fragment_shader_asset.source,
                stage: Stage::Pixel,
            })
            .unwrap();

        let pipeline = device
            .create_pipeline(&types::GraphicsPipelineDescriptor {
                vertex_shader,
                fragment_shader,
                vertex_input: types::VertexInputDescriptor {
                    bindings: vec![types::VertexInputBinding {
                        binding: 0,
                        stride: std::mem::size_of::<Vertex>() as u32,
                        input_rate: types::VertexInputRate::Vertex,
                    }],
                    attributes: vec![
                        types::VertexInputAttribute {
                            location: 0,
                            binding: 0,
                            format: types::VertexFormat::Float32x2,
                            offset: 0,
                        },
                        types::VertexInputAttribute {
                            location: 1,
                            binding: 0,
                            format: types::VertexFormat::Float32x3,
                            offset: 8,
                        },
                    ],
                },
                render_pass_format: format,
            })
            .unwrap();

        let command_pool = device.create_command_pool(&swapchain).unwrap();
        let queue = device.get_queue();

        self.window = Some(window);
        self._instance = Some(instance);
        self._surface = Some(surface);
        self.device = Some(device);
        self.swapchain = Some(swapchain);
        self.pipeline = Some(pipeline);
        self.vertex_buffer = Some(vertex_buffer);
        self.command_pool = Some(command_pool);
        self.queue = Some(queue);
        self.asset_server = Some(asset_server);
        self.vertex_shader_handle = Some(vertex_shader_handle);
        self.fragment_shader_handle = Some(fragment_shader_handle);
    }

    fn window_event(
        &mut self, event_loop: &ActiveEventLoop, _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let swapchain = self.swapchain.as_ref().unwrap();
                let command_pool = self.command_pool.as_ref().unwrap();
                let queue = self.queue.as_ref().unwrap();
                let pipeline = self.pipeline.as_ref().unwrap();
                let vertex_buffer = self.vertex_buffer.as_ref().unwrap();
                let window = self.window.as_ref().unwrap();

                let image = match swapchain.acquire_next_image() {
                    Ok(img) => img,
                    Err(e) => {
                        tracing::error!("Failed to acquire image: {:?}", e);
                        return;
                    }
                };

                let command_buffer =
                    command_pool.allocate_command_buffer().unwrap();
                command_buffer.begin().unwrap();

                let size = window.inner_size();
                command_buffer
                    .set_viewport(size.width as f32, size.height as f32);
                command_buffer.set_scissor(size.width, size.height);

                command_buffer.begin_render_pass(
                    &types::RenderPassDescriptor {
                        color_attachments: vec![types::ColorAttachment {
                            load_op: types::LoadOp::Clear,
                            store_op: types::StoreOp::Store,
                            clear_value: [0.5, 0.5, 0.5, 1.0],
                        }],
                    },
                    &image,
                );

                command_buffer.bind_pipeline(pipeline.as_ref());
                command_buffer.bind_vertex_buffer(vertex_buffer.as_ref());
                command_buffer.draw(3, 1, 0, 0);

                command_buffer.end_render_pass();
                command_buffer.end().unwrap();

                queue
                    .submit(
                        &[command_buffer],
                        Some(image.wait_semaphore),
                        Some(image.signal_semaphore),
                    )
                    .unwrap();

                if let Err(e) = swapchain.present(image) {
                    tracing::error!("Failed to present: {:?}", e);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing logging system from moonfield-core
    moonfield_core::logging::init_auto_logging()
        .expect("Failed to initialize logging system");

    tracing::info!("Starting triangle example application");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = TriangleApp::default();
    event_loop.run_app(&mut app)?;

    tracing::info!("Application exited");
    Ok(())
}
