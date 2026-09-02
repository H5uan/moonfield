//! Public resource descriptions shared by Moonfield's Vulkan renderer.
//!
//! The descriptions remain independent of raw Vulkan handles so higher-level
//! renderer code does not need to construct ash types directly.

/// Pixel/color formats supported by the engine. Grow as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 8-bit BGRA unorm; the preferred swapchain and offscreen format.
    B8G8R8A8Unorm,
    /// 8-bit RGBA unorm.
    R8G8B8A8Unorm,
    /// 32-bit float depth (D32_SFLOAT), used for the engine's reverse-Z depth
    /// attachments.
    D32Sfloat,
}

impl Format {
    /// Convert to the equivalent Vulkan format.
    pub(crate) fn to_vk(self) -> ash::vk::Format {
        match self {
            Self::B8G8R8A8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
            Self::R8G8B8A8Unorm => ash::vk::Format::R8G8B8A8_UNORM,
            Self::D32Sfloat => ash::vk::Format::D32_SFLOAT,
        }
    }
}

/// Buffer usage flags. Const-fn combinable, no external deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferUsage(u32);

impl BufferUsage {
    /// The buffer can be bound as a vertex buffer.
    pub const VERTEX: Self = Self(1 << 0);
    /// The buffer can be bound as an index buffer.
    pub const INDEX: Self = Self(1 << 1);
    /// The buffer can be bound as a uniform buffer.
    pub const UNIFORM: Self = Self(1 << 2);
    /// The buffer can be bound as a storage buffer.
    pub const STORAGE: Self = Self(1 << 3);
    /// The buffer can be the destination of a copy.
    pub const COPY_DST: Self = Self(1 << 4);
    /// The buffer can be the source of a copy.
    pub const COPY_SRC: Self = Self(1 << 5);
    /// The buffer can back an indirect draw/dispatch command.
    pub const INDIRECT: Self = Self(1 << 6);

    /// An empty set of usage flags.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The union of two usage sets.
    pub const fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether all flags in `other` are set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for BufferUsage {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.bitor(rhs)
    }
}

impl BufferUsage {
    /// Convert to the equivalent Vulkan usage flags.
    pub(crate) fn to_vk(self) -> ash::vk::BufferUsageFlags {
        let mut flags = ash::vk::BufferUsageFlags::empty();
        if self.contains(Self::VERTEX) {
            flags |= ash::vk::BufferUsageFlags::VERTEX_BUFFER;
        }
        if self.contains(Self::INDEX) {
            flags |= ash::vk::BufferUsageFlags::INDEX_BUFFER;
        }
        if self.contains(Self::UNIFORM) {
            flags |= ash::vk::BufferUsageFlags::UNIFORM_BUFFER;
        }
        if self.contains(Self::STORAGE) {
            flags |= ash::vk::BufferUsageFlags::STORAGE_BUFFER;
        }
        if self.contains(Self::COPY_DST) {
            flags |= ash::vk::BufferUsageFlags::TRANSFER_DST;
        }
        if self.contains(Self::COPY_SRC) {
            flags |= ash::vk::BufferUsageFlags::TRANSFER_SRC;
        }
        if self.contains(Self::INDIRECT) {
            flags |= ash::vk::BufferUsageFlags::INDIRECT_BUFFER;
        }
        flags
    }
}

/// Vertex attribute formats supported by the Vulkan renderer. Grow as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexFormat {
    /// Two 32-bit floats.
    Float32x2,
    /// Three 32-bit floats.
    Float32x3,
    /// Four 32-bit floats.
    Float32x4,
    /// One 32-bit unsigned integer (e.g. a packed RGBA color).
    Uint32,
}

impl VertexFormat {
    /// Convert to the equivalent Vulkan format.
    pub(crate) fn to_vk(self) -> ash::vk::Format {
        match self {
            Self::Float32x2 => ash::vk::Format::R32G32_SFLOAT,
            Self::Float32x3 => ash::vk::Format::R32G32B32_SFLOAT,
            Self::Float32x4 => ash::vk::Format::R32G32B32A32_SFLOAT,
            Self::Uint32 => ash::vk::Format::R32_UINT,
        }
    }
}

/// A single vertex attribute within a [`VertexBufferLayout`].
#[derive(Debug, Clone, Copy)]
pub struct VertexAttribute {
    /// The shader input location.
    pub location: u32,
    /// The attribute format.
    pub format: VertexFormat,
    /// Byte offset of the attribute within a vertex.
    pub offset: u32,
}

/// The layout of a single vertex buffer (binding 0, per-vertex input rate).
#[derive(Debug, Clone)]
pub struct VertexBufferLayout {
    /// Byte stride between consecutive vertices.
    pub stride: u32,
    /// The vertex attributes.
    pub attributes: Vec<VertexAttribute>,
}

// ===========================================================================
// Pass-recording vocabulary
//
// The types below are the crate's own vocabulary for recording render passes,
// so feature crates (meshes, UI) never construct raw `ash` types. Each maps
// onto exactly one Vulkan concept via a `pub(crate) to_vk`.
// ===========================================================================

/// A 2D extent in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extent2d {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl From<(u32, u32)> for Extent2d {
    fn from((width, height): (u32, u32)) -> Self {
        Self { width, height }
    }
}

impl Extent2d {
    pub(crate) fn to_vk(self) -> ash::vk::Extent2D {
        ash::vk::Extent2D {
            width: self.width,
            height: self.height,
        }
    }
}

/// A 2D offset in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Offset2d {
    /// Horizontal offset.
    pub x: i32,
    /// Vertical offset.
    pub y: i32,
}

/// An axis-aligned rectangle in pixels (render areas, scissor rects).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect2d {
    /// Top-left corner.
    pub offset: Offset2d,
    /// Size.
    pub extent: Extent2d,
}

impl Rect2d {
    /// A rectangle covering a full target of the given size.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            offset: Offset2d::default(),
            extent: Extent2d { width, height },
        }
    }

    pub(crate) fn to_vk(self) -> ash::vk::Rect2D {
        ash::vk::Rect2D {
            offset: ash::vk::Offset2D {
                x: self.offset.x,
                y: self.offset.y,
            },
            extent: self.extent.to_vk(),
        }
    }
}

/// A viewport rectangle in framebuffer coordinates, with depth range.
///
/// A negative `height` maps the engine's Y-up NDC convention onto Vulkan's
/// top-left framebuffer origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width (positive).
    pub width: f32,
    /// Height; negative flips Y.
    pub height: f32,
    /// Minimum depth.
    pub min_depth: f32,
    /// Maximum depth.
    pub max_depth: f32,
}

impl Viewport {
    /// A viewport covering `width`×`height`, with a negative height for the
    /// engine's Y-up clip convention.
    pub fn y_flipped(width: u32, height: u32) -> Self {
        Self {
            x: 0.0,
            y: height as f32,
            width: width as f32,
            height: -(height as f32),
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }

    pub(crate) fn to_vk(self) -> ash::vk::Viewport {
        ash::vk::Viewport {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            min_depth: self.min_depth,
            max_depth: self.max_depth,
        }
    }
}

/// Depth/stencil comparison function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Never passes.
    Never,
    /// Passes when less.
    Less,
    /// Passes when equal.
    Equal,
    /// Passes when less or equal.
    LessOrEqual,
    /// Passes when greater.
    Greater,
    /// Passes when not equal.
    NotEqual,
    /// Passes when greater or equal (the engine's reverse-Z direction).
    GreaterOrEqual,
    /// Always passes.
    Always,
}

impl CompareOp {
    pub(crate) fn to_vk(self) -> ash::vk::CompareOp {
        match self {
            Self::Never => ash::vk::CompareOp::NEVER,
            Self::Less => ash::vk::CompareOp::LESS,
            Self::Equal => ash::vk::CompareOp::EQUAL,
            Self::LessOrEqual => ash::vk::CompareOp::LESS_OR_EQUAL,
            Self::Greater => ash::vk::CompareOp::GREATER,
            Self::NotEqual => ash::vk::CompareOp::NOT_EQUAL,
            Self::GreaterOrEqual => ash::vk::CompareOp::GREATER_OR_EQUAL,
            Self::Always => ash::vk::CompareOp::ALWAYS,
        }
    }
}

/// Triangle culling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullMode {
    /// No culling.
    None,
    /// Cull front faces.
    Front,
    /// Cull back faces.
    Back,
}

impl CullMode {
    pub(crate) fn to_vk(self) -> ash::vk::CullModeFlags {
        match self {
            Self::None => ash::vk::CullModeFlags::NONE,
            Self::Front => ash::vk::CullModeFlags::FRONT,
            Self::Back => ash::vk::CullModeFlags::BACK,
        }
    }
}

/// The winding order considered front-facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontFace {
    /// Clockwise (pairs with the engine's Y-flip viewport).
    Clockwise,
    /// Counter-clockwise.
    CounterClockwise,
}

impl FrontFace {
    pub(crate) fn to_vk(self) -> ash::vk::FrontFace {
        match self {
            Self::Clockwise => ash::vk::FrontFace::CLOCKWISE,
            Self::CounterClockwise => ash::vk::FrontFace::COUNTER_CLOCKWISE,
        }
    }
}

/// What to do with an attachment's contents when a pass begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    /// Preserve existing contents.
    Load,
    /// Clear to the attachment's clear value.
    Clear,
}

/// What to do with an attachment's contents when a pass ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    /// Store the rendered contents.
    Store,
    /// Contents are not needed after the pass.
    Discard,
}

/// The clear value of an attachment with [`LoadOp::Clear`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClearValue {
    /// Color attachment clear (linear float RGBA).
    Color([f32; 4]),
    /// Depth/stencil attachment clear (reverse-Z depth clears to 0.0).
    DepthStencil {
        /// Depth value.
        depth: f32,
        /// Stencil value.
        stencil: u32,
    },
}

impl ClearValue {
    pub(crate) fn to_vk(self) -> ash::vk::ClearValue {
        match self {
            Self::Color(float32) => ash::vk::ClearValue {
                color: ash::vk::ClearColorValue { float32 },
            },
            Self::DepthStencil { depth, stencil } => ash::vk::ClearValue {
                depth_stencil: ash::vk::ClearDepthStencilValue { depth, stencil },
            },
        }
    }
}

/// The image layout an attachment is in during a pass (and stays in — the
/// engine does not transition layouts across passes yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentLayout {
    /// A swapchain image that remains presentable.
    Present,
    /// An offscreen target that remains sampleable in shaders.
    ShaderRead,
    /// A depth/stencil attachment.
    DepthStencil,
}

impl AttachmentLayout {
    pub(crate) fn to_vk(self) -> ash::vk::ImageLayout {
        match self {
            Self::Present => ash::vk::ImageLayout::PRESENT_SRC_KHR, // still need this layout
            Self::ShaderRead | Self::DepthStencil => ash::vk::ImageLayout::GENERAL,
        }
    }
}

/// Command buffer usage flags. Const-fn combinable, no external deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandBufferUsage(u32);

impl CommandBufferUsage {
    /// The buffer is submitted once and re-recorded.
    pub const ONE_TIME_SUBMIT: Self = Self(1);

    pub(crate) fn to_vk(self) -> ash::vk::CommandBufferUsageFlags {
        let mut flags = ash::vk::CommandBufferUsageFlags::empty();
        if self.0 & Self::ONE_TIME_SUBMIT.0 != 0 {
            flags |= ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT;
        }
        flags
    }
}

/// Texture filtering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Nearest-neighbor sampling.
    Nearest,
    /// Linear interpolation.
    Linear,
}

impl Filter {
    pub(crate) fn to_vk(self) -> ash::vk::Filter {
        match self {
            Self::Nearest => ash::vk::Filter::NEAREST,
            Self::Linear => ash::vk::Filter::LINEAR,
        }
    }
}

/// Texture wrap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrapMode {
    /// Clamp coordinates to the edge texel.
    ClampToEdge,
    /// Repeat the texture.
    Repeat,
    /// Repeat, mirroring every other tile.
    MirroredRepeat,
}

impl WrapMode {
    pub(crate) fn to_vk(self) -> ash::vk::SamplerAddressMode {
        match self {
            Self::ClampToEdge => ash::vk::SamplerAddressMode::CLAMP_TO_EDGE,
            Self::Repeat => ash::vk::SamplerAddressMode::REPEAT,
            Self::MirroredRepeat => ash::vk::SamplerAddressMode::MIRRORED_REPEAT,
        }
    }
}

/// Sampler creation parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    /// Minification filter.
    pub min_filter: Filter,
    /// Magnification filter.
    pub mag_filter: Filter,
    /// Mipmap filter; `None` (or `Nearest`) selects nearest mip sampling.
    /// Textures are single-mip today, so this only selects the enum.
    pub mipmap_filter: Option<Filter>,
    /// Wrap mode for all axes.
    pub wrap: WrapMode,
}

impl Default for SamplerDesc {
    fn default() -> Self {
        Self {
            min_filter: Filter::Linear,
            mag_filter: Filter::Linear,
            mipmap_filter: Some(Filter::Linear),
            wrap: WrapMode::ClampToEdge,
        }
    }
}
