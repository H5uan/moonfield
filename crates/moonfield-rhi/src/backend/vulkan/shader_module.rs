use std::sync::Arc;

use ash::vk::Handle;
use shader_slang::Stage;

use crate::{types::*, *};

pub struct VulkanShaderModule {
    pub device: ash::Device,
    pub shader_module: ash::vk::ShaderModule,
    pub stage: Stage,
}

impl ShaderProgram for VulkanShaderModule {}

impl Drop for VulkanShaderModule {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.shader_module, None);
        }
    }
}

struct Module {
    code: Vec<u8>,
    entry_point_name: String,
    shader_module: ash::vk::ShaderModule,
    has_bindless_desc_set: bool,
    bindless_desc_set: u32,
}

pub struct VkShaderProgram {
    device: ash::Device,
    modules: Vec<Module>,
}
