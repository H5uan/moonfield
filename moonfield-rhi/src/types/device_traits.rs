//! Device and command-related trait definitions

use super::*;
use std::ffi::c_void;

/// Command queue type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueType {
    Graphics,
}

/// Device information
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub device_type: DeviceType,
    pub limits: DeviceLimits,
    /// The name of the graphics API being used by this device.
    pub api_name: String,
    /// The name of the graphics adapter.
    pub adapter_name: String,
    /// The LUID of the graphics adapter.
    pub adapter_luid: AdapterLUID,
    /// The clock frequency used in timestamp queries.
    pub timestamp_frequency: u64,
}

/// Device limits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLimits {
    /// Maximum buffer size in bytes.
    pub max_buffer_size: u64,
    /// Maximum dimension for 1D textures.
    pub max_texture_dimension_1d: u32,
    /// Maximum dimensions for 2D textures.
    pub max_texture_dimension_2d: u32,
    /// Maximum dimensions for 3D textures.
    pub max_texture_dimension_3d: u32,
    /// Maximum dimensions for cube textures.
    pub max_texture_dimension_cube: u32,
    /// Maximum number of texture layers.
    pub max_texture_layers: u32,
    /// Maximum number of vertex input elements in a graphics pipeline.
    pub max_vertex_input_elements: u32,
    /// Maximum offset of a vertex input element in the vertex stream.
    pub max_vertex_input_element_offset: u32,
    /// Maximum number of vertex streams in a graphics pipeline.
    pub max_vertex_streams: u32,
    /// Maximum stride of a vertex stream.
    pub max_vertex_stream_stride: u32,
    /// Maximum number of threads per thread group.
    pub max_compute_threads_per_group: u32,
    /// Maximum dimensions of a thread group.
    pub max_compute_thread_group_size: [u32; 3],
    /// Maximum number of thread groups per dimension in a single dispatch.
    pub max_compute_dispatch_thread_groups: [u32; 3],
    /// Maximum number of viewports per pipeline.
    pub max_viewports: u32,
    /// Maximum viewport dimensions.
    pub max_viewport_dimensions: [u32; 2],
    /// Maximum framebuffer dimensions.
    pub max_framebuffer_dimensions: [u32; 3],
    /// Maximum samplers visible in a shader stage.
    pub max_shader_visible_samplers: u32,
}

/// Device descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceDesc {
    pub struct_type: StructType,
    pub next: Option<*const c_void>,
    
    /// The underlying API/Platform of the device.
    pub device_type: DeviceType,
    /// The device's handles (if they exist) and their associated API.
    pub existing_device_handles: DeviceNativeHandles,
    /// LUID of the adapter to use.
    pub adapter_luid: Option<AdapterLUID>,
    /// Number of required features.
    pub required_features: Vec<String>,
    
    /// NVAPI shader extension uav slot (-1 disables the extension).
    pub nvapi_ext_uav_slot: u32,
    /// NVAPI shader extension register space.
    pub nvapi_ext_register_space: u32,
    
    /// Enable RHI validation layer.
    pub enable_validation: bool,
    /// Enable backend API raytracing validation layer.
    pub enable_ray_tracing_validation: bool,
    
    /// Enable reporting of shader compilation timings.
    pub enable_compilation_reports: bool,
    
    /// Size of a page in staging heap.
    pub staging_heap_page_size: Size,
}

impl Default for DeviceDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::DeviceDesc,
            next: None,
            device_type: DeviceType::Default,
            existing_device_handles: DeviceNativeHandles::default(),
            adapter_luid: None,
            required_features: Vec::new(),
            nvapi_ext_uav_slot: u32::MAX,
            nvapi_ext_register_space: 0,
            enable_validation: false,
            enable_ray_tracing_validation: false,
            enable_compilation_reports: false,
            staging_heap_page_size: 16 * 1024 * 1024,
        }
    }
}

/// Device native handles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceNativeHandles {
    pub handles: [NativeHandle; 3],
}


/// Render state for draw commands
#[derive(Debug, Clone, PartialEq)]
pub struct RenderState {
    pub stencil_ref: u32,
    pub viewports: Vec<Viewport>,
    pub scissor_rects: Vec<ScissorRect>,
    pub vertex_buffers: Vec<BufferOffsetPair>,
    pub index_buffer: Option<BufferOffsetPair>,
    pub index_format: IndexFormat,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            stencil_ref: 0,
            viewports: Vec::new(),
            scissor_rects: Vec::new(),
            vertex_buffers: Vec::new(),
            index_buffer: None,
            index_format: IndexFormat::Uint32,
        }
    }
}

/// Viewport specification
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub origin_x: f32,
    pub origin_y: f32,
    pub extent_x: f32,
    pub extent_y: f32,
    pub min_z: f32,
    pub max_z: f32,
}

impl Viewport {
    pub fn from_size(width: f32, height: f32) -> Self {
        Self {
            origin_x: 0.0,
            origin_y: 0.0,
            extent_x: width,
            extent_y: height,
            min_z: 0.0,
            max_z: 1.0,
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            origin_x: 0.0,
            origin_y: 0.0,
            extent_x: 0.0,
            extent_y: 0.0,
            min_z: 0.0,
            max_z: 1.0,
        }
    }
}

/// Scissor rectangle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScissorRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl ScissorRect {
    pub fn from_size(width: u32, height: u32) -> Self {
        Self {
            min_x: 0,
            min_y: 0,
            max_x: width,
            max_y: height,
        }
    }
}


/// Buffer with offset pair
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferOffsetPair {
    pub buffer: Option<*mut c_void>, // IBuffer*
    pub offset: Offset,
}

impl BufferOffsetPair {
    pub fn new(buffer: *mut c_void, offset: Offset) -> Self {
        Self {
            buffer: Some(buffer),
            offset,
        }
    }
    
    pub fn is_valid(&self) -> bool {
        self.buffer.is_some()
    }
}


/// Draw arguments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawArguments {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub start_vertex_location: u32,
    pub start_instance_location: u32,
    pub start_index_location: u32,
}

impl Default for DrawArguments {
    fn default() -> Self {
        Self {
            vertex_count: 0,
            instance_count: 1,
            start_vertex_location: 0,
            start_instance_location: 0,
            start_index_location: 0,
        }
    }
}

/// Device trait - the main RHI device interface
pub trait Device: RHIObject {
    /// Get information about the device.
    fn get_info(&self) -> &DeviceInfo;
    
    /// Get device type
    fn get_device_type(&self) -> DeviceType {
        self.get_info().device_type
    }
    
    /// Get native device handles
    fn get_native_device_handles(&self) -> Result<DeviceNativeHandles, RHIError>;
    
    /// Returns a list of features supported by the device.
    fn get_features(&self) -> Result<Vec<Feature>, RHIError>;
    
    /// Check if device has a specific feature
    fn has_feature(&self, feature: Feature) -> bool;
    
    /// Check if device has a feature by name
    fn has_feature_by_name(&self, feature: &str) -> bool;
    
    /// Get format support information
    fn get_format_support(&self, format: Format) -> Result<FormatSupport, RHIError>;
    
    /// Create a texture resource
    fn create_texture(
        &self,
        desc: &TextureDesc,
        init_data: Option<&[SubresourceData]>,
    ) -> Result<Box<dyn Texture>, RHIError>;
    
    /// Create a buffer resource
    fn create_buffer(
        &self,
        desc: &BufferDesc,
        init_data: Option<&[u8]>,
    ) -> Result<Box<dyn Buffer>, RHIError>;
    
    /// Create a sampler
    fn create_sampler(&self, desc: &SamplerDesc) -> Result<Box<dyn Sampler>, RHIError>;
    
    /// Create a texture view
    fn create_texture_view(
        &self,
        texture: &dyn Texture,
        desc: &TextureViewDesc,
    ) -> Result<Box<dyn TextureView>, RHIError>;
    
    /// Create a surface for presentation
    fn create_surface(&self, window_handle: WindowHandle) -> Result<Box<dyn Surface>, RHIError>;
    
    /// Create an input layout
    fn create_input_layout(&self, desc: &InputLayoutDesc) -> Result<Box<dyn InputLayout>, RHIError>;
    
    /// Get a command queue
    fn get_queue(&self, queue_type: QueueType) -> Result<Box<dyn CommandQueue>, RHIError>;
    
    /// Create a shader program
    fn create_shader_program(&self, desc: &ShaderProgramDesc) -> Result<Box<dyn ShaderProgram>, RHIError>;
    
    /// Create a render pipeline
    fn create_render_pipeline(&self, desc: &RenderPipelineDesc) -> Result<Box<dyn RenderPipeline>, RHIError>;
    
    /// Create a compute pipeline
    fn create_compute_pipeline(&self, desc: &ComputePipelineDesc) -> Result<Box<dyn ComputePipeline>, RHIError>;
    
    /// Create a ray tracing pipeline
    fn create_ray_tracing_pipeline(&self, desc: &RayTracingPipelineDesc) -> Result<Box<dyn RayTracingPipeline>, RHIError>;
    
    /// Create a query pool
    fn create_query_pool(&self, desc: &QueryPoolDesc) -> Result<Box<dyn QueryPool>, RHIError>;
    
    /// Create an acceleration structure
    fn create_acceleration_structure(&self, desc: &AccelerationStructureDesc) -> Result<Box<dyn AccelerationStructure>, RHIError>;
    
    /// Create a fence
    fn create_fence(&self, desc: &FenceDesc) -> Result<Box<dyn Fence>, RHIError>;
    
    /// Create a heap
    fn create_heap(&self, desc: &HeapDesc) -> Result<Box<dyn Heap>, RHIError>;
    
    /// Map buffer memory
    fn map_buffer(&self, buffer: &dyn Buffer, mode: CpuAccessMode) -> Result<*mut c_void, RHIError>;
    
    /// Unmap buffer memory
    fn unmap_buffer(&self, buffer: &dyn Buffer) -> Result<(), RHIError>;
    
    /// Wait for fences to signal
    fn wait_for_fences(
        &self,
        fences: &[&dyn Fence],
        fence_values: &[u64],
        wait_for_all: bool,
        timeout: u64,
    ) -> Result<(), RHIError>;
}

/// Command queue trait
pub trait CommandQueue: RHIObject {
    /// Get the queue type
    fn get_type(&self) -> QueueType;
    
    /// Create a command encoder
    fn create_command_encoder(&self) -> Result<Box<dyn CommandEncoder>, RHIError>;
    
    /// Submit command buffers
    fn submit(&self, command_buffers: &[&dyn CommandBuffer]) -> Result<(), RHIError>;
    
    /// Wait on host for all operations to complete
    fn wait_on_host(&self) -> Result<(), RHIError>;
    
    /// Get native handle
    fn get_native_handle(&self) -> Result<NativeHandle, RHIError>;
}

/// Command encoder trait
pub trait CommandEncoder: RHIObject {
    /// Begin a render pass
    fn begin_render_pass(&mut self, desc: &RenderPassDesc) -> Result<Box<dyn RenderPassEncoder>, RHIError>;
    
    /// Begin a compute pass
    fn begin_compute_pass(&mut self) -> Result<Box<dyn ComputePassEncoder>, RHIError>;
    
    /// Begin a ray tracing pass
    fn begin_ray_tracing_pass(&mut self) -> Result<Box<dyn RayTracingPassEncoder>, RHIError>;
    
    /// Copy buffer to buffer
    fn copy_buffer(
        &mut self,
        dst: &dyn Buffer,
        dst_offset: Offset,
        src: &dyn Buffer,
        src_offset: Offset,
        size: Size,
    ) -> Result<(), RHIError>;
    
    /// Copy texture to texture
    fn copy_texture(
        &mut self,
        dst: &dyn Texture,
        dst_subresource: SubresourceRange,
        dst_offset: Offset3D,
        src: &dyn Texture,
        src_subresource: SubresourceRange,
        src_offset: Offset3D,
        extent: Extent3D,
    ) -> Result<(), RHIError>;
    
    /// Finish encoding and get command buffer
    fn finish(&mut self) -> Result<Box<dyn CommandBuffer>, RHIError>;
    
    /// Get native handle
    fn get_native_handle(&self) -> Result<NativeHandle, RHIError>;
}

/// Base pass encoder trait
pub trait PassEncoder: RHIObject {
    /// Push debug group
    fn push_debug_group(&mut self, name: &str, color: &MarkerColor);
    
    /// Pop debug group
    fn pop_debug_group(&mut self);
    
    /// Insert debug marker
    fn insert_debug_marker(&mut self, name: &str, color: &MarkerColor);
    
    /// Write timestamp
    fn write_timestamp(&mut self, query_pool: &dyn QueryPool, query_index: u32);
    
    /// End the pass
    fn end(&mut self);
}

/// Render pass encoder trait
pub trait RenderPassEncoder: PassEncoder {
    /// Bind render pipeline
    fn bind_pipeline(&mut self, pipeline: &dyn RenderPipeline);
    
    /// Set render state
    fn set_render_state(&mut self, state: &RenderState);
    
    /// Draw primitives
    fn draw(&mut self, args: &DrawArguments);
    
    /// Draw indexed primitives
    fn draw_indexed(&mut self, args: &DrawArguments);
}

/// Compute pass encoder trait
pub trait ComputePassEncoder: PassEncoder {
    /// Bind compute pipeline
    fn bind_pipeline(&mut self, pipeline: &dyn ComputePipeline);
    
    /// Dispatch compute work
    fn dispatch_compute(&mut self, x: u32, y: u32, z: u32);
}

/// Ray tracing pass encoder trait
pub trait RayTracingPassEncoder: PassEncoder {
    /// Bind ray tracing pipeline
    fn bind_pipeline(&mut self, pipeline: &dyn RayTracingPipeline, shader_table: &dyn ShaderTable);
    
    /// Dispatch rays
    fn dispatch_rays(&mut self, ray_gen_shader_index: u32, width: u32, height: u32, depth: u32);
}

/// Render pass descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPassDesc {
    pub color_attachments: Vec<RenderPassColorAttachment>,
    pub depth_stencil_attachment: Option<RenderPassDepthStencilAttachment>,
}

/// Render pass color attachment
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPassColorAttachment {
    pub view: Option<*mut c_void>, // ITextureView*
    pub resolve_target: Option<*mut c_void>, // ITextureView*
    pub load_op: LoadOp,
    pub store_op: StoreOp,
    pub clear_value: [f32; 4],
}

impl Default for RenderPassColorAttachment {
    fn default() -> Self {
        Self {
            view: None,
            resolve_target: None,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_value: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Render pass depth/stencil attachment
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPassDepthStencilAttachment {
    pub view: Option<*mut c_void>, // ITextureView*
    pub depth_load_op: LoadOp,
    pub depth_store_op: StoreOp,
    pub depth_clear_value: f32,
    pub depth_read_only: bool,
    pub stencil_load_op: LoadOp,
    pub stencil_store_op: StoreOp,
    pub stencil_clear_value: u8,
    pub stencil_read_only: bool,
}

impl Default for RenderPassDepthStencilAttachment {
    fn default() -> Self {
        Self {
            view: None,
            depth_load_op: LoadOp::Clear,
            depth_store_op: StoreOp::Store,
            depth_clear_value: 1.0,
            depth_read_only: false,
            stencil_load_op: LoadOp::Clear,
            stencil_store_op: StoreOp::Store,
            stencil_clear_value: 0,
            stencil_read_only: false,
        }
    }
}

/// Subresource data for texture initialization
#[derive(Debug, Clone, PartialEq)]
pub struct SubresourceData {
    /// Pointer to texel data for the subresource tensor.
    pub data: *const c_void,
    /// Stride in bytes between rows of the subresource tensor.
    pub row_pitch: Size,
    /// Stride in bytes between layers of the subresource tensor.
    pub slice_pitch: Size,
}

impl Default for SubresourceData {
    fn default() -> Self {
        Self {
            data: std::ptr::null(),
            row_pitch: 0,
            slice_pitch: 0,
        }
    }
}

/// Indirect draw arguments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IndirectDrawArguments {
    pub vertex_count_per_instance: u32,
    pub instance_count: u32,
    pub start_vertex_location: u32,
    pub start_instance_location: u32,
}

/// Indirect draw indexed arguments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IndirectDrawIndexedArguments {
    pub index_count_per_instance: u32,
    pub instance_count: u32,
    pub start_index_location: u32,
    pub base_vertex_location: i32,
    pub start_instance_location: u32,
}

/// Indirect dispatch arguments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IndirectDispatchArguments {
    pub thread_group_count_x: u32,
    pub thread_group_count_y: u32,
    pub thread_group_count_z: u32,
}

/// Submit descriptor for command queue
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SubmitDesc {
    pub command_buffers: Vec<*mut c_void>, // ICommandBuffer**
    pub wait_fences: Vec<*mut c_void>,     // IFence**
    pub wait_fence_values: Vec<u64>,
    pub signal_fences: Vec<*mut c_void>, // IFence**
    pub signal_fence_values: Vec<u64>,
    /// The CUDA stream to use for the submission. Ignored on non-CUDA backends.
    pub cuda_stream: Option<*mut c_void>,
}

/// Adapter information
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AdapterInfo {
    /// Descriptive name of the adapter.
    pub name: String,
    /// Unique identifier for the vendor (only available for D3D and Vulkan).
    pub vendor_id: u32,
    /// Unique identifier for the physical device among devices from the vendor.
    pub device_id: u32,
    /// Logically unique identifier of the adapter.
    pub luid: AdapterLUID,
}
