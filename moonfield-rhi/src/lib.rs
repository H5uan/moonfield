use std::ops::Range;

mod error;
mod features;
mod instance;

pub type BufferAddress = u64;
pub type BufferSize = core::num::NonZeroU64;
pub type ShaderLocation = u32;

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
    
    pub const ALL: [Backend; Self::COUNT] = [
        Self::Noop,
        Self::Vulkan,
        Self::Metal,
        Self::Dx12,
    ];

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
    /// Represents the backends that wgpu will use.
    #[repr(transparent)]
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    #[cfg_attr(feature = "serde", serde(transparent))]
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

        /// All the apis that wgpu offers first tier of support for.
        ///
        /// * [`Backends::VULKAN`]
        /// * [`Backends::METAL`]
        /// * [`Backends::DX12`]
        /// * [`Backends::BROWSER_WEBGPU`]
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
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase", default))]
pub struct Limits {
    /// Maximum allowed value for the `size.width` of a texture created with `TextureDimension::D1`.
    /// Defaults to 8192. Higher is "better".
    #[cfg_attr(feature = "serde", serde(rename = "maxTextureDimension1D"))]
    pub max_texture_dimension_1d: u32,
    /// Maximum allowed value for the `size.width` and `size.height` of a texture created with `TextureDimension::D2`.
    /// Defaults to 8192. Higher is "better".
    #[cfg_attr(feature = "serde", serde(rename = "maxTextureDimension2D"))]
    pub max_texture_dimension_2d: u32,
    /// Maximum allowed value for the `size.width`, `size.height`, and `size.depth_or_array_layers`
    /// of a texture created with `TextureDimension::D3`.
    /// Defaults to 2048. Higher is "better".
    #[cfg_attr(feature = "serde", serde(rename = "maxTextureDimension3D"))]
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

/// Collections of shader features a device supports if they support less than WebGPU normally allows.
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
