//! Core trait definitions for RHI interfaces

use super::*;
use std::ffi::c_void;

/// Base trait for all RHI objects (equivalent to ISlangUnknown)
pub trait RHIObject {
    /// Get a reference count (if applicable)
    fn add_ref(&self) -> u32 { 1 }
    
    /// Release a reference (if applicable)
    fn release(&self) -> u32 { 0 }
    
    /// Query for a specific interface
    fn query_interface(&self, _iid: &str) -> Option<*mut c_void> { None }
}

/// Base resource trait
pub trait Resource: RHIObject {
    /// Get the native handle for this resource
    fn get_native_handle(&self) -> Result<NativeHandle, crate::RHIError>;
}

/// Buffer resource trait
pub trait Buffer: Resource {
    /// Get the buffer descriptor
    fn get_desc(&self) -> &BufferDesc;
    
    /// Get shared handle for cross-process sharing
    fn get_shared_handle(&self) -> Result<NativeHandle, crate::RHIError>;
    
    /// Get device address for GPU access
    fn get_device_address(&self) -> DeviceAddress;
    
    /// Get descriptor handle for bindless access
    fn get_descriptor_handle(
        &self,
        access: DescriptorHandleAccess,
        format: Format,
        range: BufferRange,
    ) -> Result<DescriptorHandle, crate::RHIError>;
}

/// Texture resource trait
pub trait Texture: Resource {
    /// Get the texture descriptor
    fn get_desc(&self) -> &TextureDesc;
    
    /// Get shared handle for cross-process sharing
    fn get_shared_handle(&self) -> Result<NativeHandle, crate::RHIError>;
    
    /// Create a texture view
    fn create_view(&self, desc: &TextureViewDesc) -> Result<Box<dyn TextureView>, crate::RHIError>;
    
    /// Get the default view
    fn get_default_view(&self) -> Result<Box<dyn TextureView>, crate::RHIError>;
    
    /// Get layout of a subresource with given packing
    fn get_subresource_layout(
        &self,
        mip: u32,
        row_alignment: Size,
    ) -> Result<SubresourceLayout, crate::RHIError>;
}

/// Texture view trait
pub trait TextureView: Resource {
    /// Get the texture view descriptor
    fn get_desc(&self) -> &TextureViewDesc;
    
    /// Get the parent texture
    fn get_texture(&self) -> &dyn Texture;
    
    /// Get descriptor handle for bindless access
    fn get_descriptor_handle(
        &self,
        access: DescriptorHandleAccess,
    ) -> Result<DescriptorHandle, crate::RHIError>;
}

/// Sampler trait
pub trait Sampler: Resource {
    /// Get the sampler descriptor
    fn get_desc(&self) -> &SamplerDesc;
    
    /// Get descriptor handle for bindless access
    fn get_descriptor_handle(&self) -> Result<DescriptorHandle, crate::RHIError>;
}

/// Acceleration structure trait
pub trait AccelerationStructure: Resource {
    /// Get the acceleration structure handle
    fn get_handle(&self) -> AccelerationStructureHandle;
    
    /// Get device address
    fn get_device_address(&self) -> DeviceAddress;
    
    /// Get descriptor handle for bindless access
    fn get_descriptor_handle(&self) -> Result<DescriptorHandle, crate::RHIError>;
}

/// Fence trait for synchronization
pub trait Fence: RHIObject {
    /// Get the currently signaled value on the device
    fn get_current_value(&self) -> Result<u64, crate::RHIError>;
    
    /// Signal the fence from the host with the specified value
    fn set_current_value(&self, value: u64) -> Result<(), crate::RHIError>;
    
    /// Get native handle
    fn get_native_handle(&self) -> Result<NativeHandle, crate::RHIError>;
    
    /// Get shared handle
    fn get_shared_handle(&self) -> Result<NativeHandle, crate::RHIError>;
}

/// Shader program trait
pub trait ShaderProgram: RHIObject {
    /// Get the shader program descriptor
    fn get_desc(&self) -> &ShaderProgramDesc;
    
    /// Get compilation report
    fn get_compilation_report(&self) -> Result<Vec<u8>, crate::RHIError>;
    
    /// Find type by name
    fn find_type_by_name(&self, name: &str) -> Option<*mut c_void>; // slang::TypeReflection*
}

/// Input layout trait
pub trait InputLayout: RHIObject {
    /// Get the input layout descriptor
    fn get_desc(&self) -> &InputLayoutDesc;
}

/// Base pipeline trait
pub trait Pipeline: RHIObject {
    /// Get the shader program
    fn get_program(&self) -> &dyn ShaderProgram;
    
    /// Get native handle
    fn get_native_handle(&self) -> Result<NativeHandle, crate::RHIError>;
}

/// Render pipeline trait
pub trait RenderPipeline: Pipeline {
    /// Get the render pipeline descriptor
    fn get_desc(&self) -> &RenderPipelineDesc;
}

/// Compute pipeline trait
pub trait ComputePipeline: Pipeline {
    /// Get the compute pipeline descriptor
    fn get_desc(&self) -> &ComputePipelineDesc;
}

/// Ray tracing pipeline trait
pub trait RayTracingPipeline: Pipeline {
    /// Get the ray tracing pipeline descriptor
    fn get_desc(&self) -> &RayTracingPipelineDesc;
}

/// Shader table trait
pub trait ShaderTable: RHIObject {
    /// Get the shader table descriptor
    fn get_desc(&self) -> &ShaderTableDesc;
}

/// Query pool trait
pub trait QueryPool: RHIObject {
    /// Get the query pool descriptor
    fn get_desc(&self) -> &QueryPoolDesc;
    
    /// Get query results
    fn get_result(&self, query_index: u32, count: u32, data: &mut [u64]) -> Result<(), crate::RHIError>;
    
    /// Reset the query pool
    fn reset(&self) -> Result<(), crate::RHIError>;
}

/// Command buffer trait
pub trait CommandBuffer: RHIObject {
    /// Get native handle
    fn get_native_handle(&self) -> Result<NativeHandle, crate::RHIError>;
}

/// Surface trait for presentation
pub trait Surface: RHIObject {
    /// Get surface information
    fn get_info(&self) -> &SurfaceInfo;
    
    /// Get current surface configuration
    fn get_config(&self) -> Option<&SurfaceConfig>;
    
    /// Configure the surface
    fn configure(&mut self, config: &SurfaceConfig) -> Result<(), crate::RHIError>;
    
    /// Unconfigure the surface
    fn unconfigure(&mut self) -> Result<(), crate::RHIError>;
    
    /// Acquire next image for rendering
    fn acquire_next_image(&mut self) -> Result<Box<dyn Texture>, crate::RHIError>;
    
    /// Present the current image
    fn present(&mut self) -> Result<(), crate::RHIError>;
}

/// Heap trait for memory management
pub trait Heap: RHIObject {
    /// Allocate memory from the heap
    fn allocate(&mut self, desc: &HeapAllocDesc) -> Result<HeapAlloc, crate::RHIError>;
    
    /// Free allocated memory
    fn free(&mut self, allocation: HeapAlloc) -> Result<(), crate::RHIError>;
    
    /// Get heap usage report
    fn report(&self) -> Result<HeapReport, crate::RHIError>;
    
    /// Flush pending operations
    fn flush(&mut self) -> Result<(), crate::RHIError>;
    
    /// Remove empty pages
    fn remove_empty_pages(&mut self) -> Result<(), crate::RHIError>;
}

// Additional types needed for the traits

/// Subresource layout information
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubresourceLayout {
    /// Dimensions of the subresource (in texels).
    pub size: Extent3D,
    /// Stride in bytes between columns (i.e. blocks of pixels) of the subresource tensor.
    pub col_pitch: Size,
    /// Stride in bytes between rows of the subresource tensor.
    pub row_pitch: Size,
    /// Stride in bytes between layers of the subresource tensor.
    pub slice_pitch: Size,
    /// Overall size required to fit the subresource data.
    pub size_in_bytes: Size,
    /// Block width in texels (1 for uncompressed formats).
    pub block_width: Size,
    /// Block height in texels (1 for uncompressed formats).
    pub block_height: Size,
    /// Number of rows.
    pub row_count: Size,
}

/// Acceleration structure handle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AccelerationStructureHandle {
    pub value: u64,
}


/// Surface information
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceInfo {
    /// The preferred format for the surface.
    pub preferred_format: Format,
    /// The supported texture usage for the surface.
    pub supported_usage: TextureUsage,
    /// The list of supported formats for the surface.
    pub formats: Vec<Format>,
}

/// Surface configuration
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceConfig {
    /// Surface format. If left undefined, the preferred format is used.
    pub format: Format,
    /// Usage of the surface. If left undefined, the supported usage is used.
    pub usage: TextureUsage,
    /// Width of the surface in pixels.
    pub width: u32,
    /// Height of the surface in pixels.
    pub height: u32,
    /// Desired number of images in the swap chain.
    pub desired_image_count: u32,
    /// Enable/disable vertical synchronization.
    pub vsync: bool,
}

impl Default for SurfaceConfig {
    fn default() -> Self {
        Self {
            format: Format::Undefined,
            usage: TextureUsage::NONE,
            width: 0,
            height: 0,
            desired_image_count: 3,
            vsync: true,
        }
    }
}

/// Heap allocation descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HeapAllocDesc {
    pub size: Size,
    pub alignment: Size,
}


/// Heap allocation result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapAlloc {
    pub offset: Offset,
    pub size: Size,
    pub page_id: *mut c_void,
    pub node_index: u32,
    pub address: usize,
}

impl HeapAlloc {
    pub fn get_device_address(&self) -> DeviceAddress {
        self.address as DeviceAddress
    }
    
    pub fn get_host_ptr(&self) -> *mut c_void {
        self.address as *mut c_void
    }
    
    pub fn is_valid(&self) -> bool {
        self.address != 0
    }
}

impl Default for HeapAlloc {
    fn default() -> Self {
        Self {
            offset: 0,
            size: 0,
            page_id: std::ptr::null_mut(),
            node_index: 0xffffffff,
            address: 0,
        }
    }
}

/// Heap usage report
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeapReport {
    pub label: String,
    pub num_pages: u32,
    pub total_allocated: u64,
    pub total_mem_usage: u64,
    pub num_allocations: u64,
}

/// Pipeline type for compilation reports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineType {
    Render,
    Compute,
    RayTracing,
}

/// Entry point compilation report
#[derive(Debug, Clone, PartialEq)]
pub struct EntryPointReport {
    pub name: String,
    pub start_time: u64, // TimePoint
    pub end_time: u64,   // TimePoint
    pub create_time: f64,
    pub compile_time: f64,
    pub compile_slang_time: f64,
    pub compile_downstream_time: f64,
    pub is_cached: bool,
    pub cache_size: Size,
}

/// Pipeline compilation report
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineReport {
    pub pipeline_type: PipelineType,
    pub start_time: u64, // TimePoint
    pub end_time: u64,   // TimePoint
    pub create_time: f64,
    pub is_cached: bool,
    pub cache_size: Size,
}

/// Compilation report for shader programs
#[derive(Debug, Clone, PartialEq)]
pub struct CompilationReport {
    /// Shader program label.
    pub label: String,
    /// Shader program is currently alive.
    pub alive: bool,
    /// Total time spent creating the shader program (seconds).
    pub create_time: f64,
    /// Total time spent compiling entry points (seconds).
    pub compile_time: f64,
    /// Total time spent in the slang compiler backend (seconds).
    pub compile_slang_time: f64,
    /// Total time spent in the downstream compiler (seconds).
    pub compile_downstream_time: f64,
    /// Total time spent creating pipelines (seconds).
    pub create_pipeline_time: f64,

    /// Entry points compilation reports.
    pub entry_point_reports: Vec<EntryPointReport>,
    /// Pipelines creation reports.
    pub pipeline_reports: Vec<PipelineReport>,
}

/// Compilation report list
#[derive(Debug, Clone, PartialEq)]
pub struct CompilationReportList {
    pub reports: Vec<CompilationReport>,
}


pub trait RHI{
    fn get_format_info(format: Format) -> &'static FormatInfo;
    fn get_device_type_name(device_type: DeviceType) -> &'static str;
    fn is_device_type_supported(device_type: DeviceType) -> bool;
    fn get_feature_name(feature: Feature) -> &'static str;
    fn get_capability_name() -> &'static str;
    fn get_adapters();
    fn enable_debug_layers();
    fn create_device();
    fn create_blob();
    /// needed for d3d
    fn report_live_objects();
    fn set_task_pool();
}

pub trait Device{
    fn get_info() -> DeviceInfo;
    fn get_device_type() -> DeviceType;
    fn get_native_device_handles(handles: DeviceNativeHandles);
    fn get_features();
    fn has_features();
    fn get_capabilities();
    fn has_capabilities();

    fn get_format_support(format: Format) -> FormatSupport;
    fn get_slang_session();

    fn create_texture();
    fn create_texture_from_native_handle();
    fn create_texture_from_shared_handle();

    fn create_buffer();
    fn create_buffer_from_native_handle();
    fn create_buffer_from_shared_handle();

    fn map_buffer();
    fn unmap_buffer();

    fn create_sampler();
    fn create_texture_view();

    fn create_surface();
    fn create_input_layout();
    
    fn get_queue();
    fn create_shader_object();

    fn create_shader_object_from_type_layout();
    fn create_root_shader_object();

    fn create_shader_table();
    fn create_shader_program();

    fn create_render_pipeline();

    fn create_compute_pipeline();

    fn create_ray_tracing_pipeline();

    fn get_compilation_report_list();

    fn read_texture();

    fn read_buffer();

    fn create_query_pool();

    fn create_acceleration_structure();

    fn create_fence();

    fn wait_for_fences();

    fn create_heap();

    fn get_texture_row_alignment();

    fn get_cooperative_vector_properties(){
        
    }

    fn convert_cooperative_vector_matrix(){

    }

    fn report_heaps();

}