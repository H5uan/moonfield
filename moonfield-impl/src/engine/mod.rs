use std::rc::Rc;

use tracing::{debug, info, warn};
use winit::{
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes},
};

use crate::engine::error::EngineError;

pub mod error;

pub struct InitilizedGraphicsContext {
    pub window: Window,
    params: GraphicsContextParams,
}

#[derive(Clone)]
pub struct GraphicsContextParams {
    pub window_attributes: WindowAttributes,
    pub vsync: bool,
    pub msaa_sample_count: Option<u8>,
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
    elapsed_time: f32,
}

impl Engine {
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

    pub fn initialize_graphics_context(
        &mut self, active_event_loop: &ActiveEventLoop,
    ) -> Result<(), EngineError> {
        info!("Initializing graphics context");

        if let GraphicsContext::UnInitialized(params) = &self.graphics_context {
            debug!("Creating window");
            let window = active_event_loop
                .create_window(params.window_attributes.clone())
                .map_err(|e| {
                    EngineError::Custom(format!(
                        "Failed to create window: {}",
                        e
                    ))
                })?;

            let frame_size =
                (window.inner_size().width, window.inner_size().height);
            info!(
                "Window created with size: {}x{}",
                frame_size.0, frame_size.1
            );

            self.graphics_context =
                GraphicsContext::Initilized(InitilizedGraphicsContext {
                    window,
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

    pub fn render(&mut self) -> Result<(), EngineError> {
        // Rendering logic removed - no backend available
        Ok(())
    }

    pub fn handle_window_resize(
        &mut self, new_size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<(), EngineError> {
        if let GraphicsContext::Initilized(ref mut ctx) = self.graphics_context
        {
            let frame_size = (new_size.width, new_size.height);

            if frame_size.0 == 0 || frame_size.1 == 0 {
                return Ok(()); // ignore minimized case
            }

            ctx.window.request_redraw();
            Ok(())
        } else {
            Err(EngineError::Custom(
                "Graphics context not initialized".to_string(),
            ))
        }
    }
}
