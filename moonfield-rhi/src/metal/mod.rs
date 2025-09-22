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
    backend::{self, BackendCapabilities, Device as LegacyDevice, SharedGraphicsBackend}, 
    dynamic::{
        impl_dyn_object, DynObject, DynInstance, DynSurface, DynAdapter, DynDevice,
        DynResource, DynBuffer, DynTexture, DynTextureView, DynSurfaceTexture,
        DynSampler, DynAccelerationStructure, DynShaderProgram, DynShaderObject,
        DynShaderTable, DynPipeline, DynRenderPipeline, DynComputePipeline,
        DynRayTracingPipeline, DynCommandBuffer, DynCommandEncoder, DynPassEncoder,
        RenderPassEncoder as DynRenderPassEncoder, ComputePassEncoder as DynComputePassEncoder,
        DynRayTracingPassEncoder, DynCommandQueue, DynInputLayout, DynFence,
        DynQueryPool, DynPersistentCache, DynHeap
    },
    error::GraphicsError, 
    geometry_buffer::GeometryBufferWarpper, 
    metal::{
        frame_buffer::MetalFrameBuffer, geometry_buffer::MetalGeometryBuffer,
    }, 
    Backend
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
            GraphicsError::device_error(
                "Metal",
                "Failed to create default Metal device",
            )
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
            GraphicsError::command_error(
                "Metal",
                "Failed to create command queue",
            )
        })?;

        let shader_source = NSString::from_str(include_str!(
            "../../../assets/shaders/out/metal/basic_triangle.metal"
        ));

        let library = device
            .newLibraryWithSource_options_error(&shader_source, None)
            .map_err(|e| {
                GraphicsError::shader_error(
                    "Metal",
                    &format!("Failed to create shader library: {:?}", e),
                )
            })?;

        let vertex_function = library
            .newFunctionWithName(&NSString::from_str("vertex_main"))
            .ok_or_else(|| {
                GraphicsError::shader_error(
                    "Metal",
                    "Failed to find vertex_main function",
                )
            })?;

        let fragment_function = library
            .newFunctionWithName(&NSString::from_str("fragment_main"))
            .ok_or_else(|| {
                GraphicsError::shader_error(
                    "Metal",
                    "Failed to find fragment_main function",
                )
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
                GraphicsError::pipeline_error(
                    "Metal",
                    &format!("Failed to create render pipeline state: {:?}", e),
                )
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

impl LegacyDevice for MetalGraphicsBackend {
    /// Get the images in swapchain that havent been rendereds
    fn back_buffer(
        &self,
    ) -> Result<crate::frame_buffer::SharedFrameBuffer, GraphicsError> {
        // Get the swapchain+surface
        let drawable = unsafe {
            self.layer.nextDrawable().ok_or_else(|| {
                GraphicsError::SwapchainError(
                    "Failed to get drawable (surface and swapchain)"
                        .to_string(),
                )
            })
        }?;

        let texture = unsafe { drawable.texture() };
        let width = texture.width() as u32;
        let height = texture.height() as u32;
        
        tracing::debug!(
            "Metal backend: Creating framebuffer with drawable texture size: {}x{}",
            width, height
        );

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
                GraphicsError::command_error(
                    "Metal",
                    "Failed to create command buffer",
                )
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
        tracing::info!(
            "Metal backend: Setting frame size to {}x{}",
            width, height
        );
        
        unsafe {
            // Get current drawable size for comparison
            let current_size = self.layer.drawableSize();
            tracing::debug!(
                "Metal backend: Current drawable size: {}x{}, setting to: {}x{}",
                current_size.width, current_size.height, width, height
            );
            
            self.layer.setDrawableSize(CGSize {
                width: width as f64,
                height: height as f64,
            });
            
            // Verify the size was set
            let new_drawable_size = self.layer.drawableSize();
            tracing::info!(
                "Metal backend: Drawable size after setting: {}x{}",
                new_drawable_size.width, new_drawable_size.height
            );
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


#[derive(Debug)]
pub struct Instance {

}

impl DynInstance for Instance {}

#[derive(Debug)]
pub struct Surface {}
impl DynSurface for Surface {}

#[derive(Debug)]
pub struct Adapter {}
impl DynAdapter for Adapter {}

#[derive(Debug)]
pub struct Device {}
impl DynDevice for Device {}

#[derive(Debug)]
pub struct Queue {}
impl DynResource for Queue {}
impl DynCommandQueue for Queue {}

#[derive(Debug)]
pub struct CommandEncoder {}
impl DynCommandEncoder for CommandEncoder {}

#[derive(Debug)]
pub struct CommandBuffer {}
impl DynCommandBuffer for CommandBuffer {}

#[derive(Debug)]
pub struct Buffer {}
impl DynResource for Buffer {}
impl DynBuffer for Buffer {}

#[derive(Debug)]
pub struct Texture {}
impl DynResource for Texture {}
impl DynTexture for Texture {}

#[derive(Debug)]
pub struct TextureView {}
impl DynResource for TextureView {}
impl DynTextureView for TextureView {}

pub struct SurfaceTexture {
    texture: Texture,
    drawable: Retained<dyn CAMetalDrawable>,
    present_with_transaction: bool,
}

impl std::fmt::Debug for SurfaceTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceTexture")
            .field("texture", &self.texture)
            .field("drawable", &"<CAMetalDrawable>")
            .field("present_with_transaction", &self.present_with_transaction)
            .finish()
    }
}

impl DynResource for SurfaceTexture {}
impl DynSurfaceTexture for SurfaceTexture {}

impl std::borrow::Borrow<Texture> for SurfaceTexture {
    fn borrow(&self) -> &Texture {
        &self.texture
    }
}

impl core::borrow::Borrow<dyn DynTexture> for SurfaceTexture {
    fn borrow(&self) -> &dyn DynTexture {
        &self.texture
    }
}

unsafe impl Send for SurfaceTexture {}
unsafe impl Sync for SurfaceTexture {}



#[derive(Debug)]
pub struct Sampler {}
impl DynResource for Sampler {}
impl DynSampler for Sampler {}

#[derive(Debug)]
pub struct AccelerationStructure {}
impl DynResource for AccelerationStructure {}
impl DynAccelerationStructure for AccelerationStructure {}


#[derive(Debug)]
pub struct ShaderProgram {}
impl DynShaderProgram for ShaderProgram {}
#[derive(Debug)]
pub struct ShaderObject {}
impl DynShaderObject for ShaderObject {}
#[derive(Debug)]
pub struct ShaderTable {}
impl DynShaderTable for ShaderTable {}
#[derive(Debug)]
pub struct RenderPipeline {}
impl DynPipeline for RenderPipeline {}
impl DynRenderPipeline for RenderPipeline {}
#[derive(Debug)]
pub struct ComputePipeline {}
impl DynPipeline for ComputePipeline {}
impl DynComputePipeline for ComputePipeline {}
#[derive(Debug)]
pub struct RayTracingPipeline {}
impl DynPipeline for RayTracingPipeline {}  
impl DynRayTracingPipeline for RayTracingPipeline {}

#[derive(Debug)]
pub struct PassEncoder {}
impl DynPassEncoder for PassEncoder {}
#[derive(Debug)]
pub struct RenderPassEncoder {}
impl DynPassEncoder for RenderPassEncoder {}
impl DynRenderPassEncoder for RenderPassEncoder {}
#[derive(Debug)]
pub struct ComputePassEncoder {}    
impl DynPassEncoder for ComputePassEncoder {}
impl DynComputePassEncoder for ComputePassEncoder {}
#[derive(Debug)]
pub struct RayTracingPassEncoder {}
impl DynPassEncoder for RayTracingPassEncoder {}
impl DynRayTracingPassEncoder for RayTracingPassEncoder {}


#[derive(Debug)]
pub struct InputLayout {}
impl DynInputLayout for InputLayout {}

#[derive(Debug)]
pub struct Fence {}
impl DynFence for Fence {}

#[derive(Debug)]
pub struct QueryPool {}
impl DynQueryPool for QueryPool {}

#[derive(Debug)]
pub struct PersistentCache {}
impl DynPersistentCache for PersistentCache {}

#[derive(Debug)]
pub struct Heap {}
impl DynHeap for Heap {}






#[derive(Clone, Debug)]
pub struct Api;

impl crate::Api for Api {
    const VARIANT: Backend = Backend::Metal;

    type Instance = Instance;
    type Surface = Surface;
    type Adapter = Adapter;
    type Device = Device;

    type Queue = Queue;
    type CommandEncoder = CommandEncoder;
    type CommandBuffer = CommandBuffer;

    // Resource types
    type Resource = Buffer; // Use concrete type instead of trait
    type Buffer = Buffer;
    type Texture = Texture;
    type SurfaceTexture = SurfaceTexture; // Use concrete type
    type TextureView = TextureView;
    type Sampler = Sampler;
    type AccelerationStructure = AccelerationStructure;

    // Shader and pipeline types
    type ShaderProgram = ShaderProgram;
    type ShaderObject = ShaderObject;
    type ShaderTable = ShaderTable;
    type RenderPipeline = RenderPipeline;
    type ComputePipeline = ComputePipeline;
    type RayTracingPipeline = RayTracingPipeline;

    // Pass encoder types
    type PassEncoder = PassEncoder;
    type RenderPassEncoder = RenderPassEncoder;
    type ComputePassEncoder = ComputePassEncoder;
    type RayTracingPassEncoder = RayTracingPassEncoder;

    // Other resource types
    type InputLayout = InputLayout;
    type Fence = Fence;
    type QueryPool = QueryPool;
    type PersistentCache = PersistentCache;
    type Heap = Heap;
}

impl_dyn_object!(
    Instance,
    Surface,
    Adapter,
    Device,
    Queue,
    CommandEncoder,
    CommandBuffer,
    Buffer,
    Texture,
    TextureView,
    SurfaceTexture,
    Sampler,
    AccelerationStructure,
    ShaderProgram,
    ShaderObject,
    ShaderTable,
    RenderPipeline,
    ComputePipeline,
    RayTracingPipeline,
    PassEncoder,
    RenderPassEncoder,
    ComputePassEncoder,
    RayTracingPassEncoder,
    InputLayout,
    Fence,
    QueryPool,
    PersistentCache,
    Heap
);
