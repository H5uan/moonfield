use crate::{types::*, *};
use ash::vk::Handle;
use std::sync::Arc;

pub struct VulkanShaderModule {
    pub device: ash::Device,
    pub shader_module: ash::vk::ShaderModule,
    pub stage: ShaderStage,
}

impl ShaderModule for VulkanShaderModule {}

impl Drop for VulkanShaderModule {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.shader_module, None);
        }
    }
}