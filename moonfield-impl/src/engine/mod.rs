use std::rc::Rc;

use moonfield_graphics::{
    backend::SharedGraphicsBackend, error::GraphicsError,
};

#[cfg(feature = "metal")]
use moonfield_graphics::metal_backend::MetalGraphicsBackend;
#[cfg(feature = "vulkan")]
use moonfield_graphics::vulkan_backend::VulkanGraphicsBackend;
use tracing::{debug, error, info, instrument, warn};
use winit::{
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes},
};

use crate::{engine::error::EngineError, renderer::Renderer};

pub mod error;

pub struct InitilizedGraphicsContext {
    pub window: Window,
    pub renderer: Renderer,
    params: GraphicsContextParams,
}

pub type GraphicsBackendConstructorResult =
    Result<(Window, SharedGraphicsBackend), GraphicsError>;

pub type GraphicsBackendConstructorCallback =
    dyn Fn(
        &GraphicsContextParams,
        &ActiveEventLoop,
        WindowAttributes,
        bool,
    ) -> GraphicsBackendConstructorResult;

#[derive(Clone)]
pub struct GraphicsBackendConstructor(Rc<GraphicsBackendConstructorCallback>);

impl Default for GraphicsBackendConstructor {
    fn default() -> Self {
        // Try Metal backend first if available (typically on macOS)
        #[cfg(all(feature = "metal", target_os = "macos"))]
        {
            Self(Rc::new(|params, event_loop, window_attrs, named_objects| {
                MetalGraphicsBackend::new(
                    params.vsync,
                    params.msaa_sample_count,
                    event_loop,
                    window_attrs,
                    named_objects,
                )
            }))
        }
        // Use Vulkan backend if Metal is not available but Vulkan is enabled
        #[cfg(all(feature = "vulkan", not(all(feature = "metal", target_os = "macos"))))]
        {
            Self(Rc::new(|params, event_loop, window_attrs, named_objects| {
                VulkanGraphicsBackend::new(
                    params.vsync,
                    params.msaa_sample_count,
                    event_loop,
                    window_attrs,
                    named_objects,
                )
            }))
        }
        // Fall back to unavailable if no backend is enabled
        #[cfg(not(any(
            all(feature = "metal", target_os = "macos"),
            feature = "vulkan"
        )))]
        {
            Self(Rc::new(|_params, _event_loop, _window_attrs, _named_objects| {
                Err(GraphicsError::BackendUnavailable)
            }))
        }
    }
}

#[derive(Clone)]
pub struct GraphicsContextParams {
    pub window_attributes: WindowAttributes,

    pub vsync: bool,

    pub msaa_sample_count: Option<u8>,

    pub graphics_backend_constructor: GraphicsBackendConstructor,

    // To assign meaningful names for GPU objects
    pub named_objects: bool,
}

#[allow(clippy::large_enum_variant)]
pub enum GraphicsContext {
    Initilized(InitilizedGraphicsContext),
    UnInitialized(GraphicsContextParams),
}

pub struct EngineInitParams {
    pub graphics_context_params: GraphicsContextParams,
}

pub struct Engine {
    pub graphics_context: GraphicsContext,

    // Amount of time (in seconds) that passed from creation of the engine
    elapsed_time: f32,
}

impl Engine {
    #[instrument(skip(params))]
    pub fn new(params: EngineInitParams) -> Result<Self, EngineError> {
        info!("Creating new Moonfield engine");

        let EngineInitParams { graphics_context_params } = params;

        debug!("Engine created with graphics context parameters");
        Ok(Self {
            graphics_context: GraphicsContext::UnInitialized(
                graphics_context_params,
            ),
            elapsed_time: 0.0,
        })
    }

    #[instrument(skip(self, active_event_loop))]
    pub fn initialize_graphics_context(
        &mut self, active_event_loop: &ActiveEventLoop,
    ) -> Result<(), EngineError> {
        info!("Initializing graphics context");

        if let GraphicsContext::UnInitialized(params) = &self.graphics_context {
            debug!("Creating graphics backend and window");
            let (window, backend) = params.graphics_backend_constructor.0(
                params,
                active_event_loop,
                params.window_attributes.clone(),
                params.named_objects,
            )?;

            let frame_size =
                (window.inner_size().width, window.inner_size().height);
            info!(
                "Window created with size: {}x{}",
                frame_size.0, frame_size.1
            );

            debug!("Creating renderer");
            let renderer = Renderer::new(backend, frame_size)?;

            self.graphics_context =
                GraphicsContext::Initilized(InitilizedGraphicsContext {
                    window,
                    renderer,
                    params: params.clone(),
                });

            info!("Graphics context initialized successfully");
            Ok(())
        } else {
            warn!(
                "Attempted to initialize graphics context when already initialized"
            );
            Err(EngineError::Custom(
                "Graphics context is already initialized".to_string(),
            ))
        }
    }

    pub fn render(&mut self) -> Result<(), GraphicsError> {
        if let GraphicsContext::Initilized(ref mut ctx) = self.graphics_context
        {
            ctx.renderer.render_frame()?;
        }

        Ok(())
    }
}
