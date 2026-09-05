//! Vulkan shader module abstraction.

use super::shader::CompiledShader;
use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use ash::vk;

/// A Vulkan shader module created from SPIR-V bytecode.
///
/// Carries the [`CompiledShader`] stage it was built from (when created via
/// [`from_compiled`](Self::from_compiled)) so pipeline construction can
/// validate that the module lands in the matching stage slot.
pub struct ShaderModule {
    module: vk::ShaderModule,
    device: ash::Device,
    stage: Option<vk::ShaderStageFlags>,
    entry: Option<String>,
}

impl ShaderModule {
    /// Create a shader module from raw SPIR-V bytecode, without stage
    /// information. Prefer [`from_compiled`](Self::from_compiled) — stages
    /// make pipeline slot validation possible.
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
            stage: None,
            entry: None,
        })
    }

    /// Create a shader module from a [`CompiledShader`], recording its stage
    /// and emitted entry-point name.
    pub fn from_compiled(device: &Device, compiled: &CompiledShader) -> Result<Self> {
        let mut module = Self::from_spirv(device, &compiled.spirv)?;
        module.stage = Some(compiled.stage);
        module.entry = Some(compiled.entry.clone());
        Ok(module)
    }

    /// Access the raw `vk::ShaderModule` handle.
    pub(crate) fn raw(&self) -> vk::ShaderModule {
        self.module
    }

    /// The Vulkan stage of the shader, when known (i.e. the module was built
    /// from a [`CompiledShader`]).
    pub(crate) fn stage(&self) -> Option<vk::ShaderStageFlags> {
        self.stage
    }

    /// The entry-point name in the emitted SPIR-V, when known.
    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }
}

impl Drop for ShaderModule {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.module, None);
        }
    }
}
