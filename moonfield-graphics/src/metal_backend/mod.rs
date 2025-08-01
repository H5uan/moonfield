use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::NSView;
use objc2_metal::{MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat};
use objc2_quartz_core::CAMetalLayer;
use winit::{
    event_loop::ActiveEventLoop,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowAttributes},
};

use crate::{
    backend::{GraphicsBackend, SharedGraphicsBackend},
    error::{GraphicsError, MetalError},
};

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
        event_loop: &ActiveEventLoop,
        window_attrs: WindowAttributes,
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
                return Err(GraphicsError::MetalError(MetalError::DeviceCreationError(
                    "Unsupported window handle type for Metal".to_string(),
                )));
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

impl GraphicsBackend for MetalGraphicsBackend {}
