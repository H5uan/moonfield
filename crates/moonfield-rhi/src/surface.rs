use std::any::Any;

use crate::Adapter;
use crate::types::SurfaceCapabilities;

/// Trait for surface functionality (handles window-system integration)
pub trait Surface: Any {
    /// Gets the capabilities of this surface for a given adapter
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities;
}
