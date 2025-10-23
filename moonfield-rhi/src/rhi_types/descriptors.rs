//! Descriptor structures for RHI resources

use super::*;
use crate::dynamic::{
    DynAccelerationStructure, DynBuffer, DynInputLayout, DynResource,
    DynSampler, DynTextureView,
};

/// Buffer descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct BufferDesc<'a> {
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
    pub label: Label<'a>,
}

impl Default for BufferDesc<'_> {
    fn default() -> Self {
        Self {
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
pub struct TextureDesc<'a> {
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
    pub label: Label<'a>,
}

impl TextureDesc<'_> {
    pub fn get_layer_count(&self) -> u32 {
        match self.texture_type {
            TextureType::TextureCube | TextureType::TextureCubeArray => {
                self.array_length * 6
            }
            _ => self.array_length,
        }
    }

    pub fn get_subresource_count(&self) -> u32 {
        self.mip_count * self.get_layer_count()
    }
}

impl Default for TextureDesc<'_> {
    fn default() -> Self {
        Self {
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
pub struct TextureViewDesc<'a> {
    pub format: Format,
    pub aspect: TextureAspect,
    pub subresource_range: SubresourceRange,
    pub label: Label<'a>,
}

impl Default for TextureViewDesc<'_> {
    fn default() -> Self {
        Self {
            format: Format::Undefined,
            aspect: TextureAspect::All,
            subresource_range: ENTIRE_TEXTURE,
            label: None,
        }
    }
}

/// Sampler descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct SamplerDesc<'a> {
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

    pub label: Label<'a>,
}

impl Default for SamplerDesc<'_> {
    fn default() -> Self {
        Self {
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputLayoutDesc {
    pub input_elements: Vec<InputElementDesc>,
    pub vertex_streams: Vec<VertexStreamDesc>,
}

/// Shader program descriptor
pub struct ShaderProgramDesc<'a> {
    /// The linking style of this program.
    pub linking_style: LinkingStyle,

    /// The global scope or a Slang composite component that represents the entire program.
    pub slang_global_scope: Option<shader_slang::ComponentType>,
    /// An array of Slang entry points.
    pub slang_entry_points: Vec<shader_slang::ComponentType>,

    pub label: Label<'a>,
}

impl Debug for ShaderProgramDesc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaderProgramDesc")
            .field("linking_style", &self.linking_style)
            .field("label", &self.label)
            .finish()
    }
}

impl Default for ShaderProgramDesc<'_> {
    fn default() -> Self {
        Self {
            linking_style: LinkingStyle::SingleProgram,
            slang_global_scope: None,
            slang_entry_points: Vec::new(),
            label: None,
        }
    }
}

/// Fence descriptor
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FenceDesc<'a> {
    pub initial_value: u64,
    pub is_shared: bool,

    pub label: Label<'a>,
}

/// Query pool descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct QueryPoolDesc<'a> {
    pub query_type: QueryType,
    pub count: u32,

    pub label: Label<'a>,
}

impl Default for QueryPoolDesc<'_> {
    fn default() -> Self {
        Self { query_type: QueryType::Timestamp, count: 0, label: None }
    }
}

/// Heap descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct HeapDesc<'a> {
    /// Type of memory heap should reside in.
    pub memory_type: MemoryType,

    /// Usage flags for the heap.
    pub usage: HeapUsage,

    /// The label for the heap.
    pub label: Label<'a>,
}

impl Default for HeapDesc<'_> {
    fn default() -> Self {
        Self {
            memory_type: MemoryType::DeviceLocal,
            usage: HeapUsage::NONE,
            label: None,
        }
    }
}

/// Acceleration structure descriptor
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccelerationStructureDesc<'a> {
    pub size: u64,

    pub label: Label<'a>,
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
#[derive(Debug)]
pub struct Binding {
    pub binding_type: BindingType,
    pub resource: Option<Box<dyn DynResource>>, // resource
    pub resource2: Option<Box<dyn DynResource>>, // resource
    pub buffer_range: BufferRange,
}

impl Binding {
    pub fn buffer(buffer: Box<dyn DynBuffer>, range: BufferRange) -> Self {
        Self {
            binding_type: BindingType::Buffer,
            resource: Some(buffer),
            resource2: None,
            buffer_range: range,
        }
    }

    pub fn buffer_with_counter(
        buffer: Box<dyn DynBuffer>, counter: Box<dyn DynBuffer>,
        range: BufferRange,
    ) -> Self {
        Self {
            binding_type: BindingType::BufferWithCounter,
            resource: Some(buffer),
            resource2: Some(counter),
            buffer_range: range,
        }
    }

    pub fn texture(texture_view: Box<dyn DynTextureView>) -> Self {
        Self {
            binding_type: BindingType::Texture,
            resource: Some(texture_view),
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }

    pub fn sampler(sampler: Box<dyn DynSampler>) -> Self {
        Self {
            binding_type: BindingType::Sampler,
            resource: Some(sampler),
            resource2: None,
            buffer_range: BufferRange::default(),
        }
    }

    pub fn combined_texture_sampler(
        texture_view: Box<dyn DynTextureView>, sampler: Box<dyn DynSampler>,
    ) -> Self {
        Self {
            binding_type: BindingType::CombinedTextureSampler,
            resource: Some(texture_view),
            resource2: Some(sampler),
            buffer_range: BufferRange::default(),
        }
    }

    pub fn acceleration_structure(
        acceleration_structure: Box<dyn DynAccelerationStructure>,
    ) -> Self {
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
#[derive(Default)]
pub struct SlangDesc {
    /// (optional) A slang global session object, if null a new one will be created.
    pub slang_global_session: Option<shader_slang::GlobalSession>,

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

impl Debug for SlangDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlangDesc")
            .field(
                "default_matrix_layout_mode",
                &self.default_matrix_layout_mode,
            )
            .field("search_paths", &self.search_paths)
            .field("preprocessor_macros", &self.preprocessor_macros)
            .field("compiler_option_entries", &self.compiler_option_entries)
            .field("target_profile", &self.target_profile)
            .field("floating_point_mode", &self.floating_point_mode)
            .field("optimization_level", &self.optimization_level)
            .field("target_flags", &self.target_flags)
            .field("line_directive_mode", &self.line_directive_mode)
            .finish()
    }
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

/// Depth/stencil operation descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthStencilOpDesc {
    pub stencil_fail_op: StencilOp,
    pub stencil_depth_fail_op: StencilOp,
    pub stencil_pass_op: StencilOp,
    pub stencil_func: ComparisonFunc,
}

impl Default for DepthStencilOpDesc {
    fn default() -> Self {
        Self {
            stencil_fail_op: StencilOp::Keep,
            stencil_depth_fail_op: StencilOp::Keep,
            stencil_pass_op: StencilOp::Keep,
            stencil_func: ComparisonFunc::Always,
        }
    }
}

/// Depth/stencil descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct DepthStencilDesc {
    pub format: Format,

    pub depth_test_enable: bool,
    pub depth_write_enable: bool,
    pub depth_func: ComparisonFunc,

    pub stencil_enable: bool,
    pub stencil_read_mask: u32,
    pub stencil_write_mask: u32,
    pub front_face: DepthStencilOpDesc,
    pub back_face: DepthStencilOpDesc,

    pub stencil_ref: u32, // TODO: this should be removed
}

impl Default for DepthStencilDesc {
    fn default() -> Self {
        Self {
            format: Format::Undefined,
            depth_test_enable: false,
            depth_write_enable: true,
            depth_func: ComparisonFunc::Less,
            stencil_enable: false,
            stencil_read_mask: 0xFFFFFFFF,
            stencil_write_mask: 0xFFFFFFFF,
            front_face: DepthStencilOpDesc::default(),
            back_face: DepthStencilOpDesc::default(),
            stencil_ref: 0,
        }
    }
}

/// Rasterizer descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct RasterizerDesc {
    pub fill_mode: FillMode,
    pub cull_mode: CullMode,
    pub front_face: FrontFaceMode,
    pub depth_bias: i32,
    pub depth_bias_clamp: f32,
    pub slope_scaled_depth_bias: f32,
    pub depth_clip_enable: bool,
    pub scissor_enable: bool,
    pub multisample_enable: bool,
    pub antialiased_line_enable: bool,
    pub enable_conservative_rasterization: bool,
    pub forced_sample_count: u32,
}

impl Default for RasterizerDesc {
    fn default() -> Self {
        Self {
            fill_mode: FillMode::Solid,
            cull_mode: CullMode::None,
            front_face: FrontFaceMode::CounterClockwise,
            depth_bias: 0,
            depth_bias_clamp: 0.0,
            slope_scaled_depth_bias: 0.0,
            depth_clip_enable: true,
            scissor_enable: false,
            multisample_enable: false,
            antialiased_line_enable: false,
            enable_conservative_rasterization: false,
            forced_sample_count: 0,
        }
    }
}

/// Aspect blend descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AspectBlendDesc {
    pub src_factor: BlendFactor,
    pub dst_factor: BlendFactor,
    pub op: BlendOp,
}

impl Default for AspectBlendDesc {
    fn default() -> Self {
        Self {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::Zero,
            op: BlendOp::Add,
        }
    }
}

/// Color target descriptor
#[derive(Debug, Clone, PartialEq)]
pub struct ColorTargetDesc {
    pub format: Format,
    pub color: AspectBlendDesc,
    pub alpha: AspectBlendDesc,
    pub enable_blend: bool,
    pub logic_op: LogicOp,
    pub write_mask: RenderTargetWriteMask,
}

impl Default for ColorTargetDesc {
    fn default() -> Self {
        Self {
            format: Format::Undefined,
            color: AspectBlendDesc::default(),
            alpha: AspectBlendDesc::default(),
            enable_blend: false,
            logic_op: LogicOp::NoOp,
            write_mask: RenderTargetWriteMask::ALL,
        }
    }
}

/// Multisample descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MultisampleDesc {
    pub sample_count: u32,
    pub sample_mask: u32,
    pub alpha_to_coverage_enable: bool,
    pub alpha_to_one_enable: bool,
}

impl Default for MultisampleDesc {
    fn default() -> Self {
        Self {
            sample_count: 1,
            sample_mask: 0xFFFFFFFF,
            alpha_to_coverage_enable: false,
            alpha_to_one_enable: false,
        }
    }
}

/// Render pipeline descriptor
#[derive(Debug)]
pub struct RenderPipelineDesc<'a> {
    pub program: Option<Box<dyn DynShaderProgram>>, // IShaderProgram*
    pub input_layout: Option<Box<dyn DynInputLayout>>, // IInputLayout*
    pub primitive_topology: PrimitiveTopology,
    pub targets: Vec<ColorTargetDesc>,
    pub depth_stencil: DepthStencilDesc,
    pub rasterizer: RasterizerDesc,
    pub multisample: MultisampleDesc,

    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,

    pub label: Label<'a>,
}

impl Default for RenderPipelineDesc<'_> {
    fn default() -> Self {
        Self {
            program: None,
            input_layout: None,
            primitive_topology: PrimitiveTopology::TriangleList,
            targets: Vec::new(),
            depth_stencil: DepthStencilDesc::default(),
            rasterizer: RasterizerDesc::default(),
            multisample: MultisampleDesc::default(),
            defer_target_compilation: false,
            label: None,
        }
    }
}

/// Compute pipeline descriptor
#[derive(Debug, Default)]
pub struct ComputePipelineDesc<'a> {
    pub program: Option<Box<dyn DynShaderProgram>>,

    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,

    pub label: Label<'a>,
}

/// Hit group descriptor for ray tracing
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HitGroupDesc {
    pub hit_group_name: Option<String>,
    pub closest_hit_entry_point: Option<String>,
    pub any_hit_entry_point: Option<String>,
    pub intersection_entry_point: Option<String>,
}

/// Ray tracing pipeline descriptor
#[derive(Debug)]
pub struct RayTracingPipelineDesc<'a> {
    pub program: Option<Box<dyn DynShaderProgram>>, // IShaderProgram*
    pub hit_groups: Vec<HitGroupDesc>,
    pub max_recursion: u32,
    pub max_ray_payload_size: u32,
    pub max_attribute_size_in_bytes: u32,
    pub flags: RayTracingPipelineFlags,

    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,

    pub label: Label<'a>,
}

impl Default for RayTracingPipelineDesc<'_> {
    fn default() -> Self {
        Self {
            program: None,
            hit_groups: Vec::new(),
            max_recursion: 0,
            max_ray_payload_size: 0,
            max_attribute_size_in_bytes: 8,
            flags: RayTracingPipelineFlags::NONE,
            defer_target_compilation: false,
            label: None,
        }
    }
}

/// Shader record overwrite for shader tables
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShaderRecordOverwrite {
    /// Offset within the shader record.
    pub offset: u8,
    /// Number of bytes to overwrite.
    pub size: u8,
    /// Content to overwrite.
    pub data: [u8; 8],
}

/// Shader table descriptor
#[derive(Debug, Default)]
pub struct ShaderTableDesc {
    pub ray_gen_shader_entry_point_names: Vec<String>,
    pub ray_gen_shader_record_overwrites: Vec<ShaderRecordOverwrite>,

    pub miss_shader_entry_point_names: Vec<String>,
    pub miss_shader_record_overwrites: Vec<ShaderRecordOverwrite>,

    pub hit_group_names: Vec<String>,
    pub hit_group_record_overwrites: Vec<ShaderRecordOverwrite>,

    pub callable_shader_entry_point_names: Vec<String>,
    pub callable_shader_record_overwrites: Vec<ShaderRecordOverwrite>,

    pub program: Option<Box<dyn DynShaderProgram>>, // IShaderProgram*
}


bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct InstanceFlags: u32 {
        /// Generate debug information in shaders and objects.
        const DEBUG = 1 << 0;
        /// Enable validation layer in the backend API.
        const VALIDATION = 1 << 1;
    }
}

impl Default for InstanceFlags {
    fn default() -> Self {
        InstanceFlags::DEBUG
    }
}

impl InstanceFlags {
    pub fn debugging() -> Self {
        InstanceFlags::DEBUG | InstanceFlags::VALIDATION
    }
}

#[derive(Debug, Clone, Default)]
pub struct InstanceDesc {
    pub backend: Backend,
    pub flags: InstanceFlags,
}
