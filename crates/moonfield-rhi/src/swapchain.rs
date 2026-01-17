use std::any::Any;

use crate::types::{Extent2D, Format, RhiError, SwapchainImage};

/// Trait for swapchain functionality (handles presentation)
pub trait Swapchain: Any {
    /// Acquires the next available image for rendering
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError>;
    
    /// Presents a rendered image to the screen
    fn present(&self, image: SwapchainImage) -> Result<(), RhiError>;
    
    /// Gets the format of the swapchain
    fn get_format(&self) -> Format;
    
    /// Gets the extent (dimensions) of the swapchain
    fn get_extent(&self) -> Extent2D;
}