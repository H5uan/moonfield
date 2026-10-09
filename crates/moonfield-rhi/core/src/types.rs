//! Public resource descriptions shared by Moonfield's renderers.
//!
//! The descriptions remain independent of raw backend handles so higher-level
//! renderer code does not need to construct backend types directly.

/// Pixel/color formats supported by the engine. Grow as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Format {
    /// 8-bit BGRA unorm; the preferred swapchain and offscreen format.
    B8G8R8A8Unorm,
    /// 8-bit RGBA unorm.
    R8G8B8A8Unorm,
    /// 16-bit-per-channel RGBA float; the gaussian-splatting intermediate
    /// (compute-written storage image, sampled by the composite pass).
    R16G16B16A16Sfloat,
    /// 32-bit float depth (D32_SFLOAT), used for the engine's reverse-Z depth
    /// attachments.
    D32Sfloat,
}

impl Format {
    /// Bytes per pixel of a tightly packed row.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::B8G8R8A8Unorm | Self::R8G8B8A8Unorm | Self::D32Sfloat => 4,
            Self::R16G16B16A16Sfloat => 8,
        }
    }
}

// ===========================================================================
// Pass-recording vocabulary
//
// The types below are the crate's own vocabulary for recording render passes,
// so feature crates (meshes, UI) never construct backend types. Each backend
// maps them onto its own concepts (the Vulkan backend's mappings live in
// `moonfield-rhi-vulkan/src/formats.rs`).
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

/// The winding order considered front-facing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontFace {
    /// Clockwise (pairs with the engine's Y-flip viewport).
    Clockwise,
    /// Counter-clockwise.
    CounterClockwise,
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

/// The image layout an attachment is in during a pass (and stays in — the
/// engine does not transition layouts across passes yet).
///
/// Non-swapchain attachments map to `GENERAL` under the unified-layout
/// policy (see `docs/architecture.md`). When the optional
/// `VK_KHR_unified_image_layouts` device extension is enabled, `GENERAL` is
/// additionally a spec-guaranteed-optimal layout; the extension does not
/// change this mapping, it blesses it. `Present` keeps `PRESENT_SRC_KHR`
/// because presentation is explicitly exempt from the extension — the
/// swapchain already performs its one transition per frame, and on the
/// desktop targets (Windows/Linux) drivers handle `PRESENT_SRC_KHR` at full
/// performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentLayout {
    /// A swapchain image that remains presentable.
    Present,
    /// An offscreen target that remains sampleable in shaders.
    ShaderRead,
    /// A depth/stencil attachment.
    DepthStencil,
}

/// Command buffer usage flags. Const-fn combinable, no external deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandBufferUsage(u32);

impl CommandBufferUsage {
    /// The buffer is submitted once and re-recorded.
    pub const ONE_TIME_SUBMIT: Self = Self(1);

    /// Whether `other`'s bits are set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
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

/// Sampler creation parameters.
///
/// The configuration space is closed and small (36 combinations) — the
/// descriptor heap's sampler cache keys on this; see the note
/// `2026-09-04-frame-boundary-heap-bind-and-sampler-cache`.
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
