use std::any::Any;
use std::sync::Arc;

use crate::types::{RhiError, BufferDescriptor, GraphicsPipelineDescriptor, ShaderModuleDescriptor, SwapchainDescriptor};
use crate::{Buffer, CommandPool, Pipeline, Queue, ShaderModule, Swapchain};

/// Trait for logical device functionality
pub trait Device: Any {
    /// Creates a new swapchain
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> Result<Arc<dyn Swapchain>, RhiError>;
    
    /// Creates a shader module
    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> Result<Arc<dyn ShaderModule>, RhiError>;
    
    /// Creates a graphics pipeline
    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> Result<Arc<dyn Pipeline>, RhiError>;
    
    /// Creates a buffer
    fn create_buffer(&self, desc: &BufferDescriptor) -> Result<Arc<dyn Buffer>, RhiError>;
    
    /// Creates a command pool
    fn create_command_pool(&self, swapchain: &Arc<dyn Swapchain>) -> Result<Arc<dyn CommandPool>, RhiError>;
    
    /// Gets the queue for submitting commands
    fn get_queue(&self) -> Arc<dyn Queue>;
}