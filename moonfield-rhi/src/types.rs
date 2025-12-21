use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    Metal,
}

#[derive(Debug, thiserror::Error)]
pub enum RhiError {
    #[error("Initialization failed: {0}")]
    InitializationFailed(String),
    #[error("Backend not supported")]
    BackendNotSupported,
    #[error("Device creation failed")]
    DeviceCreationFailed,
    #[error("Swapchain creation failed")]
    SwapchainCreationFailed,
    #[error("Shader compilation failed: {0}")]
    ShaderCompilationFailed(String),
    #[error("Pipeline creation failed")]
    PipelineCreationFailed,
    #[error("Buffer creation failed")]
    BufferCreationFailed,
    #[error("Command pool creation failed")]
    CommandPoolCreationFailed,
    #[error("Command buffer allocation failed")]
    CommandBufferAllocationFailed,
    #[error("Acquire image failed")]
    AcquireImageFailed,
    #[error("Present failed")]
    PresentFailed,
    #[error("Submit failed")]
    SubmitFailed,
    #[error("Map failed")]
    MapFailed,
}

#[derive(Debug, Clone)]
pub struct AdapterProperties {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    B8G8R8A8Unorm,
    R8G8B8A8Unorm,
    B8G8R8A8Srgb,
    R8G8B8A8Srgb,
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
    pub stage: ShaderStage,
}

pub struct GraphicsPipelineDescriptor {
    pub vertex_shader: Arc<dyn crate::ShaderModule>,
    pub fragment_shader: Arc<dyn crate::ShaderModule>,
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