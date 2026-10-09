//! Shader modules: runtime-compiled MSL libraries.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{MTLDevice, MTLFunction, MTLLibrary};

use crate::device::Device;
use moonfield_rhi_core::{Error, Result};

/// A compiled Metal library created from MSL source at runtime.
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

    pub(crate) fn function(&self, name: &str) -> Result<Retained<ProtocolObject<dyn MTLFunction>>> {
        self.library
            .newFunctionWithName(&NSString::from_str(name))
            .ok_or_else(|| Error::ShaderCompilation(format!("function `{name}` not found")))
    }
}
