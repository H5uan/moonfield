//! Shader modules: Metal libraries from MSL source or compiled `.metallib`
//! archives.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{MTLDevice, MTLFunction, MTLLibrary};

use crate::device::Device;
use moonfield_rhi_core::shader::CompiledShader;
use moonfield_rhi_core::{Error, Result};

/// A compiled Metal library.
#[derive(Clone)]
pub struct ShaderModule {
    library: Retained<ProtocolObject<dyn MTLLibrary>>,
}

impl ShaderModule {
    /// Compile `source` (MSL) into a library on the device.
    pub fn from_msl(device: &Device, source: &str) -> Result<Self> {
        let source = NSString::from_str(source);
        let library = device
            .ctx()
            .shared()
            .device()
            .newLibraryWithSource_options_error(&source, None)
            .map_err(|err| Error::ShaderCompilation(format!("{err:?}")))?;
        Ok(Self { library })
    }

    /// Load a `.metallib` archive produced by the shared Slang compiler
    /// (`ShaderTarget::MetalLib`) — or an offline `slangc` compile.
    pub fn from_metallib(device: &Device, bytes: &[u8]) -> Result<Self> {
        let data = dispatch2::DispatchData::from_bytes(bytes);
        let library = device
            .ctx()
            .shared()
            .device()
            .newLibraryWithData_error(&data)
            .map_err(|err| Error::ShaderCompilation(format!("{err:?}")))?;
        Ok(Self { library })
    }

    /// Load a compiled Slang shader's Metal library.
    pub fn from_compiled(device: &Device, compiled: &CompiledShader) -> Result<Self> {
        Self::from_metallib(device, &compiled.code)
    }

    pub(crate) fn function(&self, name: &str) -> Result<Retained<ProtocolObject<dyn MTLFunction>>> {
        self.library
            .newFunctionWithName(&NSString::from_str(name))
            .ok_or_else(|| Error::ShaderCompilation(format!("function `{name}` not found")))
    }
}
