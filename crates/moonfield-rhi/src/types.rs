use std::sync::Arc;

use shader_slang::Stage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    Metal,
    Dx12,
}

#[derive(Debug, thiserror::Error)]
pub enum RhiError {
    #[error("Initialization failed: {0}")]
    InitializationFailed(String),
    #[error("Backend not supported")]
    BackendNotSupported,
    #[error("Device creation failed: {0}")]
    DeviceCreationFailed(String),
    #[error("Swapchain creation failed: {0}")]
    SwapchainCreationFailed(String),
    #[error("Shader compilation failed: {0}")]
    ShaderCompilationFailed(#[from] ShaderCompilationError),
    #[error("Pipeline creation failed: {0}")]
    PipelineCreationFailed(String),
    #[error("Buffer creation failed: {0}")]
    BufferCreationFailed(String),
    #[error("Command pool creation failed: {0}")]
    CommandPoolCreationFailed(String),
    #[error("Command buffer allocation failed: {0}")]
    CommandBufferAllocationFailed(String),
    #[error("Acquire image failed: {0}")]
    AcquireImageFailed(String),
    #[error("Present failed: {0}")]
    PresentFailed(String),
    #[error("Submit failed: {0}")]
    SubmitFailed(String),
    #[error("Map failed: {0}")]
    MapFailed(String),
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Resource already exists: {0}")]
    ResourceAlreadyExists(String),
    #[error("Invalid resource state: {0}")]
    InvalidResourceState(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Out of memory: {0}")]
    OutOfMemory(String),
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Driver error: {0}")]
    DriverError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ShaderCompilationError {
    #[error("Invalid shader code: {0}")]
    InvalidShaderCode(String),
    #[error("Unsupported shader stage: {0}")]
    UnsupportedShaderStage(String),
    #[error("Compilation error: {0}")]
    CompilationError(String),
}

impl From<String> for RhiError {
    fn from(s: String) -> Self {
        RhiError::InitializationFailed(s)
    }
}

impl From<&str> for RhiError {
    fn from(s: &str) -> Self {
        RhiError::InitializationFailed(s.to_string())
    }
}

impl From<std::io::Error> for RhiError {
    fn from(err: std::io::Error) -> Self {
        RhiError::InitializationFailed(err.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct AdapterProperties {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
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
}

#[derive(Debug, Clone, Copy)]
pub struct Extent2D {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct SurfaceCapabilities {
    pub formats: Vec<Format>,
    pub present_modes: Vec<PresentMode>,
    pub min_image_count: u32,
    pub max_image_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentMode {
    Immediate,
    Mailbox,
    Fifo,
}

pub struct SwapchainDescriptor {
    pub surface: Arc<dyn crate::Surface>,
    pub format: Format,
    pub extent: Extent2D,
    pub present_mode: PresentMode,
    pub image_count: u32,
}

pub struct SwapchainImage {
    pub index: u32,
    pub(crate) image_view: usize,
    pub wait_semaphore: u64,
    pub signal_semaphore: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage {
    Vertex,
    Fragment,
}

pub struct ShaderModuleDescriptor<'a> {
    pub code: &'a [u8],
    pub stage: Stage,
}

pub struct GraphicsPipelineDescriptor {
    pub vertex_shader: Arc<dyn crate::ShaderProgram>,
    pub fragment_shader: Arc<dyn crate::ShaderProgram>,
    pub vertex_input: VertexInputDescriptor,
    pub render_pass_format: Format,
}

#[derive(Default)]
pub struct VertexInputDescriptor {
    pub bindings: Vec<VertexInputBinding>,
    pub attributes: Vec<VertexInputAttribute>,
}

pub struct VertexInputBinding {
    pub binding: u32,
    pub stride: u32,
    pub input_rate: VertexInputRate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexInputRate {
    Vertex,
    Instance,
}

pub struct VertexInputAttribute {
    pub location: u32,
    pub binding: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexFormat {
    Float32x2,
    Float32x3,
    Float32x4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferUsage {
    Vertex,
    Index,
    Uniform,
}

pub struct BufferDescriptor {
    pub size: u64,
    pub usage: BufferUsage,
    pub memory_location: MemoryLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLocation {
    GpuOnly,
    CpuToGpu,
    GpuToCpu,
}

pub struct RenderPassDescriptor {
    pub color_attachments: Vec<ColorAttachment>,
}

pub struct ColorAttachment {
    pub load_op: LoadOp,
    pub store_op: StoreOp,
    pub clear_value: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    Load,
    Clear,
    DontCare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    Store,
    DontCare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    Undefined,
    Buffer,
    BufferWithCounter,
    Texture,
    Sampler,
    CombinedTextureSampler,
    AccelerationStructure,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rhi_error_variants() {
        // Test that all error variants can be created
        let _err1 = RhiError::InitializationFailed("test".to_string());
        let _err2 = RhiError::DeviceCreationFailed("test".to_string());
        let _err3 = RhiError::SwapchainCreationFailed("test".to_string());
        let _err4 = RhiError::ShaderCompilationFailed(
            ShaderCompilationError::InvalidShaderCode("test".to_string()),
        );
        let _err5 = RhiError::ResourceNotFound("test".to_string());
        let _err6 = RhiError::OutOfMemory("test".to_string());

        assert!(true); // Simple assertion to confirm test runs
    }

    #[test]
    fn test_error_conversions() {
        // Test that string conversions work
        let err_from_str: RhiError = "test error".into();
        let err_from_string: RhiError = "test error".to_string().into();

        match err_from_str {
            RhiError::InitializationFailed(_) => {}
            _ => panic!("Expected InitializationFailed variant"),
        }

        match err_from_string {
            RhiError::InitializationFailed(_) => {}
            _ => panic!("Expected InitializationFailed variant"),
        }
    }
}
