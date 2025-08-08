use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::NSView;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLLibrary, MTLLoadAction, MTLPixelFormat, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLStoreAction,
    MTLTexture, MTLVertexDescriptor, MTLVertexFormat, MTLVertexStepFunction,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use winit::{
    event_loop::ActiveEventLoop,
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowAttributes},
};

use crate::{
    backend::{self, BackendCapabilities, Device, SharedGraphicsBackend},
    error::GraphicsError,
    geometry_buffer::GeometryBufferWarpper,
    metal::{
        frame_buffer::MetalFrameBuffer, geometry_buffer::MetalGeometryBuffer,
    },
};

pub mod buffer;
pub mod frame_buffer;
pub mod geometry_buffer;

pub struct MetalGraphicsBackend {
    /// device: abstraction of the GPU, providing methods for creating objects managed by GPU
    /// like command queues, render states and shader liberaries
    /// for apple device, ususally call `MTLCreateSystemDefaultDevice()` is suffient
    /// since most apple device only have a single GPU
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    /// layer: the combination of surface and swapchain in vulkan
    layer: Retained<CAMetalLayer>,
    /// command_queue: A list of render command buffers to executed.
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    /// pipeline_state: We store the pipeline setting into an object,
    /// so that we donot need to do run-time check before draw call
    pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    pub(crate) named_objects: Cell<bool>,
    /// self reference: mostly is for other objects like buffer, texture to use
    /// since backend might manage buffer, it cannot be a strong reference
    /// the safe way is to tranfer its own weak reference
    this: RefCell<Option<Weak<MetalGraphicsBackend>>>,
}

impl MetalGraphicsBackend {
    /// Get access to the Metal device
    pub fn device(&self) -> &Retained<ProtocolObject<dyn MTLDevice>> {
        &self.device
    }

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
            GraphicsError::device_error("Metal", "Failed to create default Metal device")
        })?;

        // Create the Metal layer. Layer(suface & swapchain) must know which device will draw on layer
        // and the pixel format for the rendering image
        let layer = unsafe { CAMetalLayer::new() };
        unsafe {
            layer.setDevice(Some(&device));
            // At apple platform, BGRA is a default order. RGBA will do a extra copy or swizzle
            layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        }

        // Create the Metal command queue
        let command_queue = device.newCommandQueue().ok_or_else(|| {
            GraphicsError::command_error("Metal", "Failed to create command queue")
        })?;

        let shader_source = NSString::from_str(include_str!(
            "../../../assets/shaders/out/metal/basic_triangle.metal"
        ));

        let library = device
            .newLibraryWithSource_options_error(&shader_source, None)
            .map_err(|e| {
                GraphicsError::shader_error("Metal", &format!("Failed to create shader library: {:?}", e))
            })?;

        let vertex_function = library
            .newFunctionWithName(&NSString::from_str("vertex_main"))
            .ok_or_else(|| {
                GraphicsError::shader_error("Metal", "Failed to find vertex_main function")
            })?;

        let fragment_function = library
            .newFunctionWithName(&NSString::from_str("fragment_main"))
            .ok_or_else(|| {
                GraphicsError::shader_error("Metal", "Failed to find fragment_main function")
            })?;

        let vertex_descriptor = unsafe { MTLVertexDescriptor::new() };
        unsafe {
            let attributes = vertex_descriptor.attributes();
            let attribute0 = attributes.objectAtIndexedSubscript(0);
            attribute0.setFormat(MTLVertexFormat::Float3); // float3
            attribute0.setOffset(0);
            attribute0.setBufferIndex(0);

            let layouts = vertex_descriptor.layouts();
            let layout0 = layouts.objectAtIndexedSubscript(0);
            layout0.setStride(12); // 3 * sizeof(f32) = 12 bytes
            layout0.setStepFunction(MTLVertexStepFunction::PerVertex);
        }

        // Create the Metal pipeline state
        let pipeline_descriptor = unsafe { MTLRenderPipelineDescriptor::new() };

        unsafe {
            pipeline_descriptor.setVertexFunction(Some(&vertex_function));
            pipeline_descriptor.setFragmentFunction(Some(&fragment_function));
            pipeline_descriptor.setVertexDescriptor(Some(&vertex_descriptor));

            let color_attachments = pipeline_descriptor.colorAttachments();

            let color_attachment =
                color_attachments.objectAtIndexedSubscript(0);
            color_attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        }

        let pipeline_state = device
            .newRenderPipelineStateWithDescriptor_error(&pipeline_descriptor)
            .map_err(|e| {
                GraphicsError::pipeline_error("Metal", &format!("Failed to create render pipeline state: {:?}", e))
            })?;

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
                return Err(GraphicsError::device_error(
                    "Metal",
                    "Unsupported window handle type for Metal",
                ));
            }
        }

        let backend = Self {
            device,
            command_queue,
            layer,
            pipeline_state,
            named_objects: Cell::new(named_objects),
            this: Default::default(),
        };

        // Wrap the backend in Rc<dyn GraphicsBackend>
        let shared_backend = Rc::new(backend);

        *shared_backend.this.borrow_mut() =
            Some(Rc::downgrade(&shared_backend));

        Ok((window, shared_backend))
    }

    pub fn weak(&self) -> Weak<Self> {
        self.this.borrow().as_ref().unwrap().clone()
    }
}

impl Device for MetalGraphicsBackend {
    /// Get the images in swapchain that havent been rendereds
    fn back_buffer(
        &self,
    ) -> Result<crate::frame_buffer::SharedFrameBuffer, GraphicsError> {
        // Get the swapchain+surface
        let drawable = unsafe {
            self.layer.nextDrawable().ok_or_else(|| {
                GraphicsError::SwapchainError(
                    "Failed to get drawable (surface and swapchain)".to_string(),
                )
            })
        }?;

        let texture = unsafe { drawable.texture() };
        let width = texture.width() as u32;
        let height = texture.height() as u32;

        // Descriptor is like a guide that tell GPU how to handle old data(load)
        // and new data(store)
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
                GraphicsError::command_error("Metal", "Failed to create command buffer")
            })?;

        let frame_buffer = MetalFrameBuffer {
            backend: self.weak(),
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
        // At apple platform, we do not need to swap buffers manually
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

    fn create_geometry_buffer(
        &self, desc: crate::geometry_buffer::GeometryBufferDescriptor,
    ) -> Result<crate::geometry_buffer::GeometryBufferWarpper, GraphicsError>
    {
        let geometry_buffer = MetalGeometryBuffer::new(self, desc)?;
        Ok(GeometryBufferWarpper(Rc::new(geometry_buffer)))
    }
}
