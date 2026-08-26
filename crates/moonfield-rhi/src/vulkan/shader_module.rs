//! Vulkan shader module abstraction.

use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use ash::vk;

/// A Vulkan shader module created from SPIR-V bytecode.
pub struct ShaderModule {
    module: vk::ShaderModule,
    device: ash::Device,
}

impl ShaderModule {
    /// Create a shader module from SPIR-V bytecode.
    pub fn from_spirv(device: &Device, bytecode: &[u8]) -> Result<Self> {
        // SPIR-V bytecode is an array of 32-bit words; the byte slice length must be a multiple of 4.
        if !bytecode.len().is_multiple_of(4) {
            return Err(Error::Validation(
                "SPIR-V bytecode length must be a multiple of 4".to_string(),
            ));
        }

        // SAFETY: the length check above guarantees no remainder bytes.
        let code: Vec<u32> = bytecode
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();

        let create_info = vk::ShaderModuleCreateInfo::default().code(&code);

        let module = unsafe {
            device
                .raw()
                .create_shader_module(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create shader module: {:?}", e)))?
        };

        Ok(Self {
            module,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::ShaderModule` handle.
    pub fn raw(&self) -> vk::ShaderModule {
        self.module
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.module, None);
        }
    }
}
