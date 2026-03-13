use std::any::Any;
use std::sync::Arc;

use crate::types::AdapterProperties;
use crate::Device;
use crate::types::RhiError;

/// Trait for GPU adapter functionality
pub trait Adapter: Any {
    /// Requests a logical device from this adapter
    fn request_device(&self) -> Result<Arc<dyn Device>, RhiError>;
    
    /// Gets properties of this adapter
    fn get_properties(&self) -> AdapterProperties;
}