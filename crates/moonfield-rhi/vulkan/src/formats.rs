//! Vulkan conversions for the shared RHI vocabulary.
//!
//! The vocabulary types live in `moonfield-rhi-core`; this module maps each
//! onto exactly one Vulkan concept via the [`ToVk`] extension trait, so
//! backend code calls `value.to_vk()` while the vocabulary stays free of
//! `ash` types. Result codes convert through [`from_vk`].

use moonfield_rhi_core::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, Error, Extent2d, Filter,
    Format, FrontFace, Rect2d, Viewport, WrapMode,
};

/// Vulkan mapping for a shared vocabulary type.
pub(crate) trait ToVk {
    /// The Vulkan type this vocabulary maps onto.
    type Target;

    /// Convert to the Vulkan equivalent.
    fn to_vk(self) -> Self::Target;
}

/// Convert an ash result code into [`Error::Backend`].
pub(crate) fn from_vk(result: ash::vk::Result) -> Error {
    Error::Backend(format!("{result:?}"))
}

impl ToVk for Format {
    type Target = ash::vk::Format;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::B8G8R8A8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
            Self::R8G8B8A8Unorm => ash::vk::Format::R8G8B8A8_UNORM,
            Self::R16G16B16A16Sfloat => ash::vk::Format::R16G16B16A16_SFLOAT,
            Self::D32Sfloat => ash::vk::Format::D32_SFLOAT,
        }
    }
}

impl ToVk for Extent2d {
    type Target = ash::vk::Extent2D;

    fn to_vk(self) -> Self::Target {
        ash::vk::Extent2D {
            width: self.width,
            height: self.height,
        }
    }
}

impl ToVk for Rect2d {
    type Target = ash::vk::Rect2D;

    fn to_vk(self) -> Self::Target {
        ash::vk::Rect2D {
            offset: ash::vk::Offset2D {
                x: self.offset.x,
                y: self.offset.y,
            },
            extent: self.extent.to_vk(),
        }
    }
}

impl ToVk for Viewport {
    type Target = ash::vk::Viewport;

    fn to_vk(self) -> Self::Target {
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

impl ToVk for CompareOp {
    type Target = ash::vk::CompareOp;

    fn to_vk(self) -> Self::Target {
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

impl ToVk for CullMode {
    type Target = ash::vk::CullModeFlags;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::None => ash::vk::CullModeFlags::NONE,
            Self::Front => ash::vk::CullModeFlags::FRONT,
            Self::Back => ash::vk::CullModeFlags::BACK,
        }
    }
}

impl ToVk for FrontFace {
    type Target = ash::vk::FrontFace;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::Clockwise => ash::vk::FrontFace::CLOCKWISE,
            Self::CounterClockwise => ash::vk::FrontFace::COUNTER_CLOCKWISE,
        }
    }
}

impl ToVk for ClearValue {
    type Target = ash::vk::ClearValue;

    fn to_vk(self) -> Self::Target {
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

impl ToVk for AttachmentLayout {
    type Target = ash::vk::ImageLayout;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::Present => ash::vk::ImageLayout::PRESENT_SRC_KHR, // still need this layout
            Self::ShaderRead | Self::DepthStencil => ash::vk::ImageLayout::GENERAL,
        }
    }
}

impl ToVk for CommandBufferUsage {
    type Target = ash::vk::CommandBufferUsageFlags;

    fn to_vk(self) -> Self::Target {
        let mut flags = ash::vk::CommandBufferUsageFlags::empty();
        if self.contains(CommandBufferUsage::ONE_TIME_SUBMIT) {
            flags |= ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT;
        }
        flags
    }
}

impl ToVk for Filter {
    type Target = ash::vk::Filter;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::Nearest => ash::vk::Filter::NEAREST,
            Self::Linear => ash::vk::Filter::LINEAR,
        }
    }
}

impl ToVk for WrapMode {
    type Target = ash::vk::SamplerAddressMode;

    fn to_vk(self) -> Self::Target {
        match self {
            Self::ClampToEdge => ash::vk::SamplerAddressMode::CLAMP_TO_EDGE,
            Self::Repeat => ash::vk::SamplerAddressMode::REPEAT,
            Self::MirroredRepeat => ash::vk::SamplerAddressMode::MIRRORED_REPEAT,
        }
    }
}
