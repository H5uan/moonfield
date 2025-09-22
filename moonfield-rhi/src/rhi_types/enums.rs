use std::str::FromStr;

use bitflags::bitflags;

use super::errors::FeatureParseError;


/// Graphics API backend types supported by the RHI.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Backend {
    /// No-operation backend for testing and fallback scenarios
    Noop = 0,
    /// Vulkan API (Windows, Linux, Android, MacOS via `vulkan-portability`/MoltenVK)
    #[default]
    Vulkan = 1,
    /// Metal API (Apple platforms)
    Metal = 2,
    /// Direct3D-12 (Windows)
    D3D12 = 3,
}

impl Backend {
    /// Array containing all available backend variants.
    /// 
    /// This is useful for iterating over all possible backends or
    /// for validation purposes.
    pub const ALL: [Backend; 4] = [Self::Noop, Self::Vulkan, Self::Metal, Self::D3D12];

    /// The total number of backend variants.
    pub const COUNT: usize = Self::ALL.len();

    /// Returns the string name of the backend.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::Backend;
    /// assert_eq!(Backend::Vulkan.to_str(), "vulkan");
    /// assert_eq!(Backend::Metal.to_str(), "metal");
    /// ```
    #[must_use]
    pub const fn to_str(self) -> &'static str {
        match self {
            Backend::Noop => "noop",
            Backend::Vulkan => "vulkan",
            Backend::Metal => "metal",
            Backend::D3D12 => "d3d12",
        }
    }

    /// Returns whether this backend is available on the current platform.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::Backend;
    /// // On Windows, both D3D12 and Vulkan should be available
    /// # #[cfg(target_os = "windows")]
    /// # {
    /// #     assert!(Backend::D3D12.is_available_on_platform());
    /// #     assert!(Backend::Vulkan.is_available_on_platform());
    /// # }
    /// ```
    #[must_use]
    pub const fn is_available_on_platform(self) -> bool {
        match self {
            Backend::Noop => true, // Always available for testing
            Backend::Vulkan => true, // Available on all platforms via portability layers
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            Backend::Metal => true,
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            Backend::Metal => false,
            #[cfg(target_os = "windows")]
            Backend::D3D12 => true,
            #[cfg(not(target_os = "windows"))]
            Backend::D3D12 => false,
        }
    }

    /// Returns an iterator over all backend variants.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::Backend;
    /// let available_backends: Vec<Backend> = Backend::all_variants()
    ///     .filter(|backend| backend.is_available_on_platform())
    ///     .collect();
    /// ```
    pub fn all_variants() -> impl Iterator<Item = Backend> {
        Self::ALL.iter().copied()
    }
}

impl core::fmt::Display for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.to_str())
    }
}

bitflags::bitflags! {
    /// Bitflags representing a set of graphics API backends.
    /// 
    /// This allows for efficient storage and manipulation of backend combinations,
    /// useful for specifying supported backends or preferences.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Backends: u32 {
        /// No-operation backend flag
        const NOOP = 1 << Backend::Noop as u32;
        /// Vulkan API backend flag
        const VULKAN = 1 << Backend::Vulkan as u32;
        /// Metal API backend flag
        const METAL = 1 << Backend::Metal as u32;
        /// Direct3D-12 backend flag
        const D3D12 = 1 << Backend::D3D12 as u32;
    }
}

impl Default for Backends {
    /// Returns all available backends by default.
    fn default() -> Self {
        Self::all()
    }
}

impl From<Backend> for Backends {
    /// Converts a single backend into a backend set containing only that backend.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::{Backend, Backends};
    /// let backend_set: Backends = Backend::Vulkan.into();
    /// assert!(backend_set.contains(Backends::VULKAN));
    /// assert!(!backend_set.contains(Backends::METAL));
    /// ```
    fn from(backend: Backend) -> Self {
        // This is safe because Backend enum values are designed to work with bit shifts
        Self::from_bits_truncate(1 << backend as u32)
    }
}

impl Backends {
    /// Returns the platform-specific default backend set.
    /// 
    /// This method returns the most appropriate backends for the current platform,
    /// ordered by preference (most preferred first).
    /// 
    /// # Platform defaults
    /// 
    /// - **Windows**: D3D12 and Vulkan
    /// - **macOS/iOS**: Metal and Vulkan (via MoltenVK)
    /// - **Linux**: Vulkan
    /// - **Android**: Vulkan
    /// - **Other platforms**: Vulkan (fallback)
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::Backends;
    /// let defaults = Backends::platform_default();
    /// // On Windows, this would include both D3D12 and Vulkan
    /// ```
    pub fn platform_default() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::D3D12 | Self::VULKAN
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            Self::METAL | Self::VULKAN
        }
        #[cfg(target_os = "linux")]
        {
            Self::VULKAN
        }
        #[cfg(target_os = "android")]
        {
            Self::VULKAN
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "ios",
            target_os = "linux",
            target_os = "android"
        )))]
        {
            Self::VULKAN
        }
    }

    /// Returns an iterator over the individual backends contained in this set.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::{Backend, Backends};
    /// let backends = Backends::VULKAN | Backends::METAL;
    /// let backend_vec: Vec<Backend> = backends.backends_iter().collect();
    /// assert_eq!(backend_vec.len(), 2);
    /// ```
    pub fn backends_iter(self) -> impl Iterator<Item = Backend> {
        Backend::all_variants().filter(move |&backend| self.contains(backend.into()))
    }

    /// Returns the most preferred backend from this set for the current platform.
    /// 
    /// Returns `None` if the set is empty or contains no backends available
    /// on the current platform.
    /// 
    /// # Examples
    /// 
    /// ```
    /// # use moonfield_rhi::{Backend, Backends};
    /// let backends = Backends::platform_default();
    /// if let Some(preferred) = backends.preferred() {
    ///     println!("Using preferred backend: {}", preferred);
    /// }
    /// ```
    pub fn preferred(self) -> Option<Backend> {
        let platform_defaults = Self::platform_default();
        
        // Find the first platform default that's also in our set
        for backend in platform_defaults.backends_iter() {
            if self.contains(backend.into()) {
                return Some(backend);
            }
        }
        
        // Fallback to any available backend
        self.backends_iter().next()
    }
}

// Compile-time assertions to ensure consistency between Backend enum and Backends bitflags
const _: () = {
    // Ensure that Backend::ALL has the correct number of elements
    assert!(Backend::ALL.len() == 4);
    
    // Ensure that each backend maps to the correct bit position
    assert!((1 << Backend::Noop as u32) == Backends::NOOP.bits());
    assert!((1 << Backend::Vulkan as u32) == Backends::VULKAN.bits());
    assert!((1 << Backend::Metal as u32) == Backends::METAL.bits());
    assert!((1 << Backend::D3D12 as u32) == Backends::D3D12.bits());
};

macro_rules! rhi_features {
    ($(($name:ident, $string:literal)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Feature {
            $($name,)*
            _Count,
        }

        impl Feature {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Feature::$name => $string,)*
                    Feature::_Count => "_count",
                }
            }
        }

        impl FromStr for Feature {
            type Err = FeatureParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($string => Ok(Feature::$name),)*
                    "_count" => Ok(Feature::_Count),
                    _ => Err(FeatureParseError),
                }
            }
        }
    };
}

rhi_features! {
    (HardwareDevice, "hardware-device"),
    (SoftwareDevice, "software-device"),
    (ParameterBlock, "parameter-block"),
    (Bindless, "bindless"),
    (Surface, "surface"),
    (PipelineCache, "pipeline-cache"),
    // Rasterization features
    (Rasterization, "rasterization"),
    (Barycentrics, "barycentrics"),
    (MultiView, "multi-view"),
    (RasterizerOrderedViews, "rasterizer-ordered-views"),
    (ConservativeRasterization, "conservative-rasterization"),
    (CustomBorderColor, "custom-border-color"),
    (FragmentShadingRate, "fragment-shading-rate"),
    (SamplerFeedback, "sampler-feedback"),
    // Ray tracing features
    (AccelerationStructure, "acceleration-structure"),
    (AccelerationStructureSpheres, "acceleration-structure-spheres"),
    (AccelerationStructureLinearSweptSpheres, "acceleration-structure-linear-swept-spheres"),
    (RayTracing, "ray-tracing"),
    (RayQuery, "ray-query"),
    (ShaderExecutionReordering, "shader-execution-reordering"),
    (RayTracingValidation, "ray-tracing-validation"),
    // Other features
    (TimestampQuery, "timestamp-query"),
    (RealtimeClock, "realtime-clock"),
    (CooperativeVector, "cooperative-vector"),
    (CooperativeMatrix, "cooperative-matrix"),
    (Sm5_1, "sm_5_1"),
    (Sm6_0, "sm_6_0"),
    (Sm6_1, "sm_6_1"),
    (Sm6_2, "sm_6_2"),
    (Sm6_3, "sm_6_3"),
    (Sm6_4, "sm_6_4"),
    (Sm6_5, "sm_6_5"),
    (Sm6_6, "sm_6_6"),
    (Sm6_7, "sm_6_7"),
    (Sm6_8, "sm_6_8"),
    (Sm6_9, "sm_6_9"),
    (Half, "half"),
    (Double, "double"),
    (Int16, "int16"),
    (Int64, "int64"),
    (AtomicFloat, "atomic-float"),
    (AtomicHalf, "atomic-half"),
    (AtomicInt64, "atomic-int64"),
    (WaveOps, "wave-ops"),
    (MeshShader, "mesh-shader"),
    (Pointer, "has-ptr"),
    // D3D12 specific features
    (ConservativeRasterization1, "conservative-rasterization-1"),
    (ConservativeRasterization2, "conservative-rasterization-2"),
    (ConservativeRasterization3, "conservative-rasterization-3"),
    (ProgrammableSamplePositions1, "programmable-sample-positions-1"),
    (ProgrammableSamplePositions2, "programmable-sample-positions-2"),
    // Vulkan specific features
    (ShaderResourceMinLod, "shader-resource-min-lod"),
    // Metal specific features
    (ArgumentBufferTier2, "argument-buffer-tier-2"),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessFlag {
    None,
    Read,
    Write,
}

/// Defines how linking should be performed for a shader program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkingStyle {
    /// Compose all entry-points in a single program, then compile all entry-points together with the same set of root shader arguments.
    SingleProgram,

    /// Link and compile each entry-point individually
    SeparateEntryPointCompilation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    // D3D formats omitted: 19-22, 44-47, 65-66, 68-70, 73, 76, 79, 82, 88-89, 92-94, 97, 100-114
    // These formats are omitted due to lack of a corresponding Vulkan format. D24_UNORM_S8_UINT (DXGI_FORMAT 45)
    // has a matching Vulkan format but is also omitted as it is only supported by Nvidia.
    Undefined,

    R8Uint,
    R8Sint,
    R8Unorm,
    R8Snorm,

    RG8Uint,
    RG8Sint,
    RG8Unorm,
    RG8Snorm,

    RGBA8Uint,
    RGBA8Sint,
    RGBA8Unorm,
    RGBA8UnormSrgb,
    RGBA8Snorm,

    BGRA8Unorm,
    BGRA8UnormSrgb,
    BGRX8Unorm,
    BGRX8UnormSrgb,

    R16Uint,
    R16Sint,
    R16Unorm,
    R16Snorm,
    R16Float,

    RG16Uint,
    RG16Sint,
    RG16Unorm,
    RG16Snorm,
    RG16Float,

    RGBA16Uint,
    RGBA16Sint,
    RGBA16Unorm,
    RGBA16Snorm,
    RGBA16Float,

    R32Uint,
    R32Sint,
    R32Float,

    RG32Uint,
    RG32Sint,
    RG32Float,

    RGB32Uint,
    RGB32Sint,
    RGB32Float,

    RGBA32Uint,
    RGBA32Sint,
    RGBA32Float,

    R64Uint,
    R64Sint,

    BGRA4Unorm,
    B5G6R5Unorm,
    BGR5A1Unorm,

    RGB9E5Ufloat,
    RGB10A2Uint,
    RGB10A2Unorm,
    R11G11B10Float,

    // Depth/stencil formats
    D32Float,
    D16Unorm,
    D32FloatS8Uint,

    // Compressed formats
    BC1Unorm,
    BC1UnormSrgb,
    BC2Unorm,
    BC2UnormSrgb,
    BC3Unorm,
    BC3UnormSrgb,
    BC4Unorm,
    BC4Snorm,
    BC5Unorm,
    BC5Snorm,
    BC6HUfloat,
    BC6HSfloat,
    BC7Unorm,
    BC7UnormSrgb,

    _Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatKind {
    Integer,
    Normalized,
    Float,
    DepthStencil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexFormat {
    Uint16,
    Uint32,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FormatSupport: u32 {
        const NONE = 0x0;

        const COPY_SOURCE = 0x1;
        const COPY_DESTINATION = 0x2;

        const TEXTURE = 0x4;
        const DEPTH_STENCIL = 0x8;
        const RENDER_TARGET = 0x10;
        const BLENDABLE = 0x20;
        const MULTISAMPLING = 0x40;
        const RESOLVABLE = 0x80;

        const SHADER_LOAD = 0x100;
        const SHADER_SAMPLE = 0x200;
        const SHADER_UAV_LOAD = 0x400;
        const SHADER_UAV_STORE = 0x800;
        const SHADER_ATOMIC = 0x1000;

        const BUFFER = 0x2000;
        const INDEX_BUFFER = 0x4000;
        const VERTEX_BUFFER = 0x8000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSlotClass {
    PerVertex,
    PerInstance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    PointList,
    LineList,
    LineStrip,
    TriangleList,
    TriangleStrip,
    PatchList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceState {
    Undefined,
    General,
    VertexBuffer,
    IndexBuffer,
    ConstantBuffer,
    StreamOutput,
    ShaderResource,
    UnorderedAccess,
    RenderTarget,
    DepthRead,
    DepthWrite,
    Present,
    IndirectArgument,
    CopySource,
    CopyDestination,
    ResolveSource,
    ResolveDestination,
    AccelerationStructure,
    AccelerationStructureBuildInput,
}

/// Describes how memory for the resource should be allocated for CPU access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryType {
    DeviceLocal,
    Upload,
    ReadBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum NativeHandleType {
    #[default]
    Undefined = 0x00000000,

    Win32 = 0x00000001,
    FileDescriptor = 0x00000002,

    D3D12Device = 0x00020001,
    D3D12CommandQueue = 0x00020002,
    D3D12GraphicsCommandList = 0x00020003,
    D3D12Resource = 0x00020004,
    D3D12PipelineState = 0x00020005,
    D3D12StateObject = 0x00020006,
    D3D12CpuDescriptorHandle = 0x00020007,
    D3D12Fence = 0x00020008,
    D3D12DeviceAddress = 0x00020009,

    VkDevice = 0x00030001,
    VkPhysicalDevice = 0x00030002,
    VkInstance = 0x00030003,
    VkQueue = 0x00030004,
    VkCommandBuffer = 0x00030005,
    VkBuffer = 0x00030006,
    VkImage = 0x00030007,
    VkImageView = 0x00030008,
    VkAccelerationStructureKHR = 0x00030009,
    VkSampler = 0x0003000a,
    VkPipeline = 0x0003000b,
    VkSemaphore = 0x0003000c,

    MTLDevice = 0x00040001,
    MTLCommandQueue = 0x00040002,
    MTLCommandBuffer = 0x00040003,
    MTLTexture = 0x00040004,
    MTLBuffer = 0x00040005,
    MTLComputePipelineState = 0x00040006,
    MTLRenderPipelineState = 0x00040007,
    MTLSharedEvent = 0x00040008,
    MTLSamplerState = 0x00040009,
    MTLAccelerationStructure = 0x0004000a,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DescriptorHandleType {
    #[default]
    Undefined,
    Buffer,
    RWBuffer,
    Texture,
    RWTexture,
    Sampler,
    AccelerationStructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescriptorHandleAccess {
    Read,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuAccessMode {
    Read,
    Write,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BufferUsage: u32 {
        const NONE = 0;
        const VERTEX_BUFFER = 1 << 0;
        const INDEX_BUFFER = 1 << 1;
        const CONSTANT_BUFFER = 1 << 2;
        const SHADER_RESOURCE = 1 << 3;
        const UNORDERED_ACCESS = 1 << 4;
        const INDIRECT_ARGUMENT = 1 << 5;
        const COPY_SOURCE = 1 << 6;
        const COPY_DESTINATION = 1 << 7;
        const ACCELERATION_STRUCTURE = 1 << 8;
        const ACCELERATION_STRUCTURE_BUILD_INPUT = 1 << 9;
        const SHADER_TABLE = 1 << 10;
        const SHARED = 1 << 11;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TextureUsage: u32 {
        const NONE = 0;
        const SHADER_RESOURCE = 1 << 0;
        const UNORDERED_ACCESS = 1 << 1;
        const RENDER_TARGET = 1 << 2;
        const DEPTH_STENCIL = 1 << 3;
        const PRESENT = 1 << 4;
        const COPY_SOURCE = 1 << 5;
        const COPY_DESTINATION = 1 << 6;
        const RESOLVE_SOURCE = 1 << 7;
        const RESOLVE_DESTINATION = 1 << 8;
        const TYPELESS = 1 << 9;
        const SHARED = 1 << 10;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureType {
    Texture1D,
    Texture1DArray,
    Texture2D,
    Texture2DArray,
    Texture2DMS,
    Texture2DMSArray,
    Texture3D,
    TextureCube,
    TextureCubeArray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureDimension {
    Texture1D,
    Texture2D,
    Texture3D,
    TextureCube,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TextureAspect {
    All = 0,
    DepthOnly = 1,
    StencilOnly = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ComparisonFunc {
    Never = 0,
    Less = 1,
    Equal = 2,
    LessEqual = 3,
    Greater = 4,
    NotEqual = 5,
    GreaterEqual = 6,
    Always = 7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFilteringMode {
    Point,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureAddressingMode {
    Wrap,
    ClampToEdge,
    ClampToBorder,
    MirrorRepeat,
    MirrorOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureReductionOp {
    Average,
    Comparison,
    Minimum,
    Maximum,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AccelerationStructureGeometryFlags: u32 {
        const NONE = 0;
        const OPAQUE = 1 << 0;
        const NO_DUPLICATE_ANY_HIT_INVOCATION = 1 << 1;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AccelerationStructureInstanceFlags: u32 {
        const NONE = 0;
        const TRIANGLE_FACING_CULL_DISABLE = 1 << 0;
        const TRIANGLE_FRONT_COUNTER_CLOCKWISE = 1 << 1;
        const FORCE_OPAQUE = 1 << 2;
        const NO_OPAQUE = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccelerationStructureInstanceDescType {
    Generic,
    D3D12,
    Vulkan,
    Metal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccelerationStructureBuildInputType {
    Instances,
    Triangles,
    ProceduralPrimitives,
    Spheres,
    LinearSweptSpheres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinearSweptSpheresIndexingMode {
    List,
    Successive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinearSweptSpheresEndCapsMode {
    None,
    Chained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccelerationStructureBuildMode {
    Build,
    Update,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AccelerationStructureBuildFlags: u32 {
        const NONE = 0;
        const ALLOW_UPDATE = 1 << 0;
        const ALLOW_COMPACTION = 1 << 1;
        const PREFER_FAST_TRACE = 1 << 2;
        const PREFER_FAST_BUILD = 1 << 3;
        const MINIMIZE_MEMORY = 1 << 4;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderObjectContainerType {
    None,
    Array,
    StructuredBuffer,
    ParameterBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingType {
    Undefined,
    Buffer,
    BufferWithCounter,
    Texture,
    Sampler,
    CombinedTextureSampler,
    AccelerationStructure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StencilOp {
    Keep,
    Zero,
    Replace,
    IncrementSaturate,
    DecrementSaturate,
    Invert,
    IncrementWrap,
    DecrementWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FillMode {
    Solid,
    Wireframe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CullMode {
    None,
    Front,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrontFaceMode {
    CounterClockwise,
    Clockwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogicOp {
    NoOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendOp {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendFactor {
    Zero,
    One,
    SrcColor,
    InvSrcColor,
    SrcAlpha,
    InvSrcAlpha,
    DestAlpha,
    InvDestAlpha,
    DestColor,
    InvDestColor,
    SrcAlphaSaturate,
    BlendColor,
    InvBlendColor,
    SecondarySrcColor,
    InvSecondarySrcColor,
    SecondarySrcAlpha,
    InvSecondarySrcAlpha,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RenderTargetWriteMask: u8 {
        const NONE = 0;
        const RED = 0x01;
        const GREEN = 0x02;
        const BLUE = 0x04;
        const ALPHA = 0x08;
        const ALL = 0x0F;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct RayTracingPipelineFlags: u32 {
        const NONE = 0;
        const SKIP_TRIANGLES = 1 << 0;
        const SKIP_PROCEDURALS = 1 << 1;
        const ENABLE_SPHERES = 1 << 2;
        const ENABLE_LINEAR_SWEPT_SPHERES = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowHandleType {
    #[default]
    Undefined,
    HWND,
    NSWindow,
    XlibWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadOp {
    Load,
    Clear,
    DontCare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreOp {
    Store,
    DontCare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueryType {
    Timestamp,
    AccelerationStructureCompactedSize,
    AccelerationStructureSerializedSize,
    AccelerationStructureCurrentSize,
}

/// Specifies how acceleration structure copying should be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccelerationStructureCopyMode {
    /// Create an exact copy of the acceleration structure.
    Clone,
    /// Create a compacted copy of the acceleration structure.
    Compact,
}

/// Component types for cooperative vector operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CooperativeVectorComponentType {
    Float16 = 0,
    Float32 = 1,
    Float64 = 2,
    Sint8 = 3,
    Sint16 = 4,
    Sint32 = 5,
    Sint64 = 6,
    Uint8 = 7,
    Uint16 = 8,
    Uint32 = 9,
    Uint64 = 10,
    Sint8Packed = 11,
    Uint8Packed = 12,
    FloatE4M3 = 13,
    FloatE5M2 = 14,
}

/// Matrix layout for cooperative vector operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CooperativeVectorMatrixLayout {
    RowMajor = 0,
    ColumnMajor = 1,
    InferencingOptimal = 2,
    TrainingOptimal = 3,
}

bitflags! {
    /// Usage flags for memory heaps.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct HeapUsage: u32 {
        const NONE = 0;
        const SHARED = 1 << 0;
    }
}

/// Types of debug messages that can be generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugMessageType {
    Info,
    Warning,
    Error,
}

/// Sources of debug messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebugMessageSource {
    /// RHI layer is the source of the message
    Layer,
    /// Graphics driver is the source of the message
    Driver,
    /// Slang compiler is the source of the message
    Slang,
}
