use crate::{types::*, Swapchain, SwapchainImage, Format, Extent2D, RhiError};

// Import tracing for logging
use tracing;

pub struct MetalSwapchain {
    pub layer: objc2::rc::Retained<objc2_quartz_core::CAMetalLayer>,
    pub format: Format,
    pub extent: Extent2D,
}

impl Swapchain for MetalSwapchain {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError> {
        tracing::debug!("Acquiring next image from Metal swapchain");
        unsafe {
            let drawable = self
                .layer
                .nextDrawable()
                .ok_or_else(|| {
                    tracing::error!("Failed to acquire next drawable from Metal layer");
                    RhiError::AcquireImageFailed("Failed to acquire next drawable from Metal layer".to_string())
                })?;

            tracing::debug!("Successfully acquired next image from Metal swapchain");
            Ok(SwapchainImage {
                index: 0,
                _handle: objc2::rc::Retained::as_ptr(&drawable) as usize,
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
        tracing::debug!("Presenting image to Metal swapchain");
        Ok(())
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}