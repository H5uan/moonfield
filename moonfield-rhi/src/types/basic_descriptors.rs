//! Descriptor structures for RHI resources

use super::*;

/// Clear value for depth/stencil attachments
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthStencilClearValue {
    pub depth: f32,
    pub stencil: u32,
}

impl Default for DepthStencilClearValue {
    fn default() -> Self {
        Self {
            depth: 1.0,
            stencil: 0,
        }
    }
}

/// Clear value for color attachments
#[derive(Clone, Copy)]
pub union ColorClearValue {
    pub float_values: [f32; 4],
    pub uint_values: [u32; 4],
    pub int_values: [i32; 4],
}

impl Default for ColorClearValue {
    fn default() -> Self {
        Self {
            float_values: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

impl std::fmt::Debug for ColorClearValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            f.debug_struct("ColorClearValue")
                .field("float_values", &self.float_values)
                .finish()
        }
    }
}

impl PartialEq for ColorClearValue {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            self.float_values == other.float_values
        }
    }
}

/// Combined clear value for any attachment type
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClearValue {
    pub color: ColorClearValue,
    pub depth_stencil: DepthStencilClearValue,
}

/// Buffer descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct BufferDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    /// Total size in bytes.
    pub size: u64,
    /// Get the element stride. If > 0, this is a structured buffer.
    pub element_size: u32,
    /// Format used for typed views.
    pub format: Format,
    
    pub memory_type: MemoryType,
    pub usage: BufferUsage,
    pub default_state: ResourceState,
    
    /// The name of the buffer for debugging purposes.
    pub label: Option<String>,
}

impl Default for BufferDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::BufferDesc,
            next: None,
            size: 0,
            element_size: 0,
            format: Format::Undefined,
            memory_type: MemoryType::DeviceLocal,
            usage: BufferUsage::NONE,
            default_state: ResourceState::Undefined,
            label: None,
        }
    }
}

/// Texture descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct TextureDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub texture_type: TextureType,
    
    /// Size of the texture.
    pub size: Extent3D,
    /// Array length.
    pub array_length: u32,
    /// Number of mip levels.
    /// Set to ALL_MIPS to create all mip levels.
    pub mip_count: u32,
    
    /// The resources format.
    pub format: Format,
    
    /// Number of samples per pixel.
    pub sample_count: u32,
    /// The quality measure for the samples.
    pub sample_quality: u32,
    
    pub memory_type: MemoryType,
    pub usage: TextureUsage,
    pub default_state: ResourceState,
    
    pub optimal_clear_value: Option<ClearValue>,
    
    /// The name of the texture for debugging purposes.
    pub label: Option<String>,
}

impl TextureDesc {
    pub fn get_layer_count(&self) -> u32 {
        match self.texture_type {
            TextureType::TextureCube | TextureType::TextureCubeArray => self.array_length * 6,
            _ => self.array_length,
        }
    }
    
    pub fn get_subresource_count(&self) -> u32 {
        self.mip_count * self.get_layer_count()
    }
}

impl Default for TextureDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::TextureDesc,
            next: None,
            texture_type: TextureType::Texture2D,
            size: Extent3D::new(1, 1, 1),
            array_length: 1,
            mip_count: 1,
            format: Format::Undefined,
            sample_count: 1,
            sample_quality: 0,
            memory_type: MemoryType::DeviceLocal,
            usage: TextureUsage::NONE,
            default_state: ResourceState::Undefined,
            optimal_clear_value: None,
            label: None,
        }
    }
}

/// Texture view descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct TextureViewDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub format: Format,
    pub aspect: TextureAspect,
    pub subresource_range: SubresourceRange,
    pub label: Option<String>,
}

impl Default for TextureViewDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::TextureViewDesc,
            next: None,
            format: Format::Undefined,
            aspect: TextureAspect::All,
            subresource_range: ENTIRE_TEXTURE,
            label: None,
        }
    }
}

/// Sampler descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct SamplerDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub min_filter: TextureFilteringMode,
    pub mag_filter: TextureFilteringMode,
    pub mip_filter: TextureFilteringMode,
    pub reduction_op: TextureReductionOp,
    pub address_u: TextureAddressingMode,
    pub address_v: TextureAddressingMode,
    pub address_w: TextureAddressingMode,
    pub mip_lod_bias: f32,
    pub max_anisotropy: u32,
    pub comparison_func: ComparisonFunc,
    pub border_color: [f32; 4],
    pub min_lod: f32,
    pub max_lod: f32,
    
    pub label: Option<String>,
}

impl Default for SamplerDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::SamplerDesc,
            next: None,
            min_filter: TextureFilteringMode::Linear,
            mag_filter: TextureFilteringMode::Linear,
            mip_filter: TextureFilteringMode::Linear,
            reduction_op: TextureReductionOp::Average,
            address_u: TextureAddressingMode::Wrap,
            address_v: TextureAddressingMode::Wrap,
            address_w: TextureAddressingMode::Wrap,
            mip_lod_bias: 0.0,
            max_anisotropy: 1,
            comparison_func: ComparisonFunc::Never,
            border_color: [0.0, 0.0, 0.0, 0.0],
            min_lod: 0.0,
            max_lod: 1000.0,
            label: None,
        }
    }
}

/// Input element descriptor for vertex layouts
#[derive(Debug, Clone, PartialEq)]
pub struct InputElementDesc {
    /// The name of the corresponding parameter in shader code.
    pub semantic_name: String,
    /// The index of the corresponding parameter in shader code. Only needed if multiple parameters share a semantic name.
    pub semantic_index: u32,
    /// The format of the data being fetched for this element.
    pub format: Format,
    /// The offset in bytes of this element from the start of the corresponding chunk of vertex stream data.
    pub offset: u32,
    /// The index of the vertex stream to fetch this element's data from.
    pub buffer_slot_index: u32,
}

/// Vertex stream descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VertexStreamDesc {
    /// The stride in bytes for this vertex stream.
    pub stride: u32,
    /// Whether the stream contains per-vertex or per-instance data.
    pub slot_class: InputSlotClass,
    /// How many instances to draw per chunk of data.
    pub instance_data_step_rate: u32,
}

/// Input layout descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct InputLayoutDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub input_elements: Vec<InputElementDesc>,
    pub vertex_streams: Vec<VertexStreamDesc>,
}

impl Default for InputLayoutDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::InputLayoutDesc,
            next: None,
            input_elements: Vec::new(),
            vertex_streams: Vec::new(),
        }
    }
}

/// Shader program descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderProgramDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    /// The linking style of this program.
    pub linking_style: LinkingStyle,
    
    /// The global scope or a Slang composite component that represents the entire program.
    pub slang_global_scope: Option<*mut std::ffi::c_void>, // slang::IComponentType*
    
    /// An array of Slang entry points.
    pub slang_entry_points: Vec<*mut std::ffi::c_void>, // slang::IComponentType**
    
    pub label: Option<String>,
}

impl Default for ShaderProgramDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::ShaderProgramDesc,
            next: None,
            linking_style: LinkingStyle::SingleProgram,
            slang_global_scope: None,
            slang_entry_points: Vec::new(),
            label: None,
        }
    }
}

/// Fence descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct FenceDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub initial_value: u64,
    pub is_shared: bool,
    
    pub label: Option<String>,
}

impl Default for FenceDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::FenceDesc,
            next: None,
            initial_value: 0,
            is_shared: false,
            label: None,
        }
    }
}

/// Query pool descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct QueryPoolDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub query_type: QueryType,
    pub count: u32,
    
    pub label: Option<String>,
}

impl Default for QueryPoolDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::QueryPoolDesc,
            next: None,
            query_type: QueryType::Timestamp,
            count: 0,
            label: None,
        }
    }
}

/// Heap descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct HeapDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    /// Type of memory heap should reside in.
    pub memory_type: MemoryType,
    
    /// Usage flags for the heap.
    pub usage: HeapUsage,
    
    /// The label for the heap.
    pub label: Option<String>,
}

impl Default for HeapDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::HeapDesc,
            next: None,
            memory_type: MemoryType::DeviceLocal,
            usage: HeapUsage::NONE,
            label: None,
        }
    }
}

/// Acceleration structure descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub size: u64,
    
    pub label: Option<String>,
}

impl Default for AccelerationStructureDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::AccelerationStructureDesc,
            next: None,
            size: 0,
            label: None,
        }
    }
}

/// Shader offset for binding operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShaderOffset {
    pub uniform_offset: u32,
    pub binding_range_index: u32,
    pub binding_array_index: u32,
}

impl ShaderOffset {
    pub fn new(
        uniform_offset: u32, binding_range_index: u32, binding_array_index: u32,
    ) -> Self {
        Self { uniform_offset, binding_range_index, binding_array_index }
    }
}

impl PartialOrd for ShaderOffset {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ShaderOffset {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.binding_range_index
            .cmp(&other.binding_range_index)
            .then(self.binding_array_index.cmp(&other.binding_array_index))
            .then(self.uniform_offset.cmp(&other.uniform_offset))
    }
}

/// Binding for shader resources
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub binding_type: BindingType,
    pub resource: Option<*mut std::ffi::c_void>,
    pub resource2: Option<*mut std::ffi::c_void>,
    pub buffer_range: BufferRange,
}

impl Binding {
    pub fn buffer(buffer: *mut std::ffi::c_void, range: BufferRange) -> Self {
        Self {
            binding_type: BindingType::Buffer,
            resource: Some(buffer),
            resource2: None,
            buffer_range: range,
        }
    }

    pub fn buffer_with_counter(
        buffer: *mut std::ffi::c_void, counter: *mut std::ffi::c_void, range: BufferRange,
    ) -> Self {
        Self {
            binding_type: BindingType::BufferWithCounter,
            resource: Some(buffer),
            resource2: Some(counter),
            buffer_range: range,
        }
    }

    pub fn texture(texture_view: *mut std::ffi::c_void) -> Self {
        Self {
            binding_type: BindingType::Texture,
            resource: Some(texture_view),
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }

    pub fn sampler(sampler: *mut std::ffi::c_void) -> Self {
        Self {
            binding_type: BindingType::Sampler,
            resource: Some(sampler),
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }

    pub fn combined_texture_sampler(
        texture_view: *mut std::ffi::c_void, sampler: *mut std::ffi::c_void,
    ) -> Self {
        Self {
            binding_type: BindingType::CombinedTextureSampler,
            resource: Some(texture_view),
            resource2: Some(sampler),
            buffer_range: BufferRange::default(),
        }
    }

    pub fn acceleration_structure(acceleration_structure: *mut std::ffi::c_void) -> Self {
        Self {
            binding_type: BindingType::AccelerationStructure,
            resource: Some(acceleration_structure),
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            binding_type: BindingType::Undefined,
            resource: None,
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }
}

/// Bindless descriptor configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindlessDesc {
    /// Maximum number of bindless buffers.
    pub buffer_count: u32,
    /// Maximum number of bindless textures.
    pub texture_count: u32,
    /// Maximum number of bindless samplers.
    pub sampler_count: u32,
    /// Maximum number of bindless acceleration structures.
    pub acceleration_structure_count: u32,
}

impl Default for BindlessDesc {
    fn default() -> Self {
        Self {
            buffer_count: 1024,
            texture_count: 1024,
            sampler_count: 128,
            acceleration_structure_count: 128,
        }
    }
}

/// Slang descriptor for shader compilation
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlangDesc {
    /// (optional) A slang global session object, if null a new one will be created.
    pub slang_global_session: Option<*mut std::ffi::c_void>, // slang::IGlobalSession*

    pub default_matrix_layout_mode: i32, // SlangMatrixLayoutMode

    pub search_paths: Vec<String>,
    pub preprocessor_macros: Vec<String>, // Simplified
    pub compiler_option_entries: Vec<String>, // Simplified

    /// (optional) Target shader profile. If null this will be set to platform dependent default.
    pub target_profile: Option<String>,

    pub floating_point_mode: i32, // SlangFloatingPointMode
    pub optimization_level: i32,  // SlangOptimizationLevel
    pub target_flags: u32,        // SlangTargetFlags
    pub line_directive_mode: i32, // SlangLineDirectiveMode
}

/// Acceleration structure AABB
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AccelerationStructureAABB {
    pub min_x: f32,
    pub min_y: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub max_z: f32,
}

/// Acceleration structure build input for instances
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccelerationStructureBuildInputInstances {
    pub instance_buffer: BufferOffsetPair,
    pub instance_stride: u32,
    pub instance_count: u32,
}

/// Acceleration structure build input for triangles
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureBuildInputTriangles {
    /// List of vertex buffers, one for each motion step.
    pub vertex_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub vertex_buffer_count: u32,
    pub vertex_format: Format,
    pub vertex_count: u32,
    pub vertex_stride: u32,

    pub index_buffer: BufferOffsetPair,
    pub index_format: IndexFormat,
    pub index_count: u32,

    /// Optional buffer containing 3x4 transform matrix applied to each vertex.
    pub pre_transform_buffer: BufferOffsetPair,

    pub flags: AccelerationStructureGeometryFlags,
}

impl Default for AccelerationStructureBuildInputTriangles {
    fn default() -> Self {
        Self {
            vertex_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            vertex_buffer_count: 0,
            vertex_format: Format::Undefined,
            vertex_count: 0,
            vertex_stride: 0,
            index_buffer: BufferOffsetPair::default(),
            index_format: IndexFormat::Uint32,
            index_count: 0,
            pre_transform_buffer: BufferOffsetPair::default(),
            flags: AccelerationStructureGeometryFlags::NONE,
        }
    }
}

/// Acceleration structure build input for procedural primitives
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureBuildInputProceduralPrimitives {
    /// List of AABB buffers, one for each motion step.
    pub aabb_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub aabb_buffer_count: u32,
    pub aabb_stride: u32,
    pub primitive_count: u32,

    pub flags: AccelerationStructureGeometryFlags,
}

impl Default for AccelerationStructureBuildInputProceduralPrimitives {
    fn default() -> Self {
        Self {
            aabb_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            aabb_buffer_count: 0,
            aabb_stride: 0,
            primitive_count: 0,
            flags: AccelerationStructureGeometryFlags::NONE,
        }
    }
}

/// Acceleration structure build input for spheres
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureBuildInputSpheres {
    pub vertex_buffer_count: u32,
    pub vertex_count: u32,

    pub vertex_position_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub vertex_position_format: Format,
    pub vertex_position_stride: u32,

    pub vertex_radius_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub vertex_radius_format: Format,
    pub vertex_radius_stride: u32,

    pub index_buffer: BufferOffsetPair,
    pub index_format: IndexFormat,
    pub index_count: u32,

    pub flags: AccelerationStructureGeometryFlags,
}

impl Default for AccelerationStructureBuildInputSpheres {
    fn default() -> Self {
        Self {
            vertex_buffer_count: 0,
            vertex_count: 0,
            vertex_position_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            vertex_position_format: Format::Undefined,
            vertex_position_stride: 0,
            vertex_radius_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            vertex_radius_format: Format::Undefined,
            vertex_radius_stride: 0,
            index_buffer: BufferOffsetPair::default(),
            index_format: IndexFormat::Uint32,
            index_count: 0,
            flags: AccelerationStructureGeometryFlags::NONE,
        }
    }
}

/// Acceleration structure build input for linear swept spheres
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureBuildInputLinearSweptSpheres {
    pub vertex_buffer_count: u32,
    pub vertex_count: u32,
    pub primitive_count: u32,

    pub vertex_position_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub vertex_position_format: Format,
    pub vertex_position_stride: u32,

    pub vertex_radius_buffers: [BufferOffsetPair; MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
    pub vertex_radius_format: Format,
    pub vertex_radius_stride: u32,

    pub index_buffer: BufferOffsetPair,
    pub index_format: IndexFormat,
    pub index_count: u32,

    pub indexing_mode: LinearSweptSpheresIndexingMode,
    pub end_caps_mode: LinearSweptSpheresEndCapsMode,

    pub flags: AccelerationStructureGeometryFlags,
}

impl Default for AccelerationStructureBuildInputLinearSweptSpheres {
    fn default() -> Self {
        Self {
            vertex_buffer_count: 0,
            vertex_count: 0,
            primitive_count: 0,
            vertex_position_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            vertex_position_format: Format::Undefined,
            vertex_position_stride: 0,
            vertex_radius_buffers: [BufferOffsetPair::default(); MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT as usize],
            vertex_radius_format: Format::Undefined,
            vertex_radius_stride: 0,
            index_buffer: BufferOffsetPair::default(),
            index_format: IndexFormat::Uint32,
            index_count: 0,
            indexing_mode: LinearSweptSpheresIndexingMode::List,
            end_caps_mode: LinearSweptSpheresEndCapsMode::None,
            flags: AccelerationStructureGeometryFlags::NONE,
        }
    }
}

/// Acceleration structure build input union
pub union AccelerationStructureBuildInputData {
    pub instances: AccelerationStructureBuildInputInstances,
    pub triangles: std::mem::ManuallyDrop<AccelerationStructureBuildInputTriangles>,
    pub procedural_primitives: std::mem::ManuallyDrop<AccelerationStructureBuildInputProceduralPrimitives>,
    pub spheres: std::mem::ManuallyDrop<AccelerationStructureBuildInputSpheres>,
    pub linear_swept_spheres: std::mem::ManuallyDrop<AccelerationStructureBuildInputLinearSweptSpheres>,
}

impl std::fmt::Debug for AccelerationStructureBuildInputData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccelerationStructureBuildInputData")
            .field("data", &"<union>")
            .finish()
    }
}

impl PartialEq for AccelerationStructureBuildInputData {
    fn eq(&self, _other: &Self) -> bool {
        // Union comparison is complex, so we'll just return false for now
        false
    }
}

/// Acceleration structure build input
pub struct AccelerationStructureBuildInput {
    pub input_type: AccelerationStructureBuildInputType,
    pub data: AccelerationStructureBuildInputData,
}

impl Default for AccelerationStructureBuildInput {
    fn default() -> Self {
        Self {
            input_type: AccelerationStructureBuildInputType::Instances,
            data: AccelerationStructureBuildInputData {
                instances: AccelerationStructureBuildInputInstances::default(),
            },
        }
    }
}

/// Motion options for acceleration structure building
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AccelerationStructureBuildInputMotionOptions {
    pub key_count: u32,
    pub time_start: f32,
    pub time_end: f32,
}

/// Acceleration structure build descriptor
pub struct AccelerationStructureBuildDesc {
    /// List of build inputs. All inputs must be of the same type.
    pub inputs: Vec<AccelerationStructureBuildInput>,
    pub motion_options: AccelerationStructureBuildInputMotionOptions,
    pub mode: AccelerationStructureBuildMode,
    pub flags: AccelerationStructureBuildFlags,
}

impl Default for AccelerationStructureBuildDesc {
    fn default() -> Self {
        Self {
            inputs: Vec::new(),
            motion_options: AccelerationStructureBuildInputMotionOptions::default(),
            mode: AccelerationStructureBuildMode::Build,
            flags: AccelerationStructureBuildFlags::NONE,
        }
    }
}

/// Acceleration structure sizes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AccelerationStructureSizes {
    pub acceleration_structure_size: u64,
    pub scratch_size: u64,
    pub update_scratch_size: u64,
}

/// Acceleration structure query descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct AccelerationStructureQueryDesc {
    pub query_type: QueryType,
    pub query_pool: Option<*mut std::ffi::c_void>, // IQueryPool*
    pub first_query_index: u32,
}

impl Default for AccelerationStructureQueryDesc {
    fn default() -> Self {
        Self {
            query_type: QueryType::Timestamp,
            query_pool: None,
            first_query_index: 0,
        }
    }
}
