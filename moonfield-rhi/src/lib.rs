use std::{
    fmt,
    hash::{Hash, Hasher},
    num::NonZeroU32,
    ops::Range,
    time::Duration,
};

use bitflags::Flags;
use bytemuck::{Pod, Zeroable};
use tracing::trace;

use crate::features::Features;

mod descriptor;
mod error;
mod features;
mod instance;

pub type BufferAddress = u64;
pub type BufferSize = core::num::NonZeroU64;
pub type ShaderLocation = u32;

/// Alignment requirement for transform buffers used in acceleration structure builds
pub const TRANSFORM_BUFFER_ALIGNMENT: BufferAddress = 16;

/// Alignment requirement for instance buffers used in acceleration structure builds (`build_acceleration_structures_unsafe_tlas`)
pub const INSTANCE_BUFFER_ALIGNMENT: BufferAddress = 16;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backend {
    Noop = 0,
    Vulkan = 1,
    Metal = 2,
    Dx12 = 3,
}

impl Backend {
    pub const COUNT: usize = 4;

    pub const ALL: [Backend; Self::COUNT] =
        [Self::Noop, Self::Vulkan, Self::Metal, Self::Dx12];

    #[must_use]
    pub const fn to_str(self) -> &'static str {
        match self {
            Backend::Noop => "noop",
            Backend::Vulkan => "vulkan",
            Backend::Metal => "metal",
            Backend::Dx12 => "dx12",
        }
    }

    /// Parse a backend from a string
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "noop" => Some(Self::Noop),
            "vulkan" => Some(Self::Vulkan),
            "metal" => Some(Self::Metal),
            "dx12" => Some(Self::Dx12),
            _ => None,
        }
    }

    /// Iterator over all backend variants
    pub fn iter() -> impl Iterator<Item = Backend> {
        Self::ALL.iter().copied()
    }
}

impl core::fmt::Display for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.to_str())
    }
}

impl core::str::FromStr for Backend {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str(s).ok_or(())
    }
}

bitflags::bitflags! {
    /// Represents the graphics backends that the RHI will use.
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct Backends: u32 {
        /// [`Backend::Noop`].
        const NOOP = 1 << Backend::Noop as u32;

        /// [`Backend::Vulkan`].
        /// Supported on Windows, Linux/Android, and macOS/iOS via Vulkan Portability (with the Vulkan feature enabled)
        const VULKAN = 1 << Backend::Vulkan as u32;

        /// [`Backend::Metal`].
        /// Supported on macOS and iOS.
        const METAL = 1 << Backend::Metal as u32;

        /// [`Backend::Dx12`].
        /// Supported on Windows 10 and later
        const DX12 = 1 << Backend::Dx12 as u32;

        /// All the graphics APIs that offer first tier of support.
        ///
        /// * [`Backends::VULKAN`]
        /// * [`Backends::METAL`]
        /// * [`Backends::DX12`]
        const PRIMARY = Self::VULKAN.bits()
            | Self::METAL.bits()
            | Self::DX12.bits();
    }
}

impl Default for Backends {
    fn default() -> Self {
        Self::all()
    }
}

impl From<Backend> for Backends {
    fn from(backend: Backend) -> Self {
        Self::from_bits(1 << backend as u32).unwrap()
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Maximum allowed value for the `size.width` of a texture created with `TextureDimension::D1`.
    /// Defaults to 8192. Higher is "better".
    pub max_texture_dimension_1d: u32,
    /// Maximum allowed value for the `size.width` and `size.height` of a texture created with `TextureDimension::D2`.
    /// Defaults to 8192. Higher is "better".
    pub max_texture_dimension_2d: u32,
    /// Maximum allowed value for the `size.width`, `size.height`, and `size.depth_or_array_layers`
    /// of a texture created with `TextureDimension::D3`.
    /// Defaults to 2048. Higher is "better".
    pub max_texture_dimension_3d: u32,
    /// Maximum allowed value for the `size.depth_or_array_layers` of a texture created with `TextureDimension::D2`.
    /// Defaults to 256. Higher is "better".
    pub max_texture_array_layers: u32,
    /// Amount of bind groups that can be attached to a pipeline at the same time. Defaults to 4. Higher is "better".
    pub max_bind_groups: u32,
    /// Maximum binding index allowed in `create_bind_group_layout`. Defaults to 1000. Higher is "better".
    pub max_bindings_per_bind_group: u32,
    /// Amount of uniform buffer bindings that can be dynamic in a single pipeline. Defaults to 8. Higher is "better".
    pub max_dynamic_uniform_buffers_per_pipeline_layout: u32,
    /// Amount of storage buffer bindings that can be dynamic in a single pipeline. Defaults to 4. Higher is "better".
    pub max_dynamic_storage_buffers_per_pipeline_layout: u32,
    /// Amount of sampled textures visible in a single shader stage. Defaults to 16. Higher is "better".
    pub max_sampled_textures_per_shader_stage: u32,
    /// Amount of samplers visible in a single shader stage. Defaults to 16. Higher is "better".
    pub max_samplers_per_shader_stage: u32,
    /// Amount of storage buffers visible in a single shader stage. Defaults to 8. Higher is "better".
    pub max_storage_buffers_per_shader_stage: u32,
    /// Amount of storage textures visible in a single shader stage. Defaults to 4. Higher is "better".
    pub max_storage_textures_per_shader_stage: u32,
    /// Amount of uniform buffers visible in a single shader stage. Defaults to 12. Higher is "better".
    pub max_uniform_buffers_per_shader_stage: u32,
    /// Amount of individual resources within binding arrays that can be accessed in a single shader stage. Applies
    /// to all types of bindings except samplers.
    ///
    /// This "defaults" to 0. However if binding arrays are supported, all devices can support 500,000. Higher is "better".
    pub max_binding_array_elements_per_shader_stage: u32,
    /// Amount of individual samplers within binding arrays that can be accessed in a single shader stage.
    ///
    /// This "defaults" to 0. However if binding arrays are supported, all devices can support 1,000. Higher is "better".
    pub max_binding_array_sampler_elements_per_shader_stage: u32,
    /// Maximum size in bytes of a binding to a uniform buffer. Defaults to 64 KiB. Higher is "better".
    pub max_uniform_buffer_binding_size: u32,
    /// Maximum size in bytes of a binding to a storage buffer. Defaults to 128 MiB. Higher is "better".
    pub max_storage_buffer_binding_size: u32,
    /// Maximum length of `VertexState::buffers` when creating a `RenderPipeline`.
    /// Defaults to 8. Higher is "better".
    pub max_vertex_buffers: u32,
    /// A limit above which buffer allocations are guaranteed to fail.
    /// Defaults to 256 MiB. Higher is "better".
    ///
    /// Buffer allocations below the maximum buffer size may not succeed depending on available memory,
    /// fragmentation and other factors.
    pub max_buffer_size: u64,
    /// Maximum length of `VertexBufferLayout::attributes`, summed over all `VertexState::buffers`,
    /// when creating a `RenderPipeline`.
    /// Defaults to 16. Higher is "better".
    pub max_vertex_attributes: u32,
    /// Maximum value for `VertexBufferLayout::array_stride` when creating a `RenderPipeline`.
    /// Defaults to 2048. Higher is "better".
    pub max_vertex_buffer_array_stride: u32,
    /// Required `BufferBindingType::Uniform` alignment for `BufferBinding::offset`
    /// when creating a `BindGroup`, or for `set_bind_group` `dynamicOffsets`.
    /// Defaults to 256. Lower is "better".
    pub min_uniform_buffer_offset_alignment: u32,
    /// Required `BufferBindingType::Storage` alignment for `BufferBinding::offset`
    /// when creating a `BindGroup`, or for `set_bind_group` `dynamicOffsets`.
    /// Defaults to 256. Lower is "better".
    pub min_storage_buffer_offset_alignment: u32,
    /// Maximum allowed number of components (scalars) of input or output locations for
    /// inter-stage communication (vertex outputs to fragment inputs). Defaults to 60.
    /// Higher is "better".
    pub max_inter_stage_shader_components: u32,
    /// The maximum allowed number of color attachments.
    pub max_color_attachments: u32,
    /// The maximum number of bytes necessary to hold one sample (pixel or subpixel) of render
    /// pipeline output data, across all color attachments as described by [`TextureFormat::target_pixel_byte_cost`]
    /// and [`TextureFormat::target_component_alignment`]. Defaults to 32. Higher is "better".
    ///
    /// ⚠️ `Rgba8Unorm`/`Rgba8Snorm`/`Bgra8Unorm`/`Bgra8Snorm` are deceptively 8 bytes per sample. ⚠️
    pub max_color_attachment_bytes_per_sample: u32,
    /// Maximum number of bytes used for workgroup memory in a compute entry point. Defaults to
    /// 16384. Higher is "better".
    pub max_compute_workgroup_storage_size: u32,
    /// Maximum value of the product of the `workgroup_size` dimensions for a compute entry-point.
    /// Defaults to 256. Higher is "better".
    pub max_compute_invocations_per_workgroup: u32,
    /// The maximum value of the `workgroup_size` X dimension for a compute stage `ShaderModule` entry-point.
    /// Defaults to 256. Higher is "better".
    pub max_compute_workgroup_size_x: u32,
    /// The maximum value of the `workgroup_size` Y dimension for a compute stage `ShaderModule` entry-point.
    /// Defaults to 256. Higher is "better".
    pub max_compute_workgroup_size_y: u32,
    /// The maximum value of the `workgroup_size` Z dimension for a compute stage `ShaderModule` entry-point.
    /// Defaults to 64. Higher is "better".
    pub max_compute_workgroup_size_z: u32,
    /// The maximum value for each dimension of a `ComputePass::dispatch(x, y, z)` operation.
    /// Defaults to 65535. Higher is "better".
    pub max_compute_workgroups_per_dimension: u32,

    /// Minimal number of invocations in a subgroup. Lower is "better".
    pub min_subgroup_size: u32,
    /// Maximal number of invocations in a subgroup. Higher is "better".
    pub max_subgroup_size: u32,
    /// Amount of storage available for push constants in bytes. Defaults to 0. Higher is "better".
    /// Requesting more than 0 during device creation requires [`Features::PUSH_CONSTANTS`] to be enabled.
    ///
    /// Expect the size to be:
    /// - Vulkan: 128-256 bytes
    /// - DX12: 256 bytes
    /// - Metal: 4096 bytes
    /// - OpenGL doesn't natively support push constants, and are emulated with uniforms,
    ///   so this number is less useful but likely 256.
    pub max_push_constant_size: u32,
    /// Maximum number of live non-sampler bindings.
    ///
    /// This limit only affects the d3d12 backend. Using a large number will allow the device
    /// to create many bind groups at the cost of a large up-front allocation at device creation.
    pub max_non_sampler_bindings: u32,

    /// The maximum total value of x*y*z for a given `draw_mesh_tasks` command
    pub max_task_workgroup_total_count: u32,
    /// The maximum value for each dimension of a `RenderPass::draw_mesh_tasks(x, y, z)` operation.
    /// Defaults to 65535. Higher is "better".
    pub max_task_workgroups_per_dimension: u32,
    /// The maximum number of layers that can be output from a mesh shader
    pub max_mesh_output_layers: u32,
    /// The maximum number of views that can be used by a mesh shader
    pub max_mesh_multiview_count: u32,

    /// The maximum number of primitive (ex: triangles, aabbs) a BLAS is allowed to have. Requesting
    /// more than 0 during device creation only makes sense if [`Features::EXPERIMENTAL_RAY_QUERY`]
    /// is enabled.
    pub max_blas_primitive_count: u32,
    /// The maximum number of geometry descriptors a BLAS is allowed to have. Requesting
    /// more than 0 during device creation only makes sense if [`Features::EXPERIMENTAL_RAY_QUERY`]
    /// is enabled.
    pub max_blas_geometry_count: u32,
    /// The maximum number of instances a TLAS is allowed to have. Requesting more than 0 during
    /// device creation only makes sense if [`Features::EXPERIMENTAL_RAY_QUERY`]
    /// is enabled.
    pub max_tlas_instance_count: u32,
    /// The maximum number of acceleration structures allowed to be used in a shader stage.
    /// Requesting more than 0 during device creation only makes sense if [`Features::EXPERIMENTAL_RAY_QUERY`]
    /// is enabled.
    pub max_acceleration_structures_per_shader_stage: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Limits {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            max_texture_dimension_1d: 8192,
            max_texture_dimension_2d: 8192,
            max_texture_dimension_3d: 2048,
            max_texture_array_layers: 256,
            max_bind_groups: 4,
            max_bindings_per_bind_group: 1000,
            max_dynamic_uniform_buffers_per_pipeline_layout: 8,
            max_dynamic_storage_buffers_per_pipeline_layout: 4,
            max_sampled_textures_per_shader_stage: 16,
            max_samplers_per_shader_stage: 16,
            max_storage_buffers_per_shader_stage: 8,
            max_storage_textures_per_shader_stage: 4,
            max_uniform_buffers_per_shader_stage: 12,
            max_binding_array_elements_per_shader_stage: 0,
            max_binding_array_sampler_elements_per_shader_stage: 0,
            max_uniform_buffer_binding_size: 64 << 10, // (64 KiB)
            max_storage_buffer_binding_size: 128 << 20, // (128 MiB)
            max_vertex_buffers: 8,
            max_buffer_size: 256 << 20, // (256 MiB)
            max_vertex_attributes: 16,
            max_vertex_buffer_array_stride: 2048,
            min_uniform_buffer_offset_alignment: 256,
            min_storage_buffer_offset_alignment: 256,
            max_inter_stage_shader_components: 60,
            max_color_attachments: 8,
            max_color_attachment_bytes_per_sample: 32,
            max_compute_workgroup_storage_size: 16384,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 256,
            max_compute_workgroup_size_z: 64,
            max_compute_workgroups_per_dimension: 65535,
            min_subgroup_size: 0,
            max_subgroup_size: 0,
            max_push_constant_size: 0,
            max_non_sampler_bindings: 1_000_000,

            max_task_workgroup_total_count: 0,
            max_task_workgroups_per_dimension: 0,
            max_mesh_multiview_count: 0,
            max_mesh_output_layers: 0,

            max_blas_primitive_count: 0,
            max_blas_geometry_count: 0,
            max_tlas_instance_count: 0,
            max_acceleration_structures_per_shader_stage: 0,
        }
    }

    #[must_use]
    pub const fn using_resolution(self, other: Self) -> Self {
        Self {
            max_texture_dimension_1d: other.max_texture_dimension_1d,
            max_texture_dimension_2d: other.max_texture_dimension_2d,
            max_texture_dimension_3d: other.max_texture_dimension_3d,
            ..self
        }
    }

    #[must_use]
    pub const fn using_alignment(self, other: Self) -> Self {
        Self {
            min_uniform_buffer_offset_alignment: other
                .min_uniform_buffer_offset_alignment,
            min_storage_buffer_offset_alignment: other
                .min_storage_buffer_offset_alignment,
            ..self
        }
    }

    #[must_use]
    pub const fn using_minimum_supported_acceleration_structure_values(
        self,
    ) -> Self {
        Self {
            max_blas_geometry_count: (1 << 24) - 1, // 2^24 - 1: Vulkan's minimum
            max_tlas_instance_count: (1 << 24) - 1, // 2^24 - 1: Vulkan's minimum
            max_blas_primitive_count: (1 << 24) - 1, // Should be 2^28: Metal's minimum, but due to an llvmpipe bug it is 2^24 - 1
            max_acceleration_structures_per_shader_stage: 16, // Vulkan's minimum
            ..self
        }
    }

    #[must_use]
    pub const fn using_acceleration_structure_values(
        self, other: Self,
    ) -> Self {
        Self {
            max_blas_geometry_count: other.max_blas_geometry_count,
            max_tlas_instance_count: other.max_tlas_instance_count,
            max_blas_primitive_count: other.max_blas_primitive_count,
            max_acceleration_structures_per_shader_stage: other
                .max_acceleration_structures_per_shader_stage,
            ..self
        }
    }

    #[must_use]
    pub const fn using_recommended_minimum_mesh_shader_values(self) -> Self {
        Self {
            // Literally just made this up as 256^2 or 2^16.
            // My GPU supports 2^22, and compute shaders don't have this kind of limit.
            // This very likely is never a real limiter
            max_task_workgroup_total_count: 65536,
            max_task_workgroups_per_dimension: 256,
            // llvmpipe reports 0 multiview count, which just means no multiview is allowed
            max_mesh_multiview_count: 0,
            // llvmpipe once again requires this to be 8. An RTX 3060 supports well over 1024.
            max_mesh_output_layers: 8,
            ..self
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaderModel {
    Sm2,
    Sm4,
    Sm5,
}

/// Supported physical device types.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DeviceType {
    /// Other or Unknown.
    Other,
    /// Integrated GPU with shared CPU/GPU memory.
    IntegratedGpu,
    /// Discrete GPU with separate CPU/GPU memory.
    DiscreteGpu,
    /// Virtual / Hosted.
    VirtualGpu,
    /// Cpu / Software Rendering.
    Cpu,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AdapterInfo {
    /// Adapter name
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    /// Type of device
    pub device_type: DeviceType,
    pub device_pci_bus_id: String,
    /// Driver name
    pub driver: String,
    /// Driver info
    pub driver_info: String,
    /// Backend used for device
    pub backend: Backend,
    /// If true, adding [`TextureUsages::TRANSIENT`] to a texture will decrease memory usage.
    pub transient_saves_memory: bool,
}

#[derive(Clone, Debug, Default)]
pub enum MemoryHints {
    /// Favor performance over memory usage (the default value).
    #[default]
    Performance,
    /// Favor memory usage over performance.
    MemoryUsage,
    /// Applications that have control over the content that is rendered
    /// (typically games) may find an optimal compromise between memory
    /// usage and performance by specifying the allocation configuration.
    Manual {
        /// Defines the range of allowed memory block sizes for sub-allocated
        /// resources.
        ///
        /// The backend may attempt to group multiple resources into fewer
        /// device memory blocks (sub-allocation) for performance reasons.
        /// The start of the provided range specifies the initial memory
        /// block size for sub-allocated resources. After running out of
        /// space in existing memory blocks, the backend may chose to
        /// progressively increase the block size of subsequent allocations
        /// up to a limit specified by the end of the range.
        ///
        /// This does not limit resource sizes. If a resource does not fit
        /// in the specified range, it will typically be placed in a dedicated
        /// memory block.
        suballocated_device_memory_block_size: Range<u64>,
    },
}

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct ShaderStages: u32 {
        /// Binding is not visible from any shader stage.
        const NONE = 0;
        const VERTEX = 1 << 0;
        const FRAGMENT = 1 << 1;
        const COMPUTE = 1 << 2;
        const VERTEX_FRAGMENT = Self::VERTEX.bits() | Self::FRAGMENT.bits();
        const TASK = 1 << 3;
        const MESH = 1 << 4;
    }
}

/// Order in which texture data is laid out in memory.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub enum TextureDataOrder {
    /// The texture is laid out densely in memory as:
    ///
    /// ```text
    /// Layer0Mip0 Layer0Mip1 Layer0Mip2
    /// Layer1Mip0 Layer1Mip1 Layer1Mip2
    /// Layer2Mip0 Layer2Mip1 Layer2Mip2
    /// ````
    ///
    /// This is the layout used by dds files.
    #[default]
    LayerMajor,
    /// The texture is laid out densely in memory as:
    ///
    /// ```text
    /// Layer0Mip0 Layer1Mip0 Layer2Mip0
    /// Layer0Mip1 Layer1Mip1 Layer2Mip1
    /// Layer0Mip2 Layer1Mip2 Layer2Mip2
    /// ```
    ///
    /// This is the layout used by ktx and ktx2 files.
    MipMajor,
}

/// Dimensionality of a texture.
#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum TextureDimension {
    /// 1D texture
    D1,
    /// 2D texture
    D2,
    /// 3D texture
    D3,
}

/// Dimensions of a particular texture view.
#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum TextureViewDimension {
    D1,
    D2,
    D2Array,
    Cube,
    CubeArray,
    D3,
}

impl TextureViewDimension {
    #[must_use]
    pub fn compatible_texture_dimension(self) -> TextureDimension {
        match self {
            Self::D1 => TextureDimension::D1,
            Self::D2 | Self::D2Array | Self::Cube | Self::CubeArray => {
                TextureDimension::D2
            }
            Self::D3 => TextureDimension::D3,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum BlendFactor {
    /// 0.0
    Zero = 0,
    /// 1.0
    One = 1,
    /// S.component
    Src = 2,
    /// 1.0 - S.component
    OneMinusSrc = 3,
    /// S.alpha
    SrcAlpha = 4,
    /// 1.0 - S.alpha
    OneMinusSrcAlpha = 5,
    /// D.component
    Dst = 6,
    /// 1.0 - D.component
    OneMinusDst = 7,
    /// D.alpha
    DstAlpha = 8,
    /// 1.0 - D.alpha
    OneMinusDstAlpha = 9,
    /// min(S.alpha, 1.0 - D.alpha)
    SrcAlphaSaturated = 10,
    /// Constant
    Constant = 11,
    /// 1.0 - Constant
    OneMinusConstant = 12,
    /// S1.component
    Src1 = 13,
    /// 1.0 - S1.component
    OneMinusSrc1 = 14,
    /// S1.alpha
    Src1Alpha = 15,
    /// 1.0 - S1.alpha
    OneMinusSrc1Alpha = 16,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum BlendOperation {
    /// Src + Dst
    #[default]
    Add = 0,
    /// Src - Dst
    Subtract = 1,
    /// Dst - Src
    ReverseSubtract = 2,
    /// min(Src, Dst)
    Min = 3,
    /// max(Src, Dst)
    Max = 4,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendComponent {
    /// Multiplier for the source, which is produced by the fragment shader.
    pub src_factor: BlendFactor,
    /// Multiplier for the destination, which is stored in the target.
    pub dst_factor: BlendFactor,
    /// The binary operation applied to the source and destination,
    /// multiplied by their respective factors.
    pub operation: BlendOperation,
}

impl BlendComponent {
    /// Default blending state that replaces destination with the source.
    pub const REPLACE: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::Zero,
        operation: BlendOperation::Add,
    };

    /// Blend state of `(1 * src) + ((1 - src_alpha) * dst)`.
    pub const OVER: Self = Self {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    };

    /// Returns true if the state relies on the constant color, which is
    /// set independently on a render command encoder.
    #[must_use]
    pub fn uses_constant(&self) -> bool {
        match (self.src_factor, self.dst_factor) {
            (BlendFactor::Constant, _)
            | (BlendFactor::OneMinusConstant, _)
            | (_, BlendFactor::Constant)
            | (_, BlendFactor::OneMinusConstant) => true,
            (_, _) => false,
        }
    }
}

impl Default for BlendComponent {
    fn default() -> Self {
        Self::REPLACE
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendState {
    /// Color equation.
    pub color: BlendComponent,
    /// Alpha equation.
    pub alpha: BlendComponent,
}

impl BlendState {
    /// Blend mode that does no color blending, just overwrites the output with the contents of the shader.
    pub const REPLACE: Self = Self {
        color: BlendComponent::REPLACE,
        alpha: BlendComponent::REPLACE,
    };

    /// Blend mode that does standard alpha blending with non-premultiplied alpha.
    pub const ALPHA_BLENDING: Self = Self {
        color: BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::OVER,
    };

    /// Blend mode that does standard alpha blending with premultiplied alpha.
    pub const PREMULTIPLIED_ALPHA_BLENDING: Self =
        Self { color: BlendComponent::OVER, alpha: BlendComponent::OVER };
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ColorTargetState {
    pub format: TextureFormat,
    pub blend: Option<BlendState>,
    pub write_mask: ColorWrites,
}

impl From<TextureFormat> for ColorTargetState {
    fn from(format: TextureFormat) -> Self {
        Self { format, blend: None, write_mask: ColorWrites::ALL }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum PrimitiveTopology {
    /// Vertex data is a list of points. Each vertex is a new point.
    PointList = 0,
    /// Vertex data is a list of lines. Each pair of vertices composes a new line.
    ///
    /// Vertices `0 1 2 3` create two lines `0 1` and `2 3`
    LineList = 1,
    /// Vertex data is a strip of lines. Each set of two adjacent vertices form a line.
    ///
    /// Vertices `0 1 2 3` create three lines `0 1`, `1 2`, and `2 3`.
    LineStrip = 2,
    /// Vertex data is a list of triangles. Each set of 3 vertices composes a new triangle.
    ///
    /// Vertices `0 1 2 3 4 5` create two triangles `0 1 2` and `3 4 5`
    #[default]
    TriangleList = 3,
    /// Vertex data is a triangle strip. Each set of three adjacent vertices form a triangle.
    ///
    /// Vertices `0 1 2 3 4 5` create four triangles `0 1 2`, `2 1 3`, `2 3 4`, and `4 3 5`
    TriangleStrip = 4,
}

impl PrimitiveTopology {
    /// Returns true for strip topologies.
    #[must_use]
    pub fn is_strip(&self) -> bool {
        match *self {
            Self::PointList | Self::LineList | Self::TriangleList => false,
            Self::LineStrip | Self::TriangleStrip => true,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrontFace {
    /// Triangles with vertices in counter clockwise order are considered the front face.
    ///
    /// This is the default with right handed coordinate spaces.
    #[default]
    Ccw = 0,
    /// Triangles with vertices in clockwise order are considered the front face.
    ///
    /// This is the default with left handed coordinate spaces.
    Cw = 1,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    /// Front face
    Front = 0,
    /// Back face
    Back = 1,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PolygonMode {
    /// Polygons are filled
    #[default]
    Fill = 0,
    /// Polygons are drawn as line segments
    Line = 1,
    /// Polygons are drawn as points
    Point = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PrimitiveState {
    /// The primitive topology used to interpret vertices.
    pub topology: PrimitiveTopology,
    /// When drawing strip topologies with indices, this is the required format for the index buffer.
    /// This has no effect on non-indexed or non-strip draws.
    ///
    /// Specifying this value enables primitive restart, allowing individual strips to be separated
    /// with the index value `0xFFFF` when using `Uint16`, or `0xFFFFFFFF` when using `Uint32`.
    pub strip_index_format: Option<IndexFormat>,
    /// The face to consider the front for the purpose of culling and stencil operations.
    pub front_face: FrontFace,
    /// The face culling mode.
    pub cull_mode: Option<Face>,
    /// If set to true, the polygon depth is not clipped to 0-1 before rasterization.
    ///
    /// Enabling this requires [`Features::DEPTH_CLIP_CONTROL`] to be enabled.
    pub unclipped_depth: bool,
    /// Controls the way each polygon is rasterized. Can be either `Fill` (default), `Line` or `Point`
    ///
    /// Setting this to `Line` requires [`Features::POLYGON_MODE_LINE`] to be enabled.
    ///
    /// Setting this to `Point` requires [`Features::POLYGON_MODE_POINT`] to be enabled.
    pub polygon_mode: PolygonMode,
    /// If set to true, the primitives are rendered with conservative overestimation. I.e. any rastered pixel touched by it is filled.
    /// Only valid for `[PolygonMode::Fill`]!
    ///
    /// Enabling this requires [`Features::CONSERVATIVE_RASTERIZATION`] to be enabled.
    pub conservative: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MultisampleState {
    /// The number of samples calculated per pixel (for MSAA). For non-multisampled textures,
    /// this should be `1`
    pub count: u32,
    /// Bitmask that restricts the samples of a pixel modified by this pipeline. All samples
    /// can be enabled using the value `!0`
    pub mask: u64,
    /// When enabled, produces another sample mask per pixel based on the alpha output value, that
    /// is ANDed with the sample mask and the primitive coverage to restrict the set of samples
    /// affected by a primitive.
    ///
    /// The implicit mask produced for alpha of zero is guaranteed to be zero, and for alpha of one
    /// is guaranteed to be all 1-s.
    pub alpha_to_coverage_enabled: bool,
}

impl Default for MultisampleState {
    fn default() -> Self {
        MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        }
    }
}

bitflags::bitflags! {
    /// Feature flags for a texture format.
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct TextureFormatFeatureFlags: u32 {
        /// If not present, the texture can't be sampled with a filtering sampler.
        /// This may overwrite TextureSampleType::Float.filterable
        const FILTERABLE = 1 << 0;
        /// Allows [`TextureDescriptor::sample_count`] to be `2`.
        const MULTISAMPLE_X2 = 1 << 1;
        /// Allows [`TextureDescriptor::sample_count`] to be `4`.
        const MULTISAMPLE_X4 = 1 << 2 ;
        /// Allows [`TextureDescriptor::sample_count`] to be `8`.
        const MULTISAMPLE_X8 = 1 << 3 ;
        /// Allows [`TextureDescriptor::sample_count`] to be `16`.
        const MULTISAMPLE_X16 = 1 << 4;
        /// Allows a texture of this format to back a view passed as `resolve_target`
        /// to a render pass for an automatic driver-implemented resolve.
        const MULTISAMPLE_RESOLVE = 1 << 5;
        /// When used as a STORAGE texture, then a texture with this format can be bound with
        /// [`StorageTextureAccess::ReadOnly`].
        const STORAGE_READ_ONLY = 1 << 6;
        /// When used as a STORAGE texture, then a texture with this format can be bound with
        /// [`StorageTextureAccess::WriteOnly`].
        const STORAGE_WRITE_ONLY = 1 << 7;
        /// When used as a STORAGE texture, then a texture with this format can be bound with
        /// [`StorageTextureAccess::ReadWrite`].
        const STORAGE_READ_WRITE = 1 << 8;
        /// When used as a STORAGE texture, then a texture with this format can be bound with
        /// [`StorageTextureAccess::Atomic`].
        const STORAGE_ATOMIC = 1 << 9;
        /// If not present, the texture can't be blended into the render target.
        const BLENDABLE = 1 << 10;
    }
}

impl TextureFormatFeatureFlags {
    /// Sample count supported by a given texture format.
    ///
    /// returns `true` if `count` is a supported sample count.
    #[must_use]
    pub fn sample_count_supported(&self, count: u32) -> bool {
        use TextureFormatFeatureFlags as tfsc;

        match count {
            1 => true,
            2 => self.contains(tfsc::MULTISAMPLE_X2),
            4 => self.contains(tfsc::MULTISAMPLE_X4),
            8 => self.contains(tfsc::MULTISAMPLE_X8),
            16 => self.contains(tfsc::MULTISAMPLE_X16),
            _ => false,
        }
    }

    /// A `Vec` of supported sample counts.
    #[must_use]
    pub fn supported_sample_counts(&self) -> Vec<u32> {
        let all_possible_sample_counts: [u32; 5] = [1, 2, 4, 8, 16];
        all_possible_sample_counts
            .into_iter()
            .filter(|&sc| self.sample_count_supported(sc))
            .collect()
    }
}

/// Features supported by a given texture format
///
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct TextureFormatFeatures {
    /// Valid bits for `TextureDescriptor::Usage` provided for format creation.
    pub allowed_usages: TextureUsages,
    /// Additional property flags for the format.
    pub flags: TextureFormatFeatureFlags,
}

/// ASTC block dimensions
#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum AstcBlock {
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px).
    B4x4,
    /// 5x4 block compressed texture. 16 bytes per block (6.4 bit/px).
    B5x4,
    /// 5x5 block compressed texture. 16 bytes per block (5.12 bit/px).
    B5x5,
    /// 6x5 block compressed texture. 16 bytes per block (4.27 bit/px).
    B6x5,
    /// 6x6 block compressed texture. 16 bytes per block (3.56 bit/px).
    B6x6,
    /// 8x5 block compressed texture. 16 bytes per block (3.2 bit/px).
    B8x5,
    /// 8x6 block compressed texture. 16 bytes per block (2.67 bit/px).
    B8x6,
    /// 8x8 block compressed texture. 16 bytes per block (2 bit/px).
    B8x8,
    /// 10x5 block compressed texture. 16 bytes per block (2.56 bit/px).
    B10x5,
    /// 10x6 block compressed texture. 16 bytes per block (2.13 bit/px).
    B10x6,
    /// 10x8 block compressed texture. 16 bytes per block (1.6 bit/px).
    B10x8,
    /// 10x10 block compressed texture. 16 bytes per block (1.28 bit/px).
    B10x10,
    /// 12x10 block compressed texture. 16 bytes per block (1.07 bit/px).
    B12x10,
    /// 12x12 block compressed texture. 16 bytes per block (0.89 bit/px).
    B12x12,
}

/// ASTC RGBA channel
#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum AstcChannel {
    /// 8 bit integer RGBA, [0, 255] converted to/from linear-color float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ASTC`] must be enabled to use this channel.
    Unorm,
    /// 8 bit integer RGBA, Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ASTC`] must be enabled to use this channel.
    UnormSrgb,
    /// floating-point RGBA, linear-color float can be outside of the [0, 1] range.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ASTC_HDR`] must be enabled to use this channel.
    Hdr,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum TextureFormat {
    // Normal 8 bit formats
    /// Red channel only. 8 bit integer per channel. [0, 255] converted to/from float [0, 1] in shader.
    R8Unorm,
    /// Red channel only. 8 bit integer per channel. [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    R8Snorm,
    /// Red channel only. 8 bit integer per channel. Unsigned in shader.
    R8Uint,
    /// Red channel only. 8 bit integer per channel. Signed in shader.
    R8Sint,

    // Normal 16 bit formats
    /// Red channel only. 16 bit integer per channel. Unsigned in shader.
    R16Uint,
    /// Red channel only. 16 bit integer per channel. Signed in shader.
    R16Sint,
    /// Red channel only. 16 bit integer per channel. [0, 65535] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    R16Unorm,
    /// Red channel only. 16 bit integer per channel. [&minus;32767, 32767] converted to/from float [&minus;1, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    R16Snorm,
    /// Red channel only. 16 bit float per channel. Float in shader.
    R16Float,
    /// Red and green channels. 8 bit integer per channel. [0, 255] converted to/from float [0, 1] in shader.
    Rg8Unorm,
    /// Red and green channels. 8 bit integer per channel. [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    Rg8Snorm,
    /// Red and green channels. 8 bit integer per channel. Unsigned in shader.
    Rg8Uint,
    /// Red and green channels. 8 bit integer per channel. Signed in shader.
    Rg8Sint,

    // Normal 32 bit formats
    /// Red channel only. 32 bit integer per channel. Unsigned in shader.
    R32Uint,
    /// Red channel only. 32 bit integer per channel. Signed in shader.
    R32Sint,
    /// Red channel only. 32 bit float per channel. Float in shader.
    R32Float,
    /// Red and green channels. 16 bit integer per channel. Unsigned in shader.
    Rg16Uint,
    /// Red and green channels. 16 bit integer per channel. Signed in shader.
    Rg16Sint,
    /// Red and green channels. 16 bit integer per channel. [0, 65535] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    Rg16Unorm,
    /// Red and green channels. 16 bit integer per channel. [&minus;32767, 32767] converted to/from float [&minus;1, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    Rg16Snorm,
    /// Red and green channels. 16 bit float per channel. Float in shader.
    Rg16Float,
    /// Red, green, blue, and alpha channels. 8 bit integer per channel. [0, 255] converted to/from float [0, 1] in shader.
    Rgba8Unorm,
    /// Red, green, blue, and alpha channels. 8 bit integer per channel. Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    Rgba8UnormSrgb,
    /// Red, green, blue, and alpha channels. 8 bit integer per channel. [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    Rgba8Snorm,
    /// Red, green, blue, and alpha channels. 8 bit integer per channel. Unsigned in shader.
    Rgba8Uint,
    /// Red, green, blue, and alpha channels. 8 bit integer per channel. Signed in shader.
    Rgba8Sint,
    /// Blue, green, red, and alpha channels. 8 bit integer per channel. [0, 255] converted to/from float [0, 1] in shader.
    Bgra8Unorm,
    /// Blue, green, red, and alpha channels. 8 bit integer per channel. Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    Bgra8UnormSrgb,

    // Packed 32 bit formats
    /// Packed unsigned float with 9 bits mantisa for each RGB component, then a common 5 bits exponent
    Rgb9e5Ufloat,
    /// Red, green, blue, and alpha channels. 10 bit integer for RGB channels, 2 bit integer for alpha channel. Unsigned in shader.
    Rgb10a2Uint,
    /// Red, green, blue, and alpha channels. 10 bit integer for RGB channels, 2 bit integer for alpha channel. [0, 1023] ([0, 3] for alpha) converted to/from float [0, 1] in shader.
    Rgb10a2Unorm,
    /// Red, green, and blue channels. 11 bit float with no sign bit for RG channels. 10 bit float with no sign bit for blue channel. Float in shader.
    Rg11b10Ufloat,

    // Normal 64 bit formats
    /// Red channel only. 64 bit integer per channel. Unsigned in shader.
    ///
    /// [`Features::TEXTURE_INT64_ATOMIC`] must be enabled to use this texture format.
    R64Uint,
    /// Red and green channels. 32 bit integer per channel. Unsigned in shader.
    Rg32Uint,
    /// Red and green channels. 32 bit integer per channel. Signed in shader.
    Rg32Sint,
    /// Red and green channels. 32 bit float per channel. Float in shader.
    Rg32Float,
    /// Red, green, blue, and alpha channels. 16 bit integer per channel. Unsigned in shader.
    Rgba16Uint,
    /// Red, green, blue, and alpha channels. 16 bit integer per channel. Signed in shader.
    Rgba16Sint,
    /// Red, green, blue, and alpha channels. 16 bit integer per channel. [0, 65535] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    Rgba16Unorm,
    /// Red, green, blue, and alpha. 16 bit integer per channel. [&minus;32767, 32767] converted to/from float [&minus;1, 1] in shader.
    ///
    /// [`Features::TEXTURE_FORMAT_16BIT_NORM`] must be enabled to use this texture format.
    Rgba16Snorm,
    /// Red, green, blue, and alpha channels. 16 bit float per channel. Float in shader.
    Rgba16Float,

    // Normal 128 bit formats
    /// Red, green, blue, and alpha channels. 32 bit integer per channel. Unsigned in shader.
    Rgba32Uint,
    /// Red, green, blue, and alpha channels. 32 bit integer per channel. Signed in shader.
    Rgba32Sint,
    /// Red, green, blue, and alpha channels. 32 bit float per channel. Float in shader.
    Rgba32Float,

    // Depth and stencil formats
    /// Stencil format with 8 bit integer stencil.
    Stencil8,
    /// Special depth format with 16 bit integer depth.
    Depth16Unorm,
    /// Special depth format with at least 24 bit integer depth.
    Depth24Plus,
    /// Special depth/stencil format with at least 24 bit integer depth and 8 bits integer stencil.
    Depth24PlusStencil8,
    /// Special depth format with 32 bit floating point depth.
    Depth32Float,
    /// Special depth/stencil format with 32 bit floating point depth and 8 bits integer stencil.
    ///
    /// [`Features::DEPTH32FLOAT_STENCIL8`] must be enabled to use this texture format.
    Depth32FloatStencil8,

    /// YUV 4:2:0 chroma subsampled format.
    ///
    /// Contains two planes:
    /// - 0: Single 8 bit channel luminance.
    /// - 1: Dual 8 bit channel chrominance at half width and half height.
    ///
    /// Valid view formats for luminance are [`TextureFormat::R8Unorm`].
    ///
    /// Valid view formats for chrominance are [`TextureFormat::Rg8Unorm`].
    ///
    /// Width and height must be even.
    ///
    /// [`Features::TEXTURE_FORMAT_NV12`] must be enabled to use this texture format.
    NV12,

    /// YUV 4:2:0 chroma subsampled format.
    ///
    /// Contains two planes:
    /// - 0: Single 16 bit channel luminance, of which only the high 10 bits
    ///   are used.
    /// - 1: Dual 16 bit channel chrominance at half width and half height, of
    ///   which only the high 10 bits are used.
    ///
    /// Valid view formats for luminance are [`TextureFormat::R16Unorm`].
    ///
    /// Valid view formats for chrominance are [`TextureFormat::Rg16Unorm`].
    ///
    /// Width and height must be even.
    ///
    /// [`Features::TEXTURE_FORMAT_P010`] must be enabled to use this texture format.
    P010,

    // Compressed textures usable with `TEXTURE_COMPRESSION_BC` feature. `TEXTURE_COMPRESSION_SLICED_3D` is required to use with 3D textures.
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). 4 color + alpha pallet. 5 bit R + 6 bit G + 5 bit B + 1 bit alpha.
    /// [0, 63] ([0, 1] for alpha) converted to/from float [0, 1] in shader.
    ///
    /// Also known as DXT1.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc1RgbaUnorm,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). 4 color + alpha pallet. 5 bit R + 6 bit G + 5 bit B + 1 bit alpha.
    /// Srgb-color [0, 63] ([0, 1] for alpha) converted to/from linear-color float [0, 1] in shader.
    ///
    /// Also known as DXT1.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc1RgbaUnormSrgb,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 4 color pallet. 5 bit R + 6 bit G + 5 bit B + 4 bit alpha.
    /// [0, 63] ([0, 15] for alpha) converted to/from float [0, 1] in shader.
    ///
    /// Also known as DXT3.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc2RgbaUnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 4 color pallet. 5 bit R + 6 bit G + 5 bit B + 4 bit alpha.
    /// Srgb-color [0, 63] ([0, 255] for alpha) converted to/from linear-color float [0, 1] in shader.
    ///
    /// Also known as DXT3.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc2RgbaUnormSrgb,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 4 color pallet + 8 alpha pallet. 5 bit R + 6 bit G + 5 bit B + 8 bit alpha.
    /// [0, 63] ([0, 255] for alpha) converted to/from float [0, 1] in shader.
    ///
    /// Also known as DXT5.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc3RgbaUnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 4 color pallet + 8 alpha pallet. 5 bit R + 6 bit G + 5 bit B + 8 bit alpha.
    /// Srgb-color [0, 63] ([0, 255] for alpha) converted to/from linear-color float [0, 1] in shader.
    ///
    /// Also known as DXT5.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc3RgbaUnormSrgb,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). 8 color pallet. 8 bit R.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// Also known as RGTC1.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc4RUnorm,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). 8 color pallet. 8 bit R.
    /// [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    ///
    /// Also known as RGTC1.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc4RSnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 8 color red pallet + 8 color green pallet. 8 bit RG.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// Also known as RGTC2.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc5RgUnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). 8 color red pallet + 8 color green pallet. 8 bit RG.
    /// [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    ///
    /// Also known as RGTC2.
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc5RgSnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Variable sized pallet. 16 bit unsigned float RGB. Float in shader.
    ///
    /// Also known as BPTC (float).
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc6hRgbUfloat,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Variable sized pallet. 16 bit signed float RGB. Float in shader.
    ///
    /// Also known as BPTC (float).
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc6hRgbFloat,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Variable sized pallet. 8 bit integer RGBA.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// Also known as BPTC (unorm).
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc7RgbaUnorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Variable sized pallet. 8 bit integer RGBA.
    /// Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    ///
    /// Also known as BPTC (unorm).
    ///
    /// [`Features::TEXTURE_COMPRESSION_BC`] must be enabled to use this texture format.
    /// [`Features::TEXTURE_COMPRESSION_BC_SLICED_3D`] must be enabled to use this texture format with 3D dimension.
    Bc7RgbaUnormSrgb,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 8 bit integer RGB.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgb8Unorm,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 8 bit integer RGB.
    /// Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgb8UnormSrgb,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 8 bit integer RGB + 1 bit alpha.
    /// [0, 255] ([0, 1] for alpha) converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgb8A1Unorm,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 8 bit integer RGB + 1 bit alpha.
    /// Srgb-color [0, 255] ([0, 1] for alpha) converted to/from linear-color float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgb8A1UnormSrgb,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Complex pallet. 8 bit integer RGB + 8 bit alpha.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgba8Unorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Complex pallet. 8 bit integer RGB + 8 bit alpha.
    /// Srgb-color [0, 255] converted to/from linear-color float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    Etc2Rgba8UnormSrgb,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 11 bit integer R.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    EacR11Unorm,
    /// 4x4 block compressed texture. 8 bytes per block (4 bit/px). Complex pallet. 11 bit integer R.
    /// [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    EacR11Snorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Complex pallet. 11 bit integer R + 11 bit integer G.
    /// [0, 255] converted to/from float [0, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    EacRg11Unorm,
    /// 4x4 block compressed texture. 16 bytes per block (8 bit/px). Complex pallet. 11 bit integer R + 11 bit integer G.
    /// [&minus;127, 127] converted to/from float [&minus;1, 1] in shader.
    ///
    /// [`Features::TEXTURE_COMPRESSION_ETC2`] must be enabled to use this texture format.
    EacRg11Snorm,
    /// block compressed texture. 16 bytes per block.
    ///
    /// Features [`TEXTURE_COMPRESSION_ASTC`] or [`TEXTURE_COMPRESSION_ASTC_HDR`]
    /// must be enabled to use this texture format.
    ///
    /// [`TEXTURE_COMPRESSION_ASTC`]: Features::TEXTURE_COMPRESSION_ASTC
    /// [`TEXTURE_COMPRESSION_ASTC_HDR`]: Features::TEXTURE_COMPRESSION_ASTC_HDR
    Astc {
        /// compressed block dimensions
        block: AstcBlock,
        /// ASTC RGBA channel
        channel: AstcChannel,
    },
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum TextureAspect {
    /// Depth, Stencil, and Color.
    #[default]
    All,
    /// Stencil.
    StencilOnly,
    /// Depth.
    DepthOnly,
    /// Plane 0.
    Plane0,
    /// Plane 1.
    Plane1,
    /// Plane 2.
    Plane2,
}

impl TextureFormat {
    /// Returns the aspect-specific format of the original format
    ///
    #[must_use]
    pub fn aspect_specific_format(
        &self, aspect: TextureAspect,
    ) -> Option<Self> {
        match (*self, aspect) {
            (Self::Stencil8, TextureAspect::StencilOnly) => Some(*self),
            (
                Self::Depth16Unorm | Self::Depth24Plus | Self::Depth32Float,
                TextureAspect::DepthOnly,
            ) => Some(*self),
            (
                Self::Depth24PlusStencil8 | Self::Depth32FloatStencil8,
                TextureAspect::StencilOnly,
            ) => Some(Self::Stencil8),
            (Self::Depth24PlusStencil8, TextureAspect::DepthOnly) => {
                Some(Self::Depth24Plus)
            }
            (Self::Depth32FloatStencil8, TextureAspect::DepthOnly) => {
                Some(Self::Depth32Float)
            }
            (Self::NV12, TextureAspect::Plane0) => Some(Self::R8Unorm),
            (Self::NV12, TextureAspect::Plane1) => Some(Self::Rg8Unorm),
            (Self::P010, TextureAspect::Plane0) => Some(Self::R16Unorm),
            (Self::P010, TextureAspect::Plane1) => Some(Self::Rg16Unorm),
            // views to multi-planar formats must specify the plane
            (format, TextureAspect::All)
                if !format.is_multi_planar_format() =>
            {
                Some(format)
            }
            _ => None,
        }
    }

    /// Returns `true` if `self` is a depth or stencil component of the given
    /// combined depth-stencil format
    #[must_use]
    pub fn is_depth_stencil_component(&self, combined_format: Self) -> bool {
        match (combined_format, *self) {
            (Self::Depth24PlusStencil8, Self::Depth24Plus | Self::Stencil8)
            | (
                Self::Depth32FloatStencil8,
                Self::Depth32Float | Self::Stencil8,
            ) => true,
            _ => false,
        }
    }

    /// Returns `true` if the format is a depth and/or stencil format
    ///
    #[must_use]
    pub fn is_depth_stencil_format(&self) -> bool {
        match *self {
            Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float
            | Self::Depth32FloatStencil8 => true,
            _ => false,
        }
    }

    /// Returns `true` if the format is a combined depth-stencil format
    ///
    #[must_use]
    pub fn is_combined_depth_stencil_format(&self) -> bool {
        match *self {
            Self::Depth24PlusStencil8 | Self::Depth32FloatStencil8 => true,
            _ => false,
        }
    }

    /// Returns `true` if the format is a multi-planar format
    #[must_use]
    pub fn is_multi_planar_format(&self) -> bool {
        self.planes().is_some()
    }

    /// Returns the number of planes a multi-planar format has.
    #[must_use]
    pub fn planes(&self) -> Option<u32> {
        match *self {
            Self::NV12 => Some(2),
            Self::P010 => Some(2),
            _ => None,
        }
    }

    /// Returns `true` if the format has a color aspect
    #[must_use]
    pub fn has_color_aspect(&self) -> bool {
        !self.is_depth_stencil_format()
    }

    /// Returns `true` if the format has a depth aspect
    #[must_use]
    pub fn has_depth_aspect(&self) -> bool {
        match *self {
            Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float
            | Self::Depth32FloatStencil8 => true,
            _ => false,
        }
    }

    /// Returns `true` if the format has a stencil aspect
    #[must_use]
    pub fn has_stencil_aspect(&self) -> bool {
        match *self {
            Self::Stencil8
            | Self::Depth24PlusStencil8
            | Self::Depth32FloatStencil8 => true,
            _ => false,
        }
    }

    /// Returns the size multiple requirement for a texture using this format.
    #[must_use]
    pub fn size_multiple_requirement(&self) -> (u32, u32) {
        match *self {
            Self::NV12 => (2, 2),
            Self::P010 => (2, 2),
            _ => self.block_dimensions(),
        }
    }

    /// Returns the dimension of a block of texels.
    ///
    /// Uncompressed formats have a block dimension of `(1, 1)`.
    #[must_use]
    pub fn block_dimensions(&self) -> (u32, u32) {
        match *self {
            Self::R8Unorm
            | Self::R8Snorm
            | Self::R8Uint
            | Self::R8Sint
            | Self::R16Uint
            | Self::R16Sint
            | Self::R16Unorm
            | Self::R16Snorm
            | Self::R16Float
            | Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rg8Uint
            | Self::Rg8Sint
            | Self::R32Uint
            | Self::R32Sint
            | Self::R32Float
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rg16Float
            | Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            | Self::Rgb9e5Ufloat
            | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat
            | Self::R64Uint
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm
            | Self::Rgba16Float
            | Self::Rgba32Uint
            | Self::Rgba32Sint
            | Self::Rgba32Float
            | Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float
            | Self::Depth32FloatStencil8
            | Self::NV12
            | Self::P010 => (1, 1),

            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb => (4, 4),

            Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm => (4, 4),

            Self::Astc { block, .. } => match block {
                AstcBlock::B4x4 => (4, 4),
                AstcBlock::B5x4 => (5, 4),
                AstcBlock::B5x5 => (5, 5),
                AstcBlock::B6x5 => (6, 5),
                AstcBlock::B6x6 => (6, 6),
                AstcBlock::B8x5 => (8, 5),
                AstcBlock::B8x6 => (8, 6),
                AstcBlock::B8x8 => (8, 8),
                AstcBlock::B10x5 => (10, 5),
                AstcBlock::B10x6 => (10, 6),
                AstcBlock::B10x8 => (10, 8),
                AstcBlock::B10x10 => (10, 10),
                AstcBlock::B12x10 => (12, 10),
                AstcBlock::B12x12 => (12, 12),
            },
        }
    }

    /// Returns `true` for compressed formats.
    #[must_use]
    pub fn is_compressed(&self) -> bool {
        self.block_dimensions() != (1, 1)
    }

    /// Returns `true` for BCn compressed formats.
    #[must_use]
    pub fn is_bcn(&self) -> bool {
        self.required_features() == Features::TEXTURE_COMPRESSION_BC
    }

    /// Returns `true` for ASTC compressed formats.
    #[must_use]
    pub fn is_astc(&self) -> bool {
        self.required_features() == Features::TEXTURE_COMPRESSION_ASTC
            || self.required_features()
                == Features::TEXTURE_COMPRESSION_ASTC_HDR
    }

    /// Returns the required features (if any) in order to use the texture.
    #[must_use]
    pub fn required_features(&self) -> Features {
        match *self {
            Self::R8Unorm
            | Self::R8Snorm
            | Self::R8Uint
            | Self::R8Sint
            | Self::R16Uint
            | Self::R16Sint
            | Self::R16Float
            | Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rg8Uint
            | Self::Rg8Sint
            | Self::R32Uint
            | Self::R32Sint
            | Self::R32Float
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Float
            | Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            | Self::Rgb9e5Ufloat
            | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Float
            | Self::Rgba32Uint
            | Self::Rgba32Sint
            | Self::Rgba32Float
            | Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float => Features::empty(),

            Self::R64Uint => Features::TEXTURE_INT64_ATOMIC,

            Self::Depth32FloatStencil8 => Features::DEPTH32FLOAT_STENCIL8,

            Self::NV12 => Features::TEXTURE_FORMAT_NV12,
            Self::P010 => Features::TEXTURE_FORMAT_P010,

            Self::R16Unorm
            | Self::R16Snorm
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm => Features::TEXTURE_FORMAT_16BIT_NORM,

            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb => Features::TEXTURE_COMPRESSION_BC,

            Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm => Features::TEXTURE_COMPRESSION_ETC2,

            Self::Astc { channel, .. } => match channel {
                AstcChannel::Hdr => Features::TEXTURE_COMPRESSION_ASTC_HDR,
                AstcChannel::Unorm | AstcChannel::UnormSrgb => {
                    Features::TEXTURE_COMPRESSION_ASTC
                }
            },
        }
    }

    /// Returns the format features guaranteed .
    ///
    /// Additional features are available if `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` is enabled.
    #[must_use]
    pub fn guaranteed_format_features(
        &self, device_features: Features,
    ) -> TextureFormatFeatures {
        // Multisampling
        let none = TextureFormatFeatureFlags::empty();
        let msaa = TextureFormatFeatureFlags::MULTISAMPLE_X4;
        let msaa_resolve =
            msaa | TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE;

        let s_ro_wo = TextureFormatFeatureFlags::STORAGE_READ_ONLY
            | TextureFormatFeatureFlags::STORAGE_WRITE_ONLY;
        let s_all = s_ro_wo | TextureFormatFeatureFlags::STORAGE_READ_WRITE;

        // Flags
        let basic = TextureUsages::COPY_SRC
            | TextureUsages::COPY_DST
            | TextureUsages::TEXTURE_BINDING;
        let attachment =
            basic | TextureUsages::RENDER_ATTACHMENT | TextureUsages::TRANSIENT;
        let storage = basic | TextureUsages::STORAGE_BINDING;
        let binding = TextureUsages::TEXTURE_BINDING;
        let all_flags = attachment | storage | binding;
        let atomic_64 = if device_features.contains(Features::TEXTURE_ATOMIC) {
            storage | binding | TextureUsages::STORAGE_ATOMIC
        } else {
            storage | binding
        };
        let atomic = attachment | atomic_64;
        let (rg11b10f_f, rg11b10f_u) =
            if device_features.contains(Features::RG11B10UFLOAT_RENDERABLE) {
                (msaa_resolve, attachment)
            } else {
                (msaa, basic)
            };
        let (bgra8unorm_f, bgra8unorm) = if device_features
            .contains(Features::BGRA8UNORM_STORAGE)
        {
            (
                msaa_resolve | TextureFormatFeatureFlags::STORAGE_WRITE_ONLY,
                attachment | TextureUsages::STORAGE_BINDING,
            )
        } else {
            (msaa_resolve, attachment)
        };

        #[rustfmt::skip] // lets make a nice table
        let (
            mut flags,
            allowed_usages,
        ) = match *self {
            Self::R8Unorm =>              (msaa_resolve, attachment),
            Self::R8Snorm =>              (        none,      basic),
            Self::R8Uint =>               (        msaa, attachment),
            Self::R8Sint =>               (        msaa, attachment),
            Self::R16Uint =>              (        msaa, attachment),
            Self::R16Sint =>              (        msaa, attachment),
            Self::R16Float =>             (msaa_resolve, attachment),
            Self::Rg8Unorm =>             (msaa_resolve, attachment),
            Self::Rg8Snorm =>             (        none,      basic),
            Self::Rg8Uint =>              (        msaa, attachment),
            Self::Rg8Sint =>              (        msaa, attachment),
            Self::R32Uint =>              (       s_all,     atomic),
            Self::R32Sint =>              (       s_all,     atomic),
            Self::R32Float =>             (msaa | s_all,  all_flags),
            Self::Rg16Uint =>             (        msaa, attachment),
            Self::Rg16Sint =>             (        msaa, attachment),
            Self::Rg16Float =>            (msaa_resolve, attachment),
            Self::Rgba8Unorm =>           (msaa_resolve | s_ro_wo,  all_flags),
            Self::Rgba8UnormSrgb =>       (msaa_resolve, attachment),
            Self::Rgba8Snorm =>           (     s_ro_wo,    storage),
            Self::Rgba8Uint =>            (        msaa | s_ro_wo,  all_flags),
            Self::Rgba8Sint =>            (        msaa | s_ro_wo,  all_flags),
            Self::Bgra8Unorm =>           (bgra8unorm_f, bgra8unorm),
            Self::Bgra8UnormSrgb =>       (msaa_resolve, attachment),
            Self::Rgb10a2Uint =>          (        msaa, attachment),
            Self::Rgb10a2Unorm =>         (msaa_resolve, attachment),
            Self::Rg11b10Ufloat =>        (  rg11b10f_f, rg11b10f_u),
            Self::R64Uint =>              (     s_ro_wo,  atomic_64),
            Self::Rg32Uint =>             (     s_ro_wo,  all_flags),
            Self::Rg32Sint =>             (     s_ro_wo,  all_flags),
            Self::Rg32Float =>            (     s_ro_wo,  all_flags),
            Self::Rgba16Uint =>           (        msaa | s_ro_wo,  all_flags),
            Self::Rgba16Sint =>           (        msaa | s_ro_wo,  all_flags),
            Self::Rgba16Float =>          (msaa_resolve | s_ro_wo,  all_flags),
            Self::Rgba32Uint =>           (     s_ro_wo,  all_flags),
            Self::Rgba32Sint =>           (     s_ro_wo,  all_flags),
            Self::Rgba32Float =>          (     s_ro_wo,  all_flags),

            Self::Stencil8 =>             (        msaa, attachment),
            Self::Depth16Unorm =>         (        msaa, attachment),
            Self::Depth24Plus =>          (        msaa, attachment),
            Self::Depth24PlusStencil8 =>  (        msaa, attachment),
            Self::Depth32Float =>         (        msaa, attachment),
            Self::Depth32FloatStencil8 => (        msaa, attachment),

            // We only support sampling nv12 and p010 textures until we
            // implement transfer plane data.
            Self::NV12 =>                 (        none,    binding),
            Self::P010 =>                 (        none,    binding),

            Self::R16Unorm =>             (        msaa | s_ro_wo,    storage),
            Self::R16Snorm =>             (        msaa | s_ro_wo,    storage),
            Self::Rg16Unorm =>            (        msaa | s_ro_wo,    storage),
            Self::Rg16Snorm =>            (        msaa | s_ro_wo,    storage),
            Self::Rgba16Unorm =>          (        msaa | s_ro_wo,    storage),
            Self::Rgba16Snorm =>          (        msaa | s_ro_wo,    storage),

            Self::Rgb9e5Ufloat =>         (        none,      basic),

            Self::Bc1RgbaUnorm =>         (        none,      basic),
            Self::Bc1RgbaUnormSrgb =>     (        none,      basic),
            Self::Bc2RgbaUnorm =>         (        none,      basic),
            Self::Bc2RgbaUnormSrgb =>     (        none,      basic),
            Self::Bc3RgbaUnorm =>         (        none,      basic),
            Self::Bc3RgbaUnormSrgb =>     (        none,      basic),
            Self::Bc4RUnorm =>            (        none,      basic),
            Self::Bc4RSnorm =>            (        none,      basic),
            Self::Bc5RgUnorm =>           (        none,      basic),
            Self::Bc5RgSnorm =>           (        none,      basic),
            Self::Bc6hRgbUfloat =>        (        none,      basic),
            Self::Bc6hRgbFloat =>         (        none,      basic),
            Self::Bc7RgbaUnorm =>         (        none,      basic),
            Self::Bc7RgbaUnormSrgb =>     (        none,      basic),

            Self::Etc2Rgb8Unorm =>        (        none,      basic),
            Self::Etc2Rgb8UnormSrgb =>    (        none,      basic),
            Self::Etc2Rgb8A1Unorm =>      (        none,      basic),
            Self::Etc2Rgb8A1UnormSrgb =>  (        none,      basic),
            Self::Etc2Rgba8Unorm =>       (        none,      basic),
            Self::Etc2Rgba8UnormSrgb =>   (        none,      basic),
            Self::EacR11Unorm =>          (        none,      basic),
            Self::EacR11Snorm =>          (        none,      basic),
            Self::EacRg11Unorm =>         (        none,      basic),
            Self::EacRg11Snorm =>         (        none,      basic),

            Self::Astc { .. } =>          (        none,      basic),
        };

        // Get whether the format is filterable, taking features into account
        let sample_type1 = self.sample_type(None, Some(device_features));
        let is_filterable =
            sample_type1 == Some(TextureSampleType::Float { filterable: true });

        // Features that enable filtering don't affect blendability
        let sample_type2 = self.sample_type(None, None);
        let is_blendable =
            sample_type2 == Some(TextureSampleType::Float { filterable: true });

        flags.set(TextureFormatFeatureFlags::FILTERABLE, is_filterable);
        flags.set(TextureFormatFeatureFlags::BLENDABLE, is_blendable);
        flags.set(
            TextureFormatFeatureFlags::STORAGE_ATOMIC,
            allowed_usages.contains(TextureUsages::STORAGE_ATOMIC),
        );

        TextureFormatFeatures { allowed_usages, flags }
    }

    /// Returns the sample type compatible with this format and aspect.
    ///
    /// Returns `None` only if this is a combined depth-stencil format or a multi-planar format
    /// and `TextureAspect::All` or no `aspect` was provided.
    #[must_use]
    pub fn sample_type(
        &self, aspect: Option<TextureAspect>, device_features: Option<Features>,
    ) -> Option<TextureSampleType> {
        let float = TextureSampleType::Float { filterable: true };
        let unfilterable_float = TextureSampleType::Float { filterable: false };
        let float32_sample_type = TextureSampleType::Float {
            filterable: device_features
                .unwrap_or(Features::empty())
                .contains(Features::FLOAT32_FILTERABLE),
        };
        let depth = TextureSampleType::Depth;
        let uint = TextureSampleType::Uint;
        let sint = TextureSampleType::Sint;

        match *self {
            Self::R8Unorm
            | Self::R8Snorm
            | Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            | Self::R16Float
            | Self::Rg16Float
            | Self::Rgba16Float
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat => Some(float),

            Self::R32Float | Self::Rg32Float | Self::Rgba32Float => {
                Some(float32_sample_type)
            }

            Self::R8Uint
            | Self::Rg8Uint
            | Self::Rgba8Uint
            | Self::R16Uint
            | Self::Rg16Uint
            | Self::Rgba16Uint
            | Self::R32Uint
            | Self::R64Uint
            | Self::Rg32Uint
            | Self::Rgba32Uint
            | Self::Rgb10a2Uint => Some(uint),

            Self::R8Sint
            | Self::Rg8Sint
            | Self::Rgba8Sint
            | Self::R16Sint
            | Self::Rg16Sint
            | Self::Rgba16Sint
            | Self::R32Sint
            | Self::Rg32Sint
            | Self::Rgba32Sint => Some(sint),

            Self::Stencil8 => Some(uint),
            Self::Depth16Unorm | Self::Depth24Plus | Self::Depth32Float => {
                Some(depth)
            }
            Self::Depth24PlusStencil8 | Self::Depth32FloatStencil8 => {
                match aspect {
                    Some(TextureAspect::DepthOnly) => Some(depth),
                    Some(TextureAspect::StencilOnly) => Some(uint),
                    _ => None,
                }
            }

            Self::NV12 | Self::P010 => match aspect {
                Some(TextureAspect::Plane0) | Some(TextureAspect::Plane1) => {
                    Some(unfilterable_float)
                }
                _ => None,
            },

            Self::R16Unorm
            | Self::R16Snorm
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm => Some(float),

            Self::Rgb9e5Ufloat => Some(float),

            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb => Some(float),

            Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm => Some(float),

            Self::Astc { .. } => Some(float),
        }
    }

    #[must_use]
    pub fn block_copy_size(
        &self, aspect: Option<TextureAspect>,
    ) -> Option<u32> {
        match *self {
            Self::R8Unorm | Self::R8Snorm | Self::R8Uint | Self::R8Sint => {
                Some(1)
            }

            Self::Rg8Unorm | Self::Rg8Snorm | Self::Rg8Uint | Self::Rg8Sint => {
                Some(2)
            }
            Self::R16Unorm
            | Self::R16Snorm
            | Self::R16Uint
            | Self::R16Sint
            | Self::R16Float => Some(2),

            Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb => Some(4),
            Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Float => Some(4),
            Self::R32Uint | Self::R32Sint | Self::R32Float => Some(4),
            Self::Rgb9e5Ufloat
            | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat => Some(4),

            Self::Rgba16Unorm
            | Self::Rgba16Snorm
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Float => Some(8),
            Self::R64Uint
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float => Some(8),

            Self::Rgba32Uint | Self::Rgba32Sint | Self::Rgba32Float => Some(16),

            Self::Stencil8 => Some(1),
            Self::Depth16Unorm => Some(2),
            Self::Depth32Float => Some(4),
            Self::Depth24Plus => None,
            Self::Depth24PlusStencil8 => match aspect {
                Some(TextureAspect::DepthOnly) => None,
                Some(TextureAspect::StencilOnly) => Some(1),
                _ => None,
            },
            Self::Depth32FloatStencil8 => match aspect {
                Some(TextureAspect::DepthOnly) => Some(4),
                Some(TextureAspect::StencilOnly) => Some(1),
                _ => None,
            },

            Self::NV12 => match aspect {
                Some(TextureAspect::Plane0) => Some(1),
                Some(TextureAspect::Plane1) => Some(2),
                _ => None,
            },

            Self::P010 => match aspect {
                Some(TextureAspect::Plane0) => Some(2),
                Some(TextureAspect::Plane1) => Some(4),
                _ => None,
            },

            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm => Some(8),
            Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb => Some(16),

            Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm => Some(8),
            Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm => Some(16),

            Self::Astc { .. } => Some(16),
        }
    }

    /// The largest number that can be returned by [`Self::target_pixel_byte_cost`].
    pub const MAX_TARGET_PIXEL_BYTE_COST: u32 = 16;

    /// The number of bytes occupied per pixel in a color attachment
    #[must_use]
    pub fn target_pixel_byte_cost(&self) -> Option<u32> {
        match *self {
            Self::R8Unorm | Self::R8Snorm | Self::R8Uint | Self::R8Sint => Some(1),
            Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rg8Uint
            | Self::Rg8Sint
            | Self::R16Uint
            | Self::R16Sint
            | Self::R16Unorm
            | Self::R16Snorm
            | Self::R16Float => Some(2),
            Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rg16Float
            | Self::R32Uint
            | Self::R32Sint
            | Self::R32Float => Some(4),
            // Despite being 4 bytes per pixel, these are 8 bytes per pixel in the table
            Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            // ---
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm
            | Self::Rgba16Float
            | Self::R64Uint
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float
            | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat => Some(8),
            Self::Rgba32Uint | Self::Rgba32Sint | Self::Rgba32Float => Some(16),
            // ⚠️ If you add formats with larger sizes, make sure you change `MAX_TARGET_PIXEL_BYTE_COST`` ⚠️
            Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float
            | Self::Depth32FloatStencil8
            | Self::NV12
            | Self::P010
            | Self::Rgb9e5Ufloat
            | Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb
            | Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm
            | Self::Astc { .. } => None,
        }
    }

    #[must_use]
    pub fn target_component_alignment(&self) -> Option<u32> {
        match *self {
            Self::R8Unorm
            | Self::R8Snorm
            | Self::R8Uint
            | Self::R8Sint
            | Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rg8Uint
            | Self::Rg8Sint
            | Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb => Some(1),
            Self::R16Uint
            | Self::R16Sint
            | Self::R16Unorm
            | Self::R16Snorm
            | Self::R16Float
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rg16Float
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm
            | Self::Rgba16Float => Some(2),
            Self::R32Uint
            | Self::R32Sint
            | Self::R32Float
            | Self::R64Uint
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float
            | Self::Rgba32Uint
            | Self::Rgba32Sint
            | Self::Rgba32Float
            | Self::Rgb10a2Uint
            | Self::Rgb10a2Unorm
            | Self::Rg11b10Ufloat => Some(4),
            Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth24PlusStencil8
            | Self::Depth32Float
            | Self::Depth32FloatStencil8
            | Self::NV12
            | Self::P010
            | Self::Rgb9e5Ufloat
            | Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc4RUnorm
            | Self::Bc4RSnorm
            | Self::Bc5RgUnorm
            | Self::Bc5RgSnorm
            | Self::Bc6hRgbUfloat
            | Self::Bc6hRgbFloat
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb
            | Self::Etc2Rgb8Unorm
            | Self::Etc2Rgb8UnormSrgb
            | Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb
            | Self::EacR11Unorm
            | Self::EacR11Snorm
            | Self::EacRg11Unorm
            | Self::EacRg11Snorm
            | Self::Astc { .. } => None,
        }
    }

    /// Returns the number of components this format has.
    #[must_use]
    pub fn components(&self) -> u8 {
        self.components_with_aspect(TextureAspect::All)
    }

    /// Returns the number of components this format has taking into account the `aspect`.
    ///
    /// The `aspect` is only relevant for combined depth-stencil formats and multi-planar formats.
    #[must_use]
    pub fn components_with_aspect(&self, aspect: TextureAspect) -> u8 {
        match *self {
            Self::R8Unorm
            | Self::R8Snorm
            | Self::R8Uint
            | Self::R8Sint
            | Self::R16Unorm
            | Self::R16Snorm
            | Self::R16Uint
            | Self::R16Sint
            | Self::R16Float
            | Self::R32Uint
            | Self::R32Sint
            | Self::R32Float
            | Self::R64Uint => 1,

            Self::Rg8Unorm
            | Self::Rg8Snorm
            | Self::Rg8Uint
            | Self::Rg8Sint
            | Self::Rg16Unorm
            | Self::Rg16Snorm
            | Self::Rg16Uint
            | Self::Rg16Sint
            | Self::Rg16Float
            | Self::Rg32Uint
            | Self::Rg32Sint
            | Self::Rg32Float => 2,

            Self::Rgba8Unorm
            | Self::Rgba8UnormSrgb
            | Self::Rgba8Snorm
            | Self::Rgba8Uint
            | Self::Rgba8Sint
            | Self::Bgra8Unorm
            | Self::Bgra8UnormSrgb
            | Self::Rgba16Unorm
            | Self::Rgba16Snorm
            | Self::Rgba16Uint
            | Self::Rgba16Sint
            | Self::Rgba16Float
            | Self::Rgba32Uint
            | Self::Rgba32Sint
            | Self::Rgba32Float => 4,

            Self::Rgb9e5Ufloat | Self::Rg11b10Ufloat => 3,
            Self::Rgb10a2Uint | Self::Rgb10a2Unorm => 4,

            Self::Stencil8
            | Self::Depth16Unorm
            | Self::Depth24Plus
            | Self::Depth32Float => 1,

            Self::Depth24PlusStencil8 | Self::Depth32FloatStencil8 => {
                match aspect {
                    TextureAspect::DepthOnly | TextureAspect::StencilOnly => 1,
                    _ => 2,
                }
            }

            Self::NV12 | Self::P010 => match aspect {
                TextureAspect::Plane0 => 1,
                TextureAspect::Plane1 => 2,
                _ => 3,
            },

            Self::Bc4RUnorm | Self::Bc4RSnorm => 1,
            Self::Bc5RgUnorm | Self::Bc5RgSnorm => 2,
            Self::Bc6hRgbUfloat | Self::Bc6hRgbFloat => 3,
            Self::Bc1RgbaUnorm
            | Self::Bc1RgbaUnormSrgb
            | Self::Bc2RgbaUnorm
            | Self::Bc2RgbaUnormSrgb
            | Self::Bc3RgbaUnorm
            | Self::Bc3RgbaUnormSrgb
            | Self::Bc7RgbaUnorm
            | Self::Bc7RgbaUnormSrgb => 4,

            Self::EacR11Unorm | Self::EacR11Snorm => 1,
            Self::EacRg11Unorm | Self::EacRg11Snorm => 2,
            Self::Etc2Rgb8Unorm | Self::Etc2Rgb8UnormSrgb => 3,
            Self::Etc2Rgb8A1Unorm
            | Self::Etc2Rgb8A1UnormSrgb
            | Self::Etc2Rgba8Unorm
            | Self::Etc2Rgba8UnormSrgb => 4,

            Self::Astc { .. } => 4,
        }
    }

    /// Strips the `Srgb` suffix from the given texture format.
    #[must_use]
    pub fn remove_srgb_suffix(&self) -> TextureFormat {
        match *self {
            Self::Rgba8UnormSrgb => Self::Rgba8Unorm,
            Self::Bgra8UnormSrgb => Self::Bgra8Unorm,
            Self::Bc1RgbaUnormSrgb => Self::Bc1RgbaUnorm,
            Self::Bc2RgbaUnormSrgb => Self::Bc2RgbaUnorm,
            Self::Bc3RgbaUnormSrgb => Self::Bc3RgbaUnorm,
            Self::Bc7RgbaUnormSrgb => Self::Bc7RgbaUnorm,
            Self::Etc2Rgb8UnormSrgb => Self::Etc2Rgb8Unorm,
            Self::Etc2Rgb8A1UnormSrgb => Self::Etc2Rgb8A1Unorm,
            Self::Etc2Rgba8UnormSrgb => Self::Etc2Rgba8Unorm,
            Self::Astc { block, channel: AstcChannel::UnormSrgb } => {
                Self::Astc { block, channel: AstcChannel::Unorm }
            }
            _ => *self,
        }
    }

    /// Adds an `Srgb` suffix to the given texture format, if the format supports it.
    #[must_use]
    pub fn add_srgb_suffix(&self) -> TextureFormat {
        match *self {
            Self::Rgba8Unorm => Self::Rgba8UnormSrgb,
            Self::Bgra8Unorm => Self::Bgra8UnormSrgb,
            Self::Bc1RgbaUnorm => Self::Bc1RgbaUnormSrgb,
            Self::Bc2RgbaUnorm => Self::Bc2RgbaUnormSrgb,
            Self::Bc3RgbaUnorm => Self::Bc3RgbaUnormSrgb,
            Self::Bc7RgbaUnorm => Self::Bc7RgbaUnormSrgb,
            Self::Etc2Rgb8Unorm => Self::Etc2Rgb8UnormSrgb,
            Self::Etc2Rgb8A1Unorm => Self::Etc2Rgb8A1UnormSrgb,
            Self::Etc2Rgba8Unorm => Self::Etc2Rgba8UnormSrgb,
            Self::Astc { block, channel: AstcChannel::Unorm } => {
                Self::Astc { block, channel: AstcChannel::UnormSrgb }
            }
            _ => *self,
        }
    }

    /// Returns `true` for srgb formats.
    #[must_use]
    pub fn is_srgb(&self) -> bool {
        *self != self.remove_srgb_suffix()
    }

    /// Returns the theoretical memory footprint of a texture with the given format and dimensions.
    ///
    /// Actual memory usage may greatly exceed this value due to alignment and padding.
    #[must_use]
    pub fn theoretical_memory_footprint(&self, size: Extent3d) -> u64 {
        let (block_width, block_height) = self.block_dimensions();

        let block_size = self.block_copy_size(None);

        let approximate_block_size = match block_size {
            Some(size) => size,
            None => match self {
                // One f16 per pixel
                Self::Depth16Unorm => 2,
                // One u24 per pixel, padded to 4 bytes
                Self::Depth24Plus => 4,
                // One u24 per pixel, plus one u8 per pixel
                Self::Depth24PlusStencil8 => 4,
                // One f32 per pixel
                Self::Depth32Float => 4,
                // One f32 per pixel, plus one u8 per pixel, with 3 bytes intermediary padding
                Self::Depth32FloatStencil8 => 8,
                // One u8 per pixel
                Self::Stencil8 => 1,
                // Two chroma bytes per block, one luma byte per block
                Self::NV12 => 3,
                // Two chroma u16s and one luma u16 per block
                Self::P010 => 6,
                f => {
                    tracing::warn!(
                        "Memory footprint for format {f:?} is not implemented"
                    );
                    0
                }
            },
        };

        let width_blocks = size.width.div_ceil(block_width) as u64;
        let height_blocks = size.height.div_ceil(block_height) as u64;

        let total_blocks =
            width_blocks * height_blocks * size.depth_or_array_layers as u64;

        total_blocks * approximate_block_size as u64
    }
}

impl TextureAspect {
    /// Returns the texture aspect for a given plane.
    #[must_use]
    pub fn from_plane(plane: u32) -> Option<Self> {
        Some(match plane {
            0 => Self::Plane0,
            1 => Self::Plane1,
            2 => Self::Plane2,
            _ => return None,
        })
    }

    /// Returns the plane for a given texture aspect.
    #[must_use]
    pub fn to_plane(&self) -> Option<u32> {
        match self {
            TextureAspect::Plane0 => Some(0),
            TextureAspect::Plane1 => Some(1),
            TextureAspect::Plane2 => Some(2),
            _ => None,
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ColorWrites(u32);

bitflags::bitflags! {
    impl ColorWrites: u32 {
        /// Enable red channel writes
        const RED = 1 << 0;
        /// Enable green channel writes
        const GREEN = 1 << 1;
        /// Enable blue channel writes
        const BLUE = 1 << 2;
        /// Enable alpha channel writes
        const ALPHA = 1 << 3;
        /// Enable red, green, and blue channel writes
        const COLOR = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits();
        /// Enable writes to all channels.
        const ALL = Self::RED.bits() | Self::GREEN.bits() | Self::BLUE.bits() | Self::ALPHA.bits();
    }
}

impl Default for ColorWrites {
    fn default() -> Self {
        Self::ALL
    }
}

#[derive(Clone, Debug)]
pub enum PollType<T> {
    Wait {
        /// Submission index to wait for.
        ///
        /// If not specified, will wait for the most recent submission at the time of the poll.
        /// By the time the method returns, more submissions may have taken place.
        submission_index: Option<T>,

        /// Max time to wait for the submission to complete.
        ///
        /// If not specified, will wait indefinitely (or until an error is detected).
        /// If waiting for the GPU device takes this long or longer, the poll will return [`PollError::Timeout`].
        timeout: Option<Duration>,
    },

    /// Check the device for a single time without blocking.
    Poll,
}

impl<T> PollType<T> {
    /// Wait indefinitely until for the most recent submission to complete.
    ///
    /// This is a convenience function that creates a [`Self::Wait`] variant with
    /// no timeout and no submission index.
    #[must_use]
    pub const fn wait_indefinitely() -> Self {
        Self::Wait { submission_index: None, timeout: None }
    }

    /// This `PollType` represents a wait of some kind.
    #[must_use]
    pub fn is_wait(&self) -> bool {
        match *self {
            Self::Wait { .. } => true,
            Self::Poll => false,
        }
    }

    /// Map on the wait index type.
    #[must_use]
    pub fn map_index<U, F>(self, func: F) -> PollType<U>
    where
        F: FnOnce(T) -> U, {
        match self {
            Self::Wait { submission_index, timeout } => PollType::Wait {
                submission_index: submission_index.map(func),
                timeout,
            },
            Self::Poll => PollType::Poll,
        }
    }
}

/// Error states after a device poll.
#[derive(Debug)]
pub enum PollError {
    /// The requested Wait timed out before the submission was completed.
    Timeout,
    /// The requested Wait was given a wrong submission index.
    WrongSubmissionIndex(u64, u64),
}

impl fmt::Display for PollError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PollError::Timeout => {
                f.write_str("The requested Wait timed out before the submission was completed.")
            }
            PollError::WrongSubmissionIndex(requested, successful) => write!(
                f,
                "Tried to wait using a submission index ({requested}) \
                that has not been returned by a successful submission \
                (last successful submission: {successful}"
            ),
        }
    }
}

impl core::error::Error for PollError {}

/// Status of device poll operation.
#[derive(Debug, PartialEq, Eq)]
pub enum PollStatus {
    /// There are no active submissions in flight as of the beginning of the poll call.
    /// Other submissions may have been queued on other threads during the call.
    ///
    /// This implies that the given Wait was satisfied before the timeout.
    QueueEmpty,

    /// The requested Wait was satisfied before the timeout.
    WaitSucceeded,

    /// This was a poll.
    Poll,
}

impl PollStatus {
    /// Returns true if the result is [`Self::QueueEmpty`].
    #[must_use]
    pub fn is_queue_empty(&self) -> bool {
        matches!(self, Self::QueueEmpty)
    }

    /// Returns true if the result is either [`Self::WaitSucceeded`] or [`Self::QueueEmpty`].
    #[must_use]
    pub fn wait_finished(&self) -> bool {
        matches!(self, Self::WaitSucceeded | Self::QueueEmpty)
    }
}

#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StencilState {
    /// Front face mode.
    pub front: StencilFaceState,
    /// Back face mode.
    pub back: StencilFaceState,
    /// Stencil values are AND'd with this mask when reading and writing from the stencil buffer. Only low 8 bits are used.
    pub read_mask: u32,
    /// Stencil values are AND'd with this mask when writing to the stencil buffer. Only low 8 bits are used.
    pub write_mask: u32,
}

impl StencilState {
    /// Returns true if the stencil test is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        (self.front != StencilFaceState::IGNORE
            || self.back != StencilFaceState::IGNORE)
            && (self.read_mask != 0 || self.write_mask != 0)
    }
    /// Returns true if the state doesn't mutate the target values.
    #[must_use]
    pub fn is_read_only(&self, cull_mode: Option<Face>) -> bool {
        // The rules are defined in step 7 of the "Device timeline initialization steps"
        // subsection of the "Render Pipeline Creation" section

        if self.write_mask == 0 {
            return true;
        }

        let front_ro =
            cull_mode == Some(Face::Front) || self.front.is_read_only();
        let back_ro = cull_mode == Some(Face::Back) || self.back.is_read_only();

        front_ro && back_ro
    }
    /// Returns true if the stencil state uses the reference value for testing.
    #[must_use]
    pub fn needs_ref_value(&self) -> bool {
        self.front.needs_ref_value() || self.back.needs_ref_value()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DepthBiasState {
    /// Constant depth biasing factor, in basic units of the depth format.
    pub constant: i32,
    /// Slope depth biasing factor.
    pub slope_scale: f32,
    /// Depth bias clamp value (absolute).
    pub clamp: f32,
}

impl DepthBiasState {
    /// Returns true if the depth biasing is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.constant != 0 || self.slope_scale != 0.0
    }
}

impl Hash for DepthBiasState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.constant.hash(state);
        self.slope_scale.to_bits().hash(state);
        self.clamp.to_bits().hash(state);
    }
}

impl PartialEq for DepthBiasState {
    fn eq(&self, other: &Self) -> bool {
        (self.constant == other.constant)
            && (self.slope_scale.to_bits() == other.slope_scale.to_bits())
            && (self.clamp.to_bits() == other.clamp.to_bits())
    }
}

impl Eq for DepthBiasState {}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum LoadOp<V> {
    /// Loads the specified value for this attachment into the render pass.
    ///
    /// On some GPU hardware (primarily mobile), "clear" is significantly cheaper
    /// because it avoids loading data from main memory into tile-local memory.
    ///
    /// On other GPU hardware, there isn’t a significant difference.
    ///
    /// As a result, it is recommended to use "clear" rather than "load" in cases
    /// where the initial value doesn’t matter
    /// (e.g. the render target will be cleared using a skybox).
    Clear(V) = 0,
    /// Loads the existing value for this attachment into the render pass.
    Load = 1,
}

impl<V> LoadOp<V> {
    /// Returns true if variants are same (ignoring clear value)
    pub fn eq_variant<T>(&self, other: LoadOp<T>) -> bool {
        matches!(
            (self, other),
            (LoadOp::Clear(_), LoadOp::Clear(_)) | (LoadOp::Load, LoadOp::Load)
        )
    }
}

impl<V: Default> Default for LoadOp<V> {
    fn default() -> Self {
        Self::Clear(Default::default())
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Default)]
pub enum StoreOp {
    /// Stores the resulting value of the render pass for this attachment.
    #[default]
    Store = 0,
    /// Discards the resulting value of the render pass for this attachment.
    ///
    /// The attachment will be treated as uninitialized afterwards.
    /// (If only either Depth or Stencil texture-aspects is set to `Discard`,
    /// the respective other texture-aspect will be preserved.)
    ///
    /// This can be significantly faster on tile-based render hardware.
    ///
    /// Prefer this if the attachment is not read by subsequent passes.
    Discard = 1,
}

#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub struct Operations<V> {
    /// How data should be read through this attachment.
    pub load: LoadOp<V>,
    /// Whether data will be written to through this attachment.
    ///
    /// Note that resolve textures (if specified) are always written to,
    /// regardless of this setting.
    pub store: StoreOp,
}

impl<V: Default> Default for Operations<V> {
    #[inline]
    fn default() -> Self {
        Self { load: LoadOp::<V>::default(), store: StoreOp::default() }
    }
}

#[repr(C)]
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct DepthStencilState {
    /// Format of the depth/stencil buffer, must be special depth format. Must match the format
    /// of the depth/stencil attachment in [`CommandEncoder::begin_render_pass`][CEbrp].
    ///
    pub format: TextureFormat,
    /// If disabled, depth will not be written to.
    pub depth_write_enabled: bool,
    /// Comparison function used to compare depth values in the depth test.
    pub depth_compare: CompareFunction,
    /// Stencil state.
    pub stencil: StencilState,
    /// Depth bias state.
    pub bias: DepthBiasState,
}

impl DepthStencilState {
    /// Returns true if the depth testing is enabled.
    #[must_use]
    pub fn is_depth_enabled(&self) -> bool {
        self.depth_compare != CompareFunction::Always
            || self.depth_write_enabled
    }

    /// Returns true if the state doesn't mutate the depth buffer.
    #[must_use]
    pub fn is_depth_read_only(&self) -> bool {
        !self.depth_write_enabled
    }

    /// Returns true if the state doesn't mutate the stencil.
    #[must_use]
    pub fn is_stencil_read_only(&self, cull_mode: Option<Face>) -> bool {
        self.stencil.is_read_only(cull_mode)
    }

    /// Returns true if the state doesn't mutate either depth or stencil of the target.
    #[must_use]
    pub fn is_read_only(&self, cull_mode: Option<Face>) -> bool {
        self.is_depth_read_only() && self.is_stencil_read_only(cull_mode)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum IndexFormat {
    /// Indices are 16 bit unsigned integers.
    Uint16 = 0,
    /// Indices are 32 bit unsigned integers.
    #[default]
    Uint32 = 1,
}

impl IndexFormat {
    /// Returns the size in bytes of the index format
    pub fn byte_size(&self) -> usize {
        match self {
            IndexFormat::Uint16 => 2,
            IndexFormat::Uint32 => 4,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum StencilOperation {
    /// Keep stencil value unchanged.
    #[default]
    Keep = 0,
    /// Set stencil value to zero.
    Zero = 1,
    /// Replace stencil value with value provided in most recent call to
    /// [`RenderPass::set_stencil_reference`][RPssr].
    ///
    Replace = 2,
    /// Bitwise inverts stencil value.
    Invert = 3,
    /// Increments stencil value by one, clamping on overflow.
    IncrementClamp = 4,
    /// Decrements stencil value by one, clamping on underflow.
    DecrementClamp = 5,
    /// Increments stencil value by one, wrapping on overflow.
    IncrementWrap = 6,
    /// Decrements stencil value by one, wrapping on underflow.
    DecrementWrap = 7,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StencilFaceState {
    /// Comparison function that determines if the fail_op or pass_op is used on the stencil buffer.
    pub compare: CompareFunction,
    /// Operation that is performed when stencil test fails.
    pub fail_op: StencilOperation,
    /// Operation that is performed when depth test fails but stencil test succeeds.
    pub depth_fail_op: StencilOperation,
    /// Operation that is performed when stencil test success.
    pub pass_op: StencilOperation,
}

impl StencilFaceState {
    /// Ignore the stencil state for the face.
    pub const IGNORE: Self = StencilFaceState {
        compare: CompareFunction::Always,
        fail_op: StencilOperation::Keep,
        depth_fail_op: StencilOperation::Keep,
        pass_op: StencilOperation::Keep,
    };

    /// Returns true if the face state uses the reference value for testing or operation.
    #[must_use]
    pub fn needs_ref_value(&self) -> bool {
        self.compare.needs_ref_value()
            || self.fail_op == StencilOperation::Replace
            || self.depth_fail_op == StencilOperation::Replace
            || self.pass_op == StencilOperation::Replace
    }

    /// Returns true if the face state doesn't mutate the target values.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.pass_op == StencilOperation::Keep
            && self.depth_fail_op == StencilOperation::Keep
            && self.fail_op == StencilOperation::Keep
    }
}

impl Default for StencilFaceState {
    fn default() -> Self {
        Self::IGNORE
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum CompareFunction {
    /// Function never passes
    Never = 1,
    /// Function passes if new value less than existing value
    Less = 2,
    /// Function passes if new value is equal to existing value. When using
    /// this compare function, make sure to mark your Vertex Shader's `@builtin(position)`
    /// output as `@invariant` to prevent artifacting.
    Equal = 3,
    /// Function passes if new value is less than or equal to existing value
    LessEqual = 4,
    /// Function passes if new value is greater than existing value
    Greater = 5,
    /// Function passes if new value is not equal to existing value. When using
    /// this compare function, make sure to mark your Vertex Shader's `@builtin(position)`
    /// output as `@invariant` to prevent artifacting.
    NotEqual = 6,
    /// Function passes if new value is greater than or equal to existing value
    GreaterEqual = 7,
    /// Function always passes
    Always = 8,
}

impl CompareFunction {
    /// Returns true if the comparison depends on the reference value.
    #[must_use]
    pub fn needs_ref_value(self) -> bool {
        match self {
            Self::Never | Self::Always => false,
            _ => true,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum VertexStepMode {
    /// Vertex data is advanced every vertex.
    #[default]
    Vertex = 0,
    /// Vertex data is advanced every instance.
    Instance = 1,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VertexAttribute {
    /// Format of the input
    pub format: VertexFormat,
    /// Byte offset of the start of the input
    pub offset: BufferAddress,
    /// Location for this input. Must match the location in the shader.
    pub shader_location: ShaderLocation,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)]
pub enum VertexFormat {
    /// One unsigned byte (u8). `u32` in shaders.
    Uint8 = 0,
    /// Two unsigned bytes (u8). `vec2<u32>` in shaders.
    Uint8x2 = 1,
    /// Four unsigned bytes (u8). `vec4<u32>` in shaders.
    Uint8x4 = 2,
    /// One signed byte (i8). `i32` in shaders.
    Sint8 = 3,
    /// Two signed bytes (i8). `vec2<i32>` in shaders.
    Sint8x2 = 4,
    /// Four signed bytes (i8). `vec4<i32>` in shaders.
    Sint8x4 = 5,
    /// One unsigned byte (u8). [0, 255] converted to float [0, 1] `f32` in shaders.
    Unorm8 = 6,
    /// Two unsigned bytes (u8). [0, 255] converted to float [0, 1] `vec2<f32>` in shaders.
    Unorm8x2 = 7,
    /// Four unsigned bytes (u8). [0, 255] converted to float [0, 1] `vec4<f32>` in shaders.
    Unorm8x4 = 8,
    /// One signed byte (i8). [&minus;127, 127] converted to float [&minus;1, 1] `f32` in shaders.
    Snorm8 = 9,
    /// Two signed bytes (i8). [&minus;127, 127] converted to float [&minus;1, 1] `vec2<f32>` in shaders.
    Snorm8x2 = 10,
    /// Four signed bytes (i8). [&minus;127, 127] converted to float [&minus;1, 1] `vec4<f32>` in shaders.
    Snorm8x4 = 11,
    /// One unsigned short (u16). `u32` in shaders.
    Uint16 = 12,
    /// Two unsigned shorts (u16). `vec2<u32>` in shaders.
    Uint16x2 = 13,
    /// Four unsigned shorts (u16). `vec4<u32>` in shaders.
    Uint16x4 = 14,
    /// One signed short (u16). `i32` in shaders.
    Sint16 = 15,
    /// Two signed shorts (i16). `vec2<i32>` in shaders.
    Sint16x2 = 16,
    /// Four signed shorts (i16). `vec4<i32>` in shaders.
    Sint16x4 = 17,
    /// One unsigned short (u16). [0, 65535] converted to float [0, 1] `f32` in shaders.
    Unorm16 = 18,
    /// Two unsigned shorts (u16). [0, 65535] converted to float [0, 1] `vec2<f32>` in shaders.
    Unorm16x2 = 19,
    /// Four unsigned shorts (u16). [0, 65535] converted to float [0, 1] `vec4<f32>` in shaders.
    Unorm16x4 = 20,
    /// One signed short (i16). [&minus;32767, 32767] converted to float [&minus;1, 1] `f32` in shaders.
    Snorm16 = 21,
    /// Two signed shorts (i16). [&minus;32767, 32767] converted to float [&minus;1, 1] `vec2<f32>` in shaders.
    Snorm16x2 = 22,
    /// Four signed shorts (i16). [&minus;32767, 32767] converted to float [&minus;1, 1] `vec4<f32>` in shaders.
    Snorm16x4 = 23,
    /// One half-precision float (no Rust equiv). `f32` in shaders.
    Float16 = 24,
    /// Two half-precision floats (no Rust equiv). `vec2<f32>` in shaders.
    Float16x2 = 25,
    /// Four half-precision floats (no Rust equiv). `vec4<f32>` in shaders.
    Float16x4 = 26,
    /// One single-precision float (f32). `f32` in shaders.
    Float32 = 27,
    /// Two single-precision floats (f32). `vec2<f32>` in shaders.
    Float32x2 = 28,
    /// Three single-precision floats (f32). `vec3<f32>` in shaders.
    Float32x3 = 29,
    /// Four single-precision floats (f32). `vec4<f32>` in shaders.
    Float32x4 = 30,
    /// One unsigned int (u32). `u32` in shaders.
    Uint32 = 31,
    /// Two unsigned ints (u32). `vec2<u32>` in shaders.
    Uint32x2 = 32,
    /// Three unsigned ints (u32). `vec3<u32>` in shaders.
    Uint32x3 = 33,
    /// Four unsigned ints (u32). `vec4<u32>` in shaders.
    Uint32x4 = 34,
    /// One signed int (i32). `i32` in shaders.
    Sint32 = 35,
    /// Two signed ints (i32). `vec2<i32>` in shaders.
    Sint32x2 = 36,
    /// Three signed ints (i32). `vec3<i32>` in shaders.
    Sint32x3 = 37,
    /// Four signed ints (i32). `vec4<i32>` in shaders.
    Sint32x4 = 38,
    /// One double-precision float (f64). `f32` in shaders. Requires [`Features::VERTEX_ATTRIBUTE_64BIT`].
    Float64 = 39,
    /// Two double-precision floats (f64). `vec2<f32>` in shaders. Requires [`Features::VERTEX_ATTRIBUTE_64BIT`].
    Float64x2 = 40,
    /// Three double-precision floats (f64). `vec3<f32>` in shaders. Requires [`Features::VERTEX_ATTRIBUTE_64BIT`].
    Float64x3 = 41,
    /// Four double-precision floats (f64). `vec4<f32>` in shaders. Requires [`Features::VERTEX_ATTRIBUTE_64BIT`].
    Float64x4 = 42,
    /// Three unsigned 10-bit integers and one 2-bit integer, packed into a 32-bit integer (u32). [0, 1024] converted to float [0, 1] `vec4<f32>` in shaders.
    Unorm10_10_10_2 = 43,
    /// Four unsigned 8-bit integers, packed into a 32-bit integer (u32). [0, 255] converted to float [0, 1] `vec4<f32>` in shaders.
    Unorm8x4Bgra = 44,
}

impl VertexFormat {
    /// Returns the byte size of the format.
    #[must_use]
    pub const fn size(&self) -> u64 {
        match self {
            Self::Uint8 | Self::Sint8 | Self::Unorm8 | Self::Snorm8 => 1,
            Self::Uint8x2
            | Self::Sint8x2
            | Self::Unorm8x2
            | Self::Snorm8x2
            | Self::Uint16
            | Self::Sint16
            | Self::Unorm16
            | Self::Snorm16
            | Self::Float16 => 2,
            Self::Uint8x4
            | Self::Sint8x4
            | Self::Unorm8x4
            | Self::Snorm8x4
            | Self::Uint16x2
            | Self::Sint16x2
            | Self::Unorm16x2
            | Self::Snorm16x2
            | Self::Float16x2
            | Self::Float32
            | Self::Uint32
            | Self::Sint32
            | Self::Unorm10_10_10_2
            | Self::Unorm8x4Bgra => 4,
            Self::Uint16x4
            | Self::Sint16x4
            | Self::Unorm16x4
            | Self::Snorm16x4
            | Self::Float16x4
            | Self::Float32x2
            | Self::Uint32x2
            | Self::Sint32x2
            | Self::Float64 => 8,
            Self::Float32x3 | Self::Uint32x3 | Self::Sint32x3 => 12,
            Self::Float32x4
            | Self::Uint32x4
            | Self::Sint32x4
            | Self::Float64x2 => 16,
            Self::Float64x3 => 24,
            Self::Float64x4 => 32,
        }
    }

    /// Returns the size read by an acceleration structure build of the vertex format. This is
    /// slightly different from [`Self::size`] because the alpha component of 4-component formats
    /// are not read in an acceleration structure build, allowing for a smaller stride.
    #[must_use]
    pub const fn min_acceleration_structure_vertex_stride(&self) -> u64 {
        match self {
            Self::Float16x2 | Self::Snorm16x2 => 4,
            Self::Float32x3 => 12,
            Self::Float32x2 => 8,
            // This is the minimum value from DirectX
            // > A16 component is ignored, other data can be packed there, such as setting vertex stride to 6 bytes
            //
            // https://microsoft.github.io/DirectX-Specs/d3d/Raytracing.html#d3d12_raytracing_geometry_triangles_desc
            //
            // Vulkan does not express a minimum stride.
            Self::Float16x4 | Self::Snorm16x4 => 6,
            _ => unreachable!(),
        }
    }

    /// Returns the alignment required for `wgpu::BlasTriangleGeometry::vertex_stride`
    #[must_use]
    pub const fn acceleration_structure_stride_alignment(&self) -> u64 {
        match self {
            Self::Float16x4
            | Self::Float16x2
            | Self::Snorm16x4
            | Self::Snorm16x2 => 2,
            Self::Float32x2 | Self::Float32x3 => 4,
            _ => unreachable!(),
        }
    }
}

bitflags::bitflags! {
    /// Different ways that you can use a buffer.
    ///
    /// The usages determine what kind of memory the buffer is allocated from and what
    /// actions the buffer can partake in.
    ///
    /// Specifying only usages the application will actually perform may increase performance.
    /// Additionally, on the WebGL backend, there are restrictions on [`BufferUsages::INDEX`];
    /// see [`DownlevelFlags::UNRESTRICTED_INDEX_BUFFER`] for more information.
    ///
     #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct BufferUsages: u32 {
        /// Allow a buffer to be mapped for reading using [`Buffer::map_async`] + [`Buffer::get_mapped_range`].
        /// This does not include creating a buffer with [`BufferDescriptor::mapped_at_creation`] set.
        ///
        /// If [`Features::MAPPABLE_PRIMARY_BUFFERS`] isn't enabled, the only other usage a buffer
        /// may have is COPY_DST.
        const MAP_READ = 1 << 0;
        /// Allow a buffer to be mapped for writing using [`Buffer::map_async`] + [`Buffer::get_mapped_range_mut`].
        /// This does not include creating a buffer with [`BufferDescriptor::mapped_at_creation`] set.
        ///
        /// If [`Features::MAPPABLE_PRIMARY_BUFFERS`] feature isn't enabled, the only other usage a buffer
        /// may have is COPY_SRC.
        const MAP_WRITE = 1 << 1;
        /// Allow a buffer to be the source buffer for a [`CommandEncoder::copy_buffer_to_buffer`] or [`CommandEncoder::copy_buffer_to_texture`]
        /// operation.
        const COPY_SRC = 1 << 2;
        /// Allow a buffer to be the destination buffer for a [`CommandEncoder::copy_buffer_to_buffer`], [`CommandEncoder::copy_texture_to_buffer`],
        /// [`CommandEncoder::clear_buffer`] or [`Queue::write_buffer`] operation.
        const COPY_DST = 1 << 3;
        /// Allow a buffer to be the index buffer in a draw operation.
        const INDEX = 1 << 4;
        /// Allow a buffer to be the vertex buffer in a draw operation.
        const VERTEX = 1 << 5;
        /// Allow a buffer to be a [`BufferBindingType::Uniform`] inside a bind group.
        const UNIFORM = 1 << 6;
        /// Allow a buffer to be a [`BufferBindingType::Storage`] inside a bind group.
        const STORAGE = 1 << 7;
        /// Allow a buffer to be the indirect buffer in an indirect draw call.
        const INDIRECT = 1 << 8;
        /// Allow a buffer to be the destination buffer for a [`CommandEncoder::resolve_query_set`] operation.
        const QUERY_RESOLVE = 1 << 9;
        /// Allows a buffer to be used as input for a bottom level acceleration structure build
        const BLAS_INPUT = 1 << 10;
        /// Allows a buffer to be used as input for a top level acceleration structure build
        const TLAS_INPUT = 1 << 11;
    }
}

bitflags::bitflags! {
    /// Similar to `BufferUsages`, but used only for `CommandEncoder::transition_resources`.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct BufferUses: u16 {
        /// The argument to a read-only mapping.
        const MAP_READ = 1 << 0;
        /// The argument to a write-only mapping.
        const MAP_WRITE = 1 << 1;
        /// The source of a hardware copy.
        /// cbindgen:ignore
        const COPY_SRC = 1 << 2;
        /// The destination of a hardware copy.
        /// cbindgen:ignore
        const COPY_DST = 1 << 3;
        /// The index buffer used for drawing.
        const INDEX = 1 << 4;
        /// A vertex buffer used for drawing.
        const VERTEX = 1 << 5;
        /// A uniform buffer bound in a bind group.
        const UNIFORM = 1 << 6;
        /// A read-only storage buffer used in a bind group.
        /// cbindgen:ignore
        const STORAGE_READ_ONLY = 1 << 7;
        /// A read-write buffer used in a bind group.
        /// cbindgen:ignore
        const STORAGE_READ_WRITE = 1 << 8;
        /// The indirect or count buffer in a indirect draw or dispatch.
        const INDIRECT = 1 << 9;
        /// A buffer used to store query results.
        const QUERY_RESOLVE = 1 << 10;
        /// Buffer used for acceleration structure building.
        const ACCELERATION_STRUCTURE_SCRATCH = 1 << 11;
        /// Buffer used for bottom level acceleration structure building.
        const BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT = 1 << 12;
        /// Buffer used for top level acceleration structure building.
        const TOP_LEVEL_ACCELERATION_STRUCTURE_INPUT = 1 << 13;
        /// A buffer used to store the compacted size of an acceleration structure
        const ACCELERATION_STRUCTURE_QUERY = 1 << 14;
        /// The combination of states that a buffer may be in _at the same time_.
        const INCLUSIVE = Self::MAP_READ.bits() | Self::COPY_SRC.bits() |
            Self::INDEX.bits() | Self::VERTEX.bits() | Self::UNIFORM.bits() |
            Self::STORAGE_READ_ONLY.bits() | Self::INDIRECT.bits() | Self::BOTTOM_LEVEL_ACCELERATION_STRUCTURE_INPUT.bits() | Self::TOP_LEVEL_ACCELERATION_STRUCTURE_INPUT.bits();
        /// The combination of states that a buffer must exclusively be in.
        const EXCLUSIVE = Self::MAP_WRITE.bits() | Self::COPY_DST.bits() | Self::STORAGE_READ_WRITE.bits() | Self::ACCELERATION_STRUCTURE_SCRATCH.bits();
        /// The combination of all usages that the are guaranteed to be be ordered by the hardware.
        /// If a usage is ordered, then if the buffer state doesn't change between draw calls, there
        /// are no barriers needed for synchronization.
        const ORDERED = Self::INCLUSIVE.bits() | Self::MAP_WRITE.bits();
    }
}

#[derive(Clone, Debug)]
pub struct BufferTransition<T> {
    /// The buffer to transition.
    pub buffer: T,
    /// The new state to transition to.
    pub state: BufferUses,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PresentMode {
    /// Chooses the first supported mode out of:
    ///
    /// 1. [`FifoRelaxed`](Self::FifoRelaxed)
    /// 2. [`Fifo`](Self::Fifo)
    ///
    /// Because of the fallback behavior, this is supported everywhere.
    AutoVsync = 0,

    /// Chooses the first supported mode out of:
    ///
    /// 1. [`Immediate`](Self::Immediate)
    /// 2. [`Mailbox`](Self::Mailbox)
    /// 3. [`Fifo`](Self::Fifo)
    ///
    /// Because of the fallback behavior, this is supported everywhere.
    AutoNoVsync = 1,

    /// Presentation frames are kept in a First-In-First-Out queue approximately 3 frames
    /// long. Every vertical blanking period, the presentation engine will pop a frame
    /// off the queue to display. If there is no frame to display, it will present the same
    /// frame again until the next vblank.
    ///
    /// When a present command is executed on the GPU, the presented image is added on the queue.
    ///
    /// Calls to [`Surface::get_current_texture()`] will block until there is a spot in the queue.
    ///
    /// * **Tearing:** No tearing will be observed.
    /// * **Supported on**: All platforms.
    /// * **Also known as**: "Vsync On"
    ///
    /// This is the [default](Self::default) value for `PresentMode`.
    /// If you don't know what mode to choose, choose this mode.
    ///
    /// [`Surface::get_current_texture()`]: ../wgpu/struct.Surface.html#method.get_current_texture
    #[default]
    Fifo = 2,

    /// Presentation frames are kept in a First-In-First-Out queue approximately 3 frames
    /// long. Every vertical blanking period, the presentation engine will pop a frame
    /// off the queue to display. If there is no frame to display, it will present the
    /// same frame until there is a frame in the queue. The moment there is a frame in the
    /// queue, it will immediately pop the frame off the queue.
    ///
    /// When a present command is executed on the GPU, the presented image is added on the queue.
    ///
    /// Calls to [`Surface::get_current_texture()`] will block until there is a spot in the queue.
    ///
    /// * **Tearing**:
    ///   Tearing will be observed if frames last more than one vblank as the front buffer.
    /// * **Supported on**: AMD on Vulkan.
    /// * **Also known as**: "Adaptive Vsync"
    ///
    /// [`Surface::get_current_texture()`]: ../wgpu/struct.Surface.html#method.get_current_texture
    FifoRelaxed = 3,

    /// Presentation frames are not queued at all. The moment a present command
    /// is executed on the GPU, the presented image is swapped onto the front buffer
    /// immediately.
    ///
    /// * **Tearing**: Tearing can be observed.
    /// * **Supported on**: Most platforms except older DX12 and Wayland.
    /// * **Also known as**: "Vsync Off"
    Immediate = 4,

    /// Presentation frames are kept in a single-frame queue. Every vertical blanking period,
    /// the presentation engine will pop a frame from the queue. If there is no frame to display,
    /// it will present the same frame again until the next vblank.
    ///
    /// When a present command is executed on the GPU, the frame will be put into the queue.
    /// If there was already a frame in the queue, the new frame will _replace_ the old frame
    /// on the queue.
    ///
    /// * **Tearing**: No tearing will be observed.
    /// * **Supported on**: DX12 on Windows 10, NVidia on Vulkan and Wayland on Vulkan.
    /// * **Also known as**: "Fast Vsync"
    Mailbox = 5,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompositeAlphaMode {
    /// Chooses either `Opaque` or `Inherit` automatically，depending on the
    /// `alpha_mode` that the current surface can support.
    Auto = 0,
    /// The alpha channel, if it exists, of the textures is ignored in the
    /// compositing process. Instead, the textures is treated as if it has a
    /// constant alpha of 1.0.
    Opaque = 1,
    /// The alpha channel, if it exists, of the textures is respected in the
    /// compositing process. The non-alpha channels of the textures are
    /// expected to already be multiplied by the alpha channel by the
    /// application.
    PreMultiplied = 2,
    /// The alpha channel, if it exists, of the textures is respected in the
    /// compositing process. The non-alpha channels of the textures are not
    /// expected to already be multiplied by the alpha channel by the
    /// application; instead, the compositor will multiply the non-alpha
    /// channels of the texture by the alpha channel during compositing.
    PostMultiplied = 3,
    /// The alpha channel, if it exists, of the textures is unknown for processing
    /// during compositing. Instead, the application is responsible for setting
    /// the composite alpha blending mode using native WSI command. If not set,
    /// then a platform-specific default will be used.
    Inherit = 4,
}

impl Default for CompositeAlphaMode {
    fn default() -> Self {
        Self::Auto
    }
}

bitflags::bitflags! {
    /// Different ways that you can use a texture.
    ///
    /// The usages determine what kind of memory the texture is allocated from and what
    /// actions the texture can partake in.
    ///
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct TextureUsages: u32 {
        //
        // ---- Start numbering at 1 << 0 ----
        /// Allows a texture to be the source in a [`CommandEncoder::copy_texture_to_buffer`] or
        /// [`CommandEncoder::copy_texture_to_texture`] operation.
        const COPY_SRC = 1 << 0;
        /// Allows a texture to be the destination in a  [`CommandEncoder::copy_buffer_to_texture`],
        /// [`CommandEncoder::copy_texture_to_texture`], or [`Queue::write_texture`] operation.
        const COPY_DST = 1 << 1;
        /// Allows a texture to be a [`BindingType::Texture`] in a bind group.
        const TEXTURE_BINDING = 1 << 2;
        /// Allows a texture to be a [`BindingType::StorageTexture`] in a bind group.
        const STORAGE_BINDING = 1 << 3;
        /// Allows a texture to be an output attachment of a render pass.
        ///
        /// Consider adding [`TextureUsages::TRANSIENT`] if the contents are not reused.
        const RENDER_ATTACHMENT = 1 << 4;

        //
        // ---- Restart Numbering for Native Features ---
        //
        // Native Features:
        //
        /// Allows a texture to be used with image atomics. Requires [`Features::TEXTURE_ATOMIC`].
        const STORAGE_ATOMIC = 1 << 16;
        /// Specifies the contents of this texture will not be used in another pass to potentially reduce memory usage and bandwidth.
        ///
        /// No-op on platforms on platforms that do not benefit from transient textures.
        /// Generally mobile and Apple chips care about this.
        ///
        /// Incompatible with ALL other usages except [`TextureUsages::RENDER_ATTACHMENT`] and requires it.
        ///
        /// Requires [`StoreOp::Discard`].
        const TRANSIENT = 1 << 17;
    }
}

bitflags::bitflags! {
    /// Similar to `TextureUsages`, but used only for `CommandEncoder::transition_resources`.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct TextureUses: u16 {
        /// The texture is in unknown state.
        const UNINITIALIZED = 1 << 0;
        /// Ready to present image to the surface.
        const PRESENT = 1 << 1;
        /// The source of a hardware copy.
        /// cbindgen:ignore
        const COPY_SRC = 1 << 2;
        /// The destination of a hardware copy.
        /// cbindgen:ignore
        const COPY_DST = 1 << 3;
        /// Read-only sampled or fetched resource.
        const RESOURCE = 1 << 4;
        /// The color target of a renderpass.
        const COLOR_TARGET = 1 << 5;
        /// Read-only depth stencil usage.
        const DEPTH_STENCIL_READ = 1 << 6;
        /// Read-write depth stencil usage
        const DEPTH_STENCIL_WRITE = 1 << 7;
        /// Read-only storage texture usage. Corresponds to a UAV in d3d, so is exclusive, despite being read only.
        /// cbindgen:ignore
        const STORAGE_READ_ONLY = 1 << 8;
        /// Write-only storage texture usage.
        /// cbindgen:ignore
        const STORAGE_WRITE_ONLY = 1 << 9;
        /// Read-write storage texture usage.
        /// cbindgen:ignore
        const STORAGE_READ_WRITE = 1 << 10;
        /// Image atomic enabled storage.
        /// cbindgen:ignore
        const STORAGE_ATOMIC = 1 << 11;
        /// Transient texture that may not have any backing memory. Not a resource state stored in the trackers, only used for passing down usages to create_texture.
        const TRANSIENT = 1 << 12;
        /// The combination of states that a texture may be in _at the same time_.
        /// cbindgen:ignore
        const INCLUSIVE = Self::COPY_SRC.bits() | Self::RESOURCE.bits() | Self::DEPTH_STENCIL_READ.bits();
        /// The combination of states that a texture must exclusively be in.
        /// cbindgen:ignore
        const EXCLUSIVE = Self::COPY_DST.bits() | Self::COLOR_TARGET.bits() | Self::DEPTH_STENCIL_WRITE.bits() | Self::STORAGE_READ_ONLY.bits() | Self::STORAGE_WRITE_ONLY.bits() | Self::STORAGE_READ_WRITE.bits() | Self::STORAGE_ATOMIC.bits() | Self::PRESENT.bits();
        /// The combination of all usages that the are guaranteed to be be ordered by the hardware.
        /// If a usage is ordered, then if the texture state doesn't change between draw calls, there
        /// are no barriers needed for synchronization.
        /// cbindgen:ignore
        const ORDERED = Self::INCLUSIVE.bits() | Self::COLOR_TARGET.bits() | Self::DEPTH_STENCIL_WRITE.bits() | Self::STORAGE_READ_ONLY.bits();

        /// Flag used by the wgpu-core texture tracker to say a texture is in different states for every sub-resource
        const COMPLEX = 1 << 13;
        /// Flag used by the wgpu-core texture tracker to say that the tracker does not know the state of the sub-resource.
        /// This is different from UNINITIALIZED as that says the tracker does know, but the texture has not been initialized.
        const UNKNOWN = 1 << 14;
    }
}

/// A texture transition for use with `CommandEncoder::transition_resources`.
#[derive(Clone, Debug)]
pub struct TextureTransition<T> {
    /// The texture to transition.
    pub texture: T,
    /// An optional selector to transition only part of the texture.
    ///
    /// If None, the entire texture will be transitioned.
    pub selector: Option<TextureSelector>,
    /// The new state to transition to.
    pub state: TextureUses,
}

/// Specifies a particular set of subresources in a texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSelector {
    /// Range of mips to use.
    pub mips: Range<u32>,
    /// Range of layers to use.
    pub layers: Range<u32>,
}

#[derive(Debug)]
pub struct SurfaceCapabilities {
    /// List of supported formats to use with the given adapter. The first format in the vector is preferred.
    ///
    /// Returns an empty vector if the surface is incompatible with the adapter.
    pub formats: Vec<TextureFormat>,
    /// List of supported presentation modes to use with the given adapter.
    ///
    /// Returns an empty vector if the surface is incompatible with the adapter.
    pub present_modes: Vec<PresentMode>,
    /// List of supported alpha modes to use with the given adapter.
    ///
    /// Will return at least one element, [`CompositeAlphaMode::Opaque`] or [`CompositeAlphaMode::Inherit`].
    pub alpha_modes: Vec<CompositeAlphaMode>,
    /// Bitflag of supported texture usages for the surface to use with the given adapter.
    ///
    /// The usage [`TextureUsages::RENDER_ATTACHMENT`] is guaranteed.
    pub usages: TextureUsages,
}

impl Default for SurfaceCapabilities {
    fn default() -> Self {
        Self {
            formats: Vec::new(),
            present_modes: Vec::new(),
            alpha_modes: vec![CompositeAlphaMode::Opaque],
            usages: TextureUsages::RENDER_ATTACHMENT,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceConfiguration<V> {
    /// The usage of the swap chain. The only usage guaranteed to be supported is [`TextureUsages::RENDER_ATTACHMENT`].
    pub usage: TextureUsages,
    /// The texture format of the swap chain. The only formats that are guaranteed are
    /// [`TextureFormat::Bgra8Unorm`] and [`TextureFormat::Bgra8UnormSrgb`].
    pub format: TextureFormat,
    /// Width of the swap chain. Must be the same size as the surface, and nonzero.
    ///
    /// If this is not the same size as the underlying surface (e.g. if it is
    /// set once, and the window is later resized), the behaviour is defined
    /// but platform-specific, and may change in the future (currently macOS
    /// scales the surface, other platforms may do something else).
    pub width: u32,
    /// Height of the swap chain. Must be the same size as the surface, and nonzero.
    ///
    /// If this is not the same size as the underlying surface (e.g. if it is
    /// set once, and the window is later resized), the behaviour is defined
    /// but platform-specific, and may change in the future (currently macOS
    /// scales the surface, other platforms may do something else).
    pub height: u32,
    /// Presentation mode of the swap chain. Fifo is the only mode guaranteed to be supported.
    /// `FifoRelaxed`, `Immediate`, and `Mailbox` will crash if unsupported, while `AutoVsync` and
    /// `AutoNoVsync` will gracefully do a designed sets of fallbacks if their primary modes are
    /// unsupported.
    pub present_mode: PresentMode,
    /// Desired maximum number of monitor refreshes between a [`Surface::get_current_texture`] call and the
    /// texture being presented to the screen. This is sometimes called "Frames in Flight".
    ///
    /// Defaults to `2` when created via [`Surface::get_default_config`] as this is a reasonable default.
    ///
    /// This is ultimately a hint to the backend implementation and will always be clamped
    /// to the supported range.
    ///
    /// Typical values are `1` to `3`, but higher values are valid, though likely to be clamped.
    /// * Choose `1` to minimize latency above all else. This only gives a single monitor refresh for all of
    ///   the CPU and GPU work to complete. ⚠️ As a result of these short swapchains, the CPU and GPU
    ///   cannot run in parallel, prioritizing latency over throughput. For applications like GUIs doing
    ///   a small amount of GPU work each frame that need low latency, this is a reasonable choice.
    /// * Choose `2` for a balance between latency and throughput. The CPU and GPU both can each use
    ///   a full monitor refresh to do their computations. This is a reasonable default for most applications.
    /// * Choose `3` or higher to maximize throughput, sacrificing latency when the the CPU and GPU
    ///   are using less than a full monitor refresh each. For applications that use CPU-side pipelining
    ///   of frames this may be a reasonable choice. ⚠️ On 60hz displays the latency can be very noticeable.
    ///
    /// This maps to the backend in the following ways:
    /// - Vulkan: Number of frames in the swapchain is `desired_maximum_frame_latency + 1`,
    ///   clamped to the supported range.
    /// - DX12: Calls [`IDXGISwapChain2::SetMaximumFrameLatency(desired_maximum_frame_latency)`][SMFL].
    /// - Metal: Sets the `maximumDrawableCount` of the underlying `CAMetalLayer` to
    ///   `desired_maximum_frame_latency + 1`, clamped to the supported range.
    /// - OpenGL: Ignored
    ///
    /// It also has various subtle interactions with various present modes and APIs.
    /// - DX12 + Mailbox: Limits framerate to `desired_maximum_frame_latency * Monitor Hz` fps.
    /// - Vulkan/Metal + Mailbox: If this is set to `2`, limits framerate to `2 * Monitor Hz` fps. `3` or higher is unlimited.
    ///
    /// [`Surface::get_current_texture`]: ../wgpu/struct.Surface.html#method.get_current_texture
    /// [`Surface::get_default_config`]: ../wgpu/struct.Surface.html#method.get_default_config
    /// [SMFL]: https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-setmaximumframelatency
    pub desired_maximum_frame_latency: u32,
    /// Specifies how the alpha channel of the textures should be handled during compositing.
    pub alpha_mode: CompositeAlphaMode,
    /// Specifies what view formats will be allowed when calling `Texture::create_view` on the texture returned by `Surface::get_current_texture`.
    ///
    /// View formats of the same format as the texture are always allowed.
    ///
    /// Note: currently, only the srgb-ness is allowed to change. (ex: `Rgba8Unorm` texture + `Rgba8UnormSrgb` view)
    pub view_formats: V,
}

impl<V: Clone> SurfaceConfiguration<V> {
    /// Map `view_formats` of the texture descriptor into another.
    pub fn map_view_formats<M>(
        &self, fun: impl FnOnce(V) -> M,
    ) -> SurfaceConfiguration<M> {
        SurfaceConfiguration {
            usage: self.usage,
            format: self.format,
            width: self.width,
            height: self.height,
            present_mode: self.present_mode,
            desired_maximum_frame_latency: self.desired_maximum_frame_latency,
            alpha_mode: self.alpha_mode,
            view_formats: fun(self.view_formats.clone()),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub enum SurfaceStatus {
    /// No issues.
    Good,
    /// The swap chain is operational, but it does no longer perfectly
    /// match the surface. A re-configuration is needed.
    Suboptimal,
    /// Unable to get the next frame, timed out.
    Timeout,
    /// The surface under the swap chain has changed.
    Outdated,
    /// The surface under the swap chain is lost.
    Lost,
    /// The surface status is not known since `Surface::get_current_texture` previously failed.
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PresentationTimestamp(
    /// Timestamp in nanoseconds.
    pub u128,
);

impl PresentationTimestamp {
    /// A timestamp that is invalid due to the platform not having a timestamp system.
    pub const INVALID_TIMESTAMP: Self = Self(u128::MAX);

    /// Returns true if this timestamp is the invalid timestamp.
    #[must_use]
    pub fn is_invalid(self) -> bool {
        self == Self::INVALID_TIMESTAMP
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    /// Red component of the color
    pub r: f64,
    /// Green component of the color
    pub g: f64,
    /// Blue component of the color
    pub b: f64,
    /// Alpha component of the color
    pub a: f64,
}

impl Color {
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    pub const BLUE: Self = Self { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Origin2d {
    #[allow(missing_docs)]
    pub x: u32,
    #[allow(missing_docs)]
    pub y: u32,
}

impl Origin2d {
    /// Zero origin.
    pub const ZERO: Self = Self { x: 0, y: 0 };

    /// Adds the third dimension to this origin
    #[must_use]
    pub fn to_3d(self, z: u32) -> Origin3d {
        Origin3d { x: self.x, y: self.y, z }
    }
}

impl core::fmt::Debug for Origin2d {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (self.x, self.y).fmt(f)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Origin3d {
    /// X position of the origin
    pub x: u32,
    /// Y position of the origin
    pub y: u32,
    /// Z position of the origin
    pub z: u32,
}

impl Origin3d {
    /// Zero origin.
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    /// Removes the third dimension from this origin
    #[must_use]
    pub fn to_2d(self) -> Origin2d {
        Origin2d { x: self.x, y: self.y }
    }
}

impl Default for Origin3d {
    fn default() -> Self {
        Self::ZERO
    }
}

impl core::fmt::Debug for Origin3d {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (self.x, self.y, self.z).fmt(f)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Extent3d {
    /// Width of the extent
    pub width: u32,
    /// Height of the extent
    pub height: u32,
    /// The depth of the extent or the number of array layers
    pub depth_or_array_layers: u32,
}

impl core::fmt::Debug for Extent3d {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (self.width, self.height, self.depth_or_array_layers).fmt(f)
    }
}

impl Default for Extent3d {
    fn default() -> Self {
        Self { width: 1, height: 1, depth_or_array_layers: 1 }
    }
}

impl Extent3d {
    /// Calculates the [physical size] backing a texture of the given
    /// format and extent.  This includes padding to the block width
    /// and height of the format.
    ///
    /// This is the texture extent that you must upload at when uploading to _mipmaps_ of compressed textures.
    ///
    #[must_use]
    pub fn physical_size(&self, format: TextureFormat) -> Self {
        let (block_width, block_height) = format.block_dimensions();

        let width = self.width.div_ceil(block_width) * block_width;
        let height = self.height.div_ceil(block_height) * block_height;

        Self {
            width,
            height,
            depth_or_array_layers: self.depth_or_array_layers,
        }
    }

    /// Calculates the maximum possible count of mipmaps.
    ///
    /// Treats the depth as part of the mipmaps. If calculating
    /// for a 2DArray texture, which does not mipmap depth, set depth to 1.
    #[must_use]
    pub fn max_mips(&self, dim: TextureDimension) -> u32 {
        match dim {
            TextureDimension::D1 => 1,
            TextureDimension::D2 => {
                let max_dim = self.width.max(self.height);
                32 - max_dim.leading_zeros()
            }
            TextureDimension::D3 => {
                let max_dim =
                    self.width.max(self.height.max(self.depth_or_array_layers));
                32 - max_dim.leading_zeros()
            }
        }
    }

    /// Calculates the extent at a given mip level.
    /// Does *not* account for memory size being a multiple of block size.
    ///
    #[must_use]
    pub fn mip_level_size(&self, level: u32, dim: TextureDimension) -> Self {
        Self {
            width: u32::max(1, self.width >> level),
            height: match dim {
                TextureDimension::D1 => 1,
                _ => u32::max(1, self.height >> level),
            },
            depth_or_array_layers: match dim {
                TextureDimension::D1 => 1,
                TextureDimension::D2 => self.depth_or_array_layers,
                TextureDimension::D3 => {
                    u32::max(1, self.depth_or_array_layers >> level)
                }
            },
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExternalTextureFormat {
    /// Single [`TextureFormat::Rgba8Unorm`] or [`TextureFormat::Bgra8Unorm`] format plane.
    Rgba,
    /// [`TextureFormat::R8Unorm`] Y plane, and [`TextureFormat::Rg8Unorm`]
    /// interleaved CbCr plane.
    Nv12,
    /// Separate [`TextureFormat::R8Unorm`] Y, Cb, and Cr planes.
    Yu12,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ExternalTextureTransferFunction {
    pub a: f32,
    pub b: f32,
    pub g: f32,
    pub k: f32,
}

impl Default for ExternalTextureTransferFunction {
    fn default() -> Self {
        Self { a: 1.0, b: 1.0, g: 1.0, k: 1.0 }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum AddressMode {
    /// Clamp the value to the edge of the texture
    ///
    /// -0.25 -> 0.0
    /// 1.25  -> 1.0
    #[default]
    ClampToEdge = 0,
    /// Repeat the texture in a tiling fashion
    ///
    /// -0.25 -> 0.75
    /// 1.25 -> 0.25
    Repeat = 1,
    /// Repeat the texture, mirroring it every repeat
    ///
    /// -0.25 -> 0.25
    /// 1.25 -> 0.75
    MirrorRepeat = 2,
    /// Clamp the value to the border of the texture
    /// Requires feature [`Features::ADDRESS_MODE_CLAMP_TO_BORDER`]
    ///
    /// -0.25 -> border
    /// 1.25 -> border
    ClampToBorder = 3,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum FilterMode {
    /// Nearest neighbor sampling.
    ///
    /// This creates a pixelated effect.
    #[default]
    Nearest = 0,
    /// Linear Interpolation
    ///
    /// This makes textures smooth but blurry.
    Linear = 1,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Hash, Eq, PartialEq)]
pub enum MipmapFilterMode {
    /// Nearest neighbor sampling.
    ///
    /// Return the value of the texel nearest to the texture coordinates.
    #[default]
    Nearest = 0,
    /// Linear Interpolation
    ///
    /// Select two texels in each dimension and return a linear interpolation between their values.
    Linear = 1,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PushConstantRange {
    /// Stage push constant range is visible from. Each stage can only be served by at most one range.
    /// One range can serve multiple stages however.
    pub stages: ShaderStages,
    /// Range in push constant memory to use for the stage. Must be less than [`Limits::max_push_constant_size`].
    /// Start and end must be aligned to the 4s.
    pub range: Range<u32>,
}

#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CommandBufferDescriptor<L> {
    /// Debug label of this command buffer.
    pub label: L,
}

impl<L> CommandBufferDescriptor<L> {
    /// Takes a closure and maps the label of the command buffer descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> CommandBufferDescriptor<K> {
        CommandBufferDescriptor { label: fun(&self.label) }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderBundleDepthStencil {
    /// Format of the attachment.
    pub format: TextureFormat,
    /// If the depth aspect of the depth stencil attachment is going to be written to.
    ///
    /// This must match the [`RenderPassDepthStencilAttachment::depth_ops`] of the renderpass this render bundle is executed in.
    /// If `depth_ops` is `Some(..)` this must be false. If it is `None` this must be true.
    ///
    /// [`RenderPassDepthStencilAttachment::depth_ops`]: ../wgpu/struct.RenderPassDepthStencilAttachment.html#structfield.depth_ops
    pub depth_read_only: bool,

    /// If the stencil aspect of the depth stencil attachment is going to be written to.
    ///
    /// This must match the [`RenderPassDepthStencilAttachment::stencil_ops`] of the renderpass this render bundle is executed in.
    /// If `depth_ops` is `Some(..)` this must be false. If it is `None` this must be true.
    ///
    /// [`RenderPassDepthStencilAttachment::stencil_ops`]: ../wgpu/struct.RenderPassDepthStencilAttachment.html#structfield.stencil_ops
    pub stencil_read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BindGroupLayoutEntry {
    /// Binding index. Must match shader index and be unique inside a `BindGroupLayout`. A binding
    /// of index 1, would be described as `@group(0) @binding(1)` in shaders.
    pub binding: u32,
    /// Which shader stages can see this binding.
    pub visibility: ShaderStages,
    /// The type of the binding
    pub ty: BindingType,
    /// If the binding is an array of multiple resources. Corresponds to `binding_array<T>` in the shader.
    ///
    /// When this is `Some` the following validation applies:
    /// - Size must be of value 1 or greater.
    /// - When `ty == BindingType::Texture`, [`Features::TEXTURE_BINDING_ARRAY`] must be supported.
    /// - When `ty == BindingType::Sampler`, [`Features::TEXTURE_BINDING_ARRAY`] must be supported.
    /// - When `ty == BindingType::Buffer`, [`Features::BUFFER_BINDING_ARRAY`] must be supported.
    /// - When `ty == BindingType::Buffer` and `ty.ty == BufferBindingType::Storage`, [`Features::STORAGE_RESOURCE_BINDING_ARRAY`] must be supported.
    /// - When `ty == BindingType::StorageTexture`, [`Features::STORAGE_RESOURCE_BINDING_ARRAY`] must be supported.
    /// - When any binding in the group is an array, no `BindingType::Buffer` in the group may have `has_dynamic_offset == true`
    /// - When any binding in the group is an array, no `BindingType::Buffer` in the group may have `ty.ty == BufferBindingType::Uniform`.
    ///
    pub count: Option<NonZeroU32>,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TexelCopyBufferInfo<B> {
    /// The buffer to be copied to/from.
    pub buffer: B,
    /// The layout of the texture data in this buffer.
    pub layout: TexelCopyBufferLayout,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TexelCopyTextureInfo<T> {
    /// The texture to be copied to/from.
    pub texture: T,
    /// The target mip level of the texture.
    pub mip_level: u32,
    /// The base texel of the texture in the selected `mip_level`. Together
    /// with the `copy_size` argument to copy functions, defines the
    /// sub-region of the texture to copy.
    pub origin: Origin3d,
    /// The copy aspect.
    pub aspect: TextureAspect,
}

impl<T> TexelCopyTextureInfo<T> {
    /// Adds color space and premultiplied alpha information to make this
    /// descriptor tagged.
    pub fn to_tagged(
        self, color_space: PredefinedColorSpace, premultiplied_alpha: bool,
    ) -> CopyExternalImageDestInfo<T> {
        CopyExternalImageDestInfo {
            texture: self.texture,
            mip_level: self.mip_level,
            origin: self.origin,
            aspect: self.aspect,
            color_space,
            premultiplied_alpha,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PredefinedColorSpace {
    /// sRGB color space
    Srgb,
    /// Display-P3 color space
    DisplayP3,
}

#[derive(Copy, Clone, Debug)]
pub struct CopyExternalImageDestInfo<T> {
    /// The texture to be copied to/from.
    pub texture: T,
    /// The target mip level of the texture.
    pub mip_level: u32,
    /// The base texel of the texture in the selected `mip_level`.
    pub origin: Origin3d,
    /// The copy aspect.
    pub aspect: TextureAspect,
    /// The color space of this texture.
    pub color_space: PredefinedColorSpace,
    /// The premultiplication of this texture
    pub premultiplied_alpha: bool,
}

impl<T> CopyExternalImageDestInfo<T> {
    /// Removes the colorspace information from the type.
    pub fn to_untagged(self) -> TexelCopyTextureInfo<T> {
        TexelCopyTextureInfo {
            texture: self.texture,
            mip_level: self.mip_level,
            origin: self.origin,
            aspect: self.aspect,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageSubresourceRange {
    /// Aspect of the texture. Color textures must be [`TextureAspect::All`][TAA].
    ///
    pub aspect: TextureAspect,
    /// Base mip level.
    pub base_mip_level: u32,
    /// Mip level count.
    /// If `Some(count)`, `base_mip_level + count` must be less or equal to underlying texture mip count.
    /// If `None`, considered to include the rest of the mipmap levels, but at least 1 in total.
    pub mip_level_count: Option<u32>,
    /// Base array layer.
    pub base_array_layer: u32,
    /// Layer count.
    /// If `Some(count)`, `base_array_layer + count` must be less or equal to the underlying array count.
    /// If `None`, considered to include the rest of the array layers, but at least 1 in total.
    pub array_layer_count: Option<u32>,
}

impl ImageSubresourceRange {
    #[must_use]
    pub fn is_full_resource(
        &self, format: TextureFormat, mip_levels: u32, array_layers: u32,
    ) -> bool {
        // Mip level count and array layer count need to deal with both the None and Some(count) case.
        let mip_level_count = self.mip_level_count.unwrap_or(mip_levels);
        let array_layer_count = self.array_layer_count.unwrap_or(array_layers);

        let aspect_eq =
            Some(format) == format.aspect_specific_format(self.aspect);

        let base_mip_level_eq = self.base_mip_level == 0;
        let mip_level_count_eq = mip_level_count == mip_levels;

        let base_array_layer_eq = self.base_array_layer == 0;
        let array_layer_count_eq = array_layer_count == array_layers;

        aspect_eq
            && base_mip_level_eq
            && mip_level_count_eq
            && base_array_layer_eq
            && array_layer_count_eq
    }

    /// Returns the mip level range of a subresource range describes for a specific texture.
    #[must_use]
    pub fn mip_range(&self, mip_level_count: u32) -> Range<u32> {
        self.base_mip_level..match self.mip_level_count {
            Some(mip_level_count) => self.base_mip_level + mip_level_count,
            None => mip_level_count,
        }
    }

    /// Returns the layer range of a subresource range describes for a specific texture.
    #[must_use]
    pub fn layer_range(&self, array_layer_count: u32) -> Range<u32> {
        self.base_array_layer..match self.array_layer_count {
            Some(array_layer_count) => {
                self.base_array_layer + array_layer_count
            }
            None => array_layer_count,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SamplerBorderColor {
    /// [0, 0, 0, 0]
    TransparentBlack,
    /// [0, 0, 0, 1]
    OpaqueBlack,
    /// [1, 1, 1, 1]
    OpaqueWhite,

    /// On the Metal backend, this is equivalent to `TransparentBlack` for
    /// textures that have an alpha component, and equivalent to `OpaqueBlack`
    /// for textures that do not have an alpha component. On other backends,
    /// this is equivalent to `TransparentBlack`. Requires
    /// [`Features::ADDRESS_MODE_CLAMP_TO_ZERO`]. Not supported on the web.
    Zero,
}

#[derive(Copy, Clone, Debug)]
pub enum QueryType {
    /// Query returns a single 64-bit number, serving as an occlusion boolean.
    Occlusion,
    /// Query returns up to 5 64-bit numbers based on the given flags.
    ///
    /// See [`PipelineStatisticsTypes`]'s documentation for more information
    /// on how they get resolved.
    ///
    /// [`Features::PIPELINE_STATISTICS_QUERY`] must be enabled to use this query type.
    PipelineStatistics(PipelineStatisticsTypes),
    /// Query returns a 64-bit number indicating the GPU-timestamp
    /// where all previous commands have finished executing.
    ///
    /// Must be multiplied by [`Queue::get_timestamp_period`][Qgtp] to get
    /// the value in nanoseconds. Absolute values have no meaning,
    /// but timestamps can be subtracted to get the time it takes
    /// for a string of operations to complete.
    ///
    /// [`Features::TIMESTAMP_QUERY`] must be enabled to use this query type.
    ///
    Timestamp,
}

bitflags::bitflags! {
    /// Flags for which pipeline data should be recorded in a query.
    ///
    /// Used in [`QueryType`].
    ///
    /// The amount of values written when resolved depends
    /// on the amount of flags set. For example, if 3 flags are set, 3
    /// 64-bit values will be written per query.
    ///
    /// The order they are written is the order they are declared
    /// in these bitflags. For example, if you enabled `CLIPPER_PRIMITIVES_OUT`
    /// and `COMPUTE_SHADER_INVOCATIONS`, it would write 16 bytes,
    /// the first 8 bytes being the primitive out value, the last 8
    /// bytes being the compute shader invocation count.
    #[repr(transparent)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct PipelineStatisticsTypes : u8 {
        /// Amount of times the vertex shader is ran. Accounts for
        /// the vertex cache when doing indexed rendering.
        const VERTEX_SHADER_INVOCATIONS = 1 << 0;
        /// Amount of times the clipper is invoked. This
        /// is also the amount of triangles output by the vertex shader.
        const CLIPPER_INVOCATIONS = 1 << 1;
        /// Amount of primitives that are not culled by the clipper.
        /// This is the amount of triangles that are actually on screen
        /// and will be rasterized and rendered.
        const CLIPPER_PRIMITIVES_OUT = 1 << 2;
        /// Amount of times the fragment shader is ran. Accounts for
        /// fragment shaders running in 2x2 blocks in order to get
        /// derivatives.
        const FRAGMENT_SHADER_INVOCATIONS = 1 << 3;
        /// Amount of times a compute shader is invoked. This will
        /// be equivalent to the dispatch count times the workgroup size.
        const COMPUTE_SHADER_INVOCATIONS = 1 << 4;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DrawIndirectArgs {
    /// The number of vertices to draw.
    pub vertex_count: u32,
    /// The number of instances to draw.
    pub instance_count: u32,
    /// The Index of the first vertex to draw.
    pub first_vertex: u32,
    /// The instance ID of the first instance to draw.
    ///
    /// Has to be 0, unless [`Features::INDIRECT_FIRST_INSTANCE`](crate::Features::INDIRECT_FIRST_INSTANCE) is enabled.
    pub first_instance: u32,
}

impl DrawIndirectArgs {
    /// Returns the bytes representation of the struct, ready to be written in a buffer.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DrawIndexedIndirectArgs {
    /// The number of indices to draw.
    pub index_count: u32,
    /// The number of instances to draw.
    pub instance_count: u32,
    /// The first index within the index buffer.
    pub first_index: u32,
    /// The value added to the vertex index before indexing into the vertex buffer.
    pub base_vertex: i32,
    /// The instance ID of the first instance to draw.
    ///
    /// Has to be 0, unless [`Features::INDIRECT_FIRST_INSTANCE`](crate::Features::INDIRECT_FIRST_INSTANCE) is enabled.
    pub first_instance: u32,
}

impl DrawIndexedIndirectArgs {
    /// Returns the bytes representation of the struct, ready to be written in a buffer.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DispatchIndirectArgs {
    /// The number of work groups in X dimension.
    pub x: u32,
    /// The number of work groups in Y dimension.
    pub y: u32,
    /// The number of work groups in Z dimension.
    pub z: u32,
}

impl DispatchIndirectArgs {
    /// Returns the bytes representation of the struct, ready to be written into a buffer.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ShaderRuntimeChecks {
    /// Enforce bounds checks in shaders, even if the underlying driver doesn't
    /// support doing so natively.
    ///
    /// When this is `true`, `wgpu` promises that shaders can only read or
    /// write the accessible region of a bindgroup's buffer bindings. If
    /// the underlying graphics platform cannot implement these bounds checks
    /// itself, `wgpu` will inject bounds checks before presenting the
    /// shader to the platform.
    ///
    /// When this is `false`, `wgpu` only enforces such bounds checks if the
    /// underlying platform provides a way to do so itself. `wgpu` does not
    /// itself add any bounds checks to generated shader code.
    ///
    /// Note that `wgpu` users may try to initialize only those portions of
    /// buffers that they anticipate might be read from. Passing `false` here
    /// may allow shaders to see wider regions of the buffers than expected,
    /// making such deferred initialization visible to the application.
    pub bounds_checks: bool,
    ///
    /// If false, the caller MUST ensure that all passed shaders do not contain any infinite loops.
    ///
    /// If it does, backend compilers MAY treat such a loop as unreachable code and draw
    /// conclusions about other safety-critical code paths. This option SHOULD NOT be disabled
    /// when running untrusted code.
    pub force_loop_bounding: bool,
}

impl ShaderRuntimeChecks {
    /// Creates a new configuration where the shader is fully checked.
    #[must_use]
    pub const fn checked() -> Self {
        unsafe { Self::all(true) }
    }

    /// Creates a new configuration where none of the checks are performed.
    ///
    /// # Safety
    ///
    /// See the documentation for the `set_*` methods for the safety requirements
    /// of each sub-configuration.
    #[must_use]
    pub const fn unchecked() -> Self {
        unsafe { Self::all(false) }
    }

    /// Creates a new configuration where all checks are enabled or disabled. To safely
    /// create a configuration with all checks enabled, use [`ShaderRuntimeChecks::checked`].
    ///
    /// # Safety
    ///
    /// See the documentation for the `set_*` methods for the safety requirements
    /// of each sub-configuration.
    #[must_use]
    pub const unsafe fn all(all_checks: bool) -> Self {
        Self { bounds_checks: all_checks, force_loop_bounding: all_checks }
    }
}

impl Default for ShaderRuntimeChecks {
    fn default() -> Self {
        Self::checked()
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
/// Update mode for acceleration structure builds.
pub enum AccelerationStructureUpdateMode {
    /// Always perform a full build.
    Build,
    /// If possible, perform an incremental update.
    ///
    /// Not advised for major topology changes.
    /// (Useful for e.g. skinning)
    PreferUpdate,
}

bitflags::bitflags!(
    /// Flags for acceleration structures
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct AccelerationStructureFlags: u8 {
        /// Allow for incremental updates (no change in size), currently this is unimplemented
        /// and will build as normal (this is fine, update vs build should be unnoticeable)
        const ALLOW_UPDATE = 1 << 0;
        /// Allow the acceleration structure to be compacted in a copy operation
        /// (`Blas::prepare_for_compaction`, `CommandEncoder::compact_blas`).
        const ALLOW_COMPACTION = 1 << 1;
        /// Optimize for fast ray tracing performance, recommended if the geometry is unlikely
        /// to change (e.g. in a game: non-interactive scene geometry)
        const PREFER_FAST_TRACE = 1 << 2;
        /// Optimize for fast build time, recommended if geometry is likely to change frequently
        /// (e.g. in a game: player model).
        const PREFER_FAST_BUILD = 1 << 3;
        /// Optimize for low memory footprint (both while building and in the output BLAS).
        const LOW_MEMORY = 1 << 4;
        /// Use `BlasTriangleGeometry::transform_buffer` when building a BLAS (only allowed in
        /// BLAS creation)
        const USE_TRANSFORM = 1 << 5;
        /// Allow retrieval of the vertices of the triangle hit by a ray.
        const ALLOW_RAY_HIT_VERTEX_RETURN = 1 << 6;
    }
);

bitflags::bitflags!(
    /// Flags for acceleration structure geometries
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct AccelerationStructureGeometryFlags: u8 {
        /// Is OPAQUE (is there no alpha test) recommended as currently in naga there is no
        /// candidate intersections yet so currently BLASes without this flag will not have hits.
        /// Not enabling this makes the BLAS unable to be interacted with in WGSL.
        const OPAQUE = 1 << 0;
        /// NO_DUPLICATE_ANY_HIT_INVOCATION, not useful unless using hal with wgpu, ray-tracing
        /// pipelines are not supported in wgpu so any-hit shaders do not exist. For when any-hit
        /// shaders are implemented (or experienced users who combine this with an underlying library:
        /// for any primitive (triangle or AABB) multiple any-hit shaders sometimes may be invoked
        /// (especially in AABBs like a sphere), if this flag in present only one hit on a primitive may
        /// invoke an any-hit shader.
        const NO_DUPLICATE_ANY_HIT_INVOCATION = 1 << 1;
    }
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// What a copy between acceleration structures should do
pub enum AccelerationStructureCopy {
    /// Directly duplicate an acceleration structure to another
    Clone,
    /// Duplicate and compact an acceleration structure
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// What type the data of an acceleration structure is
pub enum AccelerationStructureType {
    /// The types of the acceleration structure are triangles
    Triangles,
    /// The types of the acceleration structure are axis aligned bounding boxes
    AABBs,
    /// The types of the acceleration structure are instances
    Instances,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DeviceLostReason {
    /// The device was lost for an unspecific reason, including driver errors.
    Unknown = 0,
    /// The device's `destroy` method was called.
    Destroyed = 1,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum BufferBindingType {
    #[default]
    Uniform,
    Storage {
        read_only: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TextureSampleType {
    Float {
        filterable: bool,
    },
    Depth,
    Sint,
    Uint,
}

impl Default for TextureSampleType {
    fn default() -> Self {
        Self::Float { filterable: true }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StorageTextureAccess {
    WriteOnly,
    ReadOnly,
    ReadWrite,
    Atomic,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SamplerBindingType {
    /// The sampling result is produced based on more than a single color sample from a texture,
    /// e.g. when bilinear interpolation is enabled.
    Filtering,
    /// The sampling result is produced based on a single color sample from a texture.
    NonFiltering,
    /// Use as a comparison sampler instead of a normal sampler.
    /// For more info take a look at the analogous functionality in OpenGL: <https://www.khronos.org/opengl/wiki/Sampler_Object#Comparison_mode>.
    Comparison,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum BindingType {
    /// A buffer binding.
    Buffer {
        /// Sub-type of the buffer binding.
        ty: BufferBindingType,

        /// Indicates that the binding has a dynamic offset.
        ///
        /// One offset must be passed to [`RenderPass::set_bind_group`][RPsbg]
        /// for each dynamic binding in increasing order of binding number.
        ///
        /// [RPsbg]: ../wgpu/struct.RenderPass.html#method.set_bind_group
        has_dynamic_offset: bool,

        min_binding_size: Option<BufferSize>,
    },
    Sampler(SamplerBindingType),
    Texture {
        /// Sample type of the texture binding.
        sample_type: TextureSampleType,
        /// Dimension of the texture view that is going to be sampled.
        view_dimension: TextureViewDimension,
        /// True if the texture has a sample count greater than 1. If this is true,
        /// the texture must be declared as `texture_multisampled_2d` or
        /// `texture_depth_multisampled_2d` in the shader, and read using `textureLoad`.
        multisampled: bool,
    },
    StorageTexture {
        /// Allowed access to this texture.
        access: StorageTextureAccess,
        /// Format of the texture.
        format: TextureFormat,
        /// Dimension of the texture view that is going to be sampled.
        view_dimension: TextureViewDimension,
    },

    AccelerationStructure {
        /// Whether this acceleration structure can be used to
        /// create a ray query that has flag vertex return in the shader
        ///
        /// If enabled requires [`Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN`]
        vertex_return: bool,
    },

    ExternalTexture,
}

impl BindingType {
    /// Returns true for buffer bindings with dynamic offset enabled.
    #[must_use]
    pub fn has_dynamic_offset(&self) -> bool {
        match *self {
            Self::Buffer { has_dynamic_offset, .. } => has_dynamic_offset,
            _ => false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TexelCopyBufferLayout {
    /// Offset into the buffer that is the start of the texture. Must be a multiple of texture block size.
    /// For non-compressed textures, this is 1.
    pub offset: BufferAddress,
    /// Bytes per "row" in an image.
    ///
    /// A row is one row of pixels or of compressed blocks in the x direction.
    ///
    /// This value is required if there are multiple rows (i.e. height or depth is more than one pixel or pixel block for compressed textures)
    ///
    /// Must be a multiple of 256 for [`CommandEncoder::copy_buffer_to_texture`][CEcbtt]
    /// and [`CommandEncoder::copy_texture_to_buffer`][CEcttb]. You must manually pad the
    /// image such that this is a multiple of 256. It will not affect the image data.
    ///
    /// [`Queue::write_texture`][Qwt] does not have this requirement.
    ///
    /// Must be a multiple of the texture block size. For non-compressed textures, this is 1.
    ///
    /// [CEcbtt]: ../wgpu/struct.CommandEncoder.html#method.copy_buffer_to_texture
    /// [CEcttb]: ../wgpu/struct.CommandEncoder.html#method.copy_texture_to_buffer
    /// [Qwt]: ../wgpu/struct.Queue.html#method.write_texture
    pub bytes_per_row: Option<u32>,
    /// "Rows" that make up a single "image".
    ///
    /// A row is one row of pixels or of compressed blocks in the x direction.
    ///
    /// An image is one layer in the z direction of a 3D image or 2DArray texture.
    ///
    /// The amount of rows per image may be larger than the actual amount of rows of data.
    ///
    /// Required if there are multiple images (i.e. the depth is more than one).
    pub rows_per_image: Option<u32>,
}
