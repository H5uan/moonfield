//! wgpu shader module abstraction.

use crate::error::{Error, Result};
use crate::web::device::Device;

/// A wgpu shader module, created from SPIR-V bytecode or WGSL source.
pub struct ShaderModule(wgpu::ShaderModule);

impl ShaderModule {
    /// Create a shader module from SPIR-V bytecode — the same signature as
    /// the native backend, so Slang-compiled SPIR-V is portable across both.
    ///
    /// wgpu runs the bytecode through naga's SPIR-V frontend (the crate's
    /// `spirv` feature), translating to WGSL on WebGPU targets; the
    /// `SPIRV_SHADER_PASSTHROUGH` fast path is native-only and not used.
    ///
    /// Note: wgpu validates lazily — malformed bytecode may not be reported
    /// here but instead surface as a validation error at pipeline creation.
    pub fn from_spirv(device: &Device, bytecode: &[u8]) -> Result<Self> {
        // SPIR-V bytecode is an array of 32-bit words; the byte slice length must be a multiple of 4.
        if !bytecode.len().is_multiple_of(4) {
            return Err(Error::Validation(
                "SPIR-V bytecode length must be a multiple of 4".to_string(),
            ));
        }

        let code: Vec<u32> = bytecode
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        let module = device
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("moonfield-shader-module"),
                source: wgpu::ShaderSource::SpirV(code.into()),
            });
        Ok(Self(module))
    }

    /// Create a shader module from WGSL source.
    ///
    /// Note: wgpu validates lazily — malformed WGSL may not be reported here
    /// but instead surface as a validation error at pipeline creation time.
    pub fn from_wgsl(device: &Device, source: &str) -> Result<Self> {
        let module = device
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("moonfield-shader-module"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        Ok(Self(module))
    }

    /// Access the raw `wgpu::ShaderModule` handle.
    pub fn raw(&self) -> &wgpu::ShaderModule {
        &self.0
    }
}
