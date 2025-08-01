use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::NSView;
use objc2_core_foundation::CGSize;
use objc2_metal::{
    MTLClearColor, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLLoadAction, MTLPixelFormat, MTLRenderPassDescriptor, MTLStoreAction,
    MTLTexture,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use winit::{
    event_loop::ActiveEventLoop,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowAttributes},
};

use crate::{
    backend::{BackendCapabilities, GraphicsBackend, SharedGraphicsBackend},
    error::{GraphicsError, MetalError},
    metal_backend::frame_buffer::MetalFrameBuffer,
};

pub mod frame_buffer;

pub struct MetalGraphicsBackend {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    layer: Retained<CAMetalLayer>,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pub named_objects: Cell<bool>,
    this: RefCell<Option<Weak<MetalGraphicsBackend>>>,
}

impl MetalGraphicsBackend {
    pub fn new(
        #[allow(unused_variables)] vsync: bool,
        #[allow(unused_variables)] msaa_sample_count: Option<u8>,
        event_loop: &ActiveEventLoop, window_attrs: WindowAttributes,
        named_objects: bool,
    ) -> Result<(Window, SharedGraphicsBackend), GraphicsError> {
        // Create the window
        let window = event_loop
            .create_window(window_attrs)
            .map_err(|e| GraphicsError::WindowCreationError(e.to_string()))?;

        // Get the raw window handle
        let raw_window_handle = window
            .window_handle()
            .map_err(|e| GraphicsError::WindowCreationError(e.to_string()))?
            .as_raw();

        // Create the Metal device
        let device = MTLCreateSystemDefaultDevice().ok_or_else(|| {
            GraphicsError::MetalError(MetalError::DeviceCreationError(
                "Failed to create default Metal device".to_string(),
            ))
        })?;

        // Create the Metal command queue
        let command_queue = device.newCommandQueue().ok_or_else(|| {
            GraphicsError::MetalError(MetalError::CommandQueueError(
                "Failed to create command queue".to_string(),
            ))
        })?;

        let layer = unsafe { CAMetalLayer::new() };
        unsafe {
            layer.setDevice(Some(&device));
            layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        }
        match raw_window_handle {
            RawWindowHandle::AppKit(handle) => unsafe {
                let ns_view = handle.ns_view.as_ptr();
                if !ns_view.is_null() {
                    let view: *mut NSView = ns_view.cast();
                    (*view).setLayer(Some(&layer));
                    (*view).setWantsLayer(true);
                }
            },
            _ => {
                return Err(GraphicsError::MetalError(
                    MetalError::DeviceCreationError(
                        "Unsupported window handle type for Metal".to_string(),
                    ),
                ));
            }
        }

        let backend = Self {
            device,
            command_queue,
            layer,
            named_objects: Cell::new(named_objects),
            this: Default::default(),
        };

        // Wrap the backend in Rc<dyn GraphicsBackend>
        let shared_backend: SharedGraphicsBackend = Rc::new(backend);

        Ok((window, shared_backend))
    }
}

impl GraphicsBackend for MetalGraphicsBackend {
    fn back_buffer(
        &self,
    ) -> Result<crate::frame_buffer::SharedFrameBuffer, GraphicsError> {
        let drawable = unsafe {
            self.layer.nextDrawable().ok_or_else(|| {
                MetalError::RenderPassError(
                    "Failed to get drawable (surface and swapchain)"
                        .to_string(),
                )
            })
        }?;

        let texture = unsafe { drawable.texture() };
        let width = texture.width() as u32;
        let height = texture.height() as u32;

        let render_pass_descriptor = unsafe { MTLRenderPassDescriptor::new() };
        let color_attachment = unsafe {
            render_pass_descriptor
                .colorAttachments()
                .objectAtIndexedSubscript(0)
        };
        color_attachment.setTexture(Some(&texture));
        // Don't set LoadAction and ClearColor here - let the clear() method handle it
        color_attachment.setStoreAction(MTLStoreAction::Store);

        let command_buffer =
            self.command_queue.commandBuffer().ok_or_else(|| {
                MetalError::CommandQueueError(
                    "Failed to create command buffer".to_string(),
                )
            })?;

        let frame_buffer = MetalFrameBuffer {
            drawable,
            render_pass_descriptor,
            command_buffer,
            render_encoder: None,
            width,
            height,
        };

        Ok(Box::new(frame_buffer))
    }

    fn swap_buffers(&self) -> Result<(), GraphicsError> {
        Ok(())
    }

    fn set_frame_size(&self, new_size: (u32, u32)) {
        let (width, height) = new_size;
        unsafe {
            self.layer.setDrawableSize(CGSize {
                width: width as f64,
                height: height as f64,
            })
        };
    }

    fn capabilities(&self) -> BackendCapabilities {
        let device = &self.device;

        let max_buffer_length = device.maxBufferLength();

        BackendCapabilities { max_buffer_length }
    }
}
