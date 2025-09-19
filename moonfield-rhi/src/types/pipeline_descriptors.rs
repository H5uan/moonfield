//! Pipeline descriptor structures for RHI

use super::*;

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
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPipelineDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub program: Option<*mut std::ffi::c_void>, // IShaderProgram*
    pub input_layout: Option<*mut std::ffi::c_void>, // IInputLayout*
    pub primitive_topology: PrimitiveTopology,
    pub targets: Vec<ColorTargetDesc>,
    pub depth_stencil: DepthStencilDesc,
    pub rasterizer: RasterizerDesc,
    pub multisample: MultisampleDesc,
    
    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,
    
    pub label: Option<String>,
}

impl Default for RenderPipelineDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::RenderPipelineDesc,
            next: None,
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
#[derive(Debug, Clone, PartialEq)]
pub struct ComputePipelineDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub program: Option<*mut std::ffi::c_void>, // IShaderProgram*
    pub d3d12_root_signature_override: Option<*mut std::ffi::c_void>,
    
    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,
    
    pub label: Option<String>,
}

impl Default for ComputePipelineDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::ComputePipelineDesc,
            next: None,
            program: None,
            d3d12_root_signature_override: None,
            defer_target_compilation: false,
            label: None,
        }
    }
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
#[derive(Debug, Clone, PartialEq)]
pub struct RayTracingPipelineDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub program: Option<*mut std::ffi::c_void>, // IShaderProgram*
    pub hit_groups: Vec<HitGroupDesc>,
    pub max_recursion: u32,
    pub max_ray_payload_size: u32,
    pub max_attribute_size_in_bytes: u32,
    pub flags: RayTracingPipelineFlags,
    
    /// Defer target code compilation of program to dispatch time.
    pub defer_target_compilation: bool,
    
    pub label: Option<String>,
}

impl Default for RayTracingPipelineDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::RayTracingPipelineDesc,
            next: None,
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
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderTableDesc {
    pub struct_type: StructType,
    pub next: Option<*const std::ffi::c_void>,
    
    pub ray_gen_shader_entry_point_names: Vec<String>,
    pub ray_gen_shader_record_overwrites: Vec<ShaderRecordOverwrite>,
    
    pub miss_shader_entry_point_names: Vec<String>,
    pub miss_shader_record_overwrites: Vec<ShaderRecordOverwrite>,
    
    pub hit_group_names: Vec<String>,
    pub hit_group_record_overwrites: Vec<ShaderRecordOverwrite>,
    
    pub callable_shader_entry_point_names: Vec<String>,
    pub callable_shader_record_overwrites: Vec<ShaderRecordOverwrite>,
    
    pub program: Option<*mut std::ffi::c_void>, // IShaderProgram*
}

impl Default for ShaderTableDesc {
    fn default() -> Self {
        Self {
            struct_type: StructType::ShaderTableDesc,
            next: None,
            ray_gen_shader_entry_point_names: Vec::new(),
            ray_gen_shader_record_overwrites: Vec::new(),
            miss_shader_entry_point_names: Vec::new(),
            miss_shader_record_overwrites: Vec::new(),
            hit_group_names: Vec::new(),
            hit_group_record_overwrites: Vec::new(),
            callable_shader_entry_point_names: Vec::new(),
            callable_shader_record_overwrites: Vec::new(),
            program: None,
        }
    }
}
