//! Metal conversions for the shared RHI vocabulary.
//!
//! The vocabulary types live in `moonfield-rhi-core`; this module maps each
//! onto its Metal equivalent via the [`ToMetal`] extension trait, so backend
//! code calls `value.to_metal()` while the vocabulary stays free of `objc2`
//! types.

use moonfield_rhi_core::{ClearValue, Format, LoadOp, StoreOp};

use objc2_metal::{MTLClearColor, MTLLoadAction, MTLPixelFormat, MTLStoreAction};

/// Metal mapping for a shared vocabulary type.
pub(crate) trait ToMetal {
    /// The Metal type this vocabulary maps onto.
    type Target;

    /// Convert to the Metal equivalent.
    fn to_metal(self) -> Self::Target;
}

impl ToMetal for Format {
    type Target = MTLPixelFormat;

    fn to_metal(self) -> Self::Target {
        match self {
            Self::B8G8R8A8Unorm => MTLPixelFormat::BGRA8Unorm,
            Self::R8G8B8A8Unorm => MTLPixelFormat::RGBA8Unorm,
            Self::R16G16B16A16Sfloat => MTLPixelFormat::RGBA16Float,
            Self::D32Sfloat => MTLPixelFormat::Depth32Float,
        }
    }
}

impl ToMetal for LoadOp {
    type Target = MTLLoadAction;

    fn to_metal(self) -> Self::Target {
        match self {
            Self::Load => MTLLoadAction::Load,
            Self::Clear => MTLLoadAction::Clear,
        }
    }
}

impl ToMetal for StoreOp {
    type Target = MTLStoreAction;

    fn to_metal(self) -> Self::Target {
        match self {
            Self::Store => MTLStoreAction::Store,
            Self::Discard => MTLStoreAction::DontCare,
        }
    }
}

impl ToMetal for ClearValue {
    type Target = MTLClearColor;

    fn to_metal(self) -> Self::Target {
        match self {
            Self::Color([r, g, b, a]) => MTLClearColor {
                red: r as f64,
                green: g as f64,
                blue: b as f64,
                alpha: a as f64,
            },
            Self::DepthStencil { depth, .. } => MTLClearColor {
                red: depth as f64,
                green: depth as f64,
                blue: depth as f64,
                alpha: 1.0,
            },
        }
    }
}
