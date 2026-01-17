use crate::{types::*, Surface, SurfaceCapabilities, Format, PresentMode, Adapter};

// Import tracing for logging
use tracing;

pub struct MetalSurface {}

impl Surface for MetalSurface {
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities {
        SurfaceCapabilities {
            formats: vec![Format::B8G8R8A8Unorm, Format::R8G8B8A8Unorm],
            present_modes: vec![PresentMode::Fifo],
            min_image_count: 2,
            max_image_count: 3,
        }
    }
}