use std::any::Any;

use crate::types::SurfaceCapabilities;
use crate::Adapter;

/// Trait for surface functionality (handles window-system integration)
pub trait Surface: Any {
    /// Gets the capabilities of this surface for a given adapter
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities;
}