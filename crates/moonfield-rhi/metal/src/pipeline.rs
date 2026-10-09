//! Graphics pipeline: vertex + fragment functions with a color target.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLDevice, MTLRenderPipelineDescriptor, MTLRenderPipelineState};

use crate::device::Device;
use crate::formats::ToMetal;
use crate::shader_module::ShaderModule;
use moonfield_rhi_core::{Error, Format, Result};

/// A rasterization pipeline (vertex + fragment functions, one color format).
#[derive(Clone)]
pub struct GraphicsPipeline {
    pso: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl GraphicsPipeline {
    /// Build a vertex+fragment pipeline rendering into `format`.
    pub fn new(
        device: &Device,
        module: &ShaderModule,
        vertex_entry: &str,
        fragment_entry: &str,
        format: Format,
    ) -> Result<Self> {
        let vertex = module.function(vertex_entry)?;
        let fragment = module.function(fragment_entry)?;
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(vertex.as_ref()));
        descriptor.setFragmentFunction(Some(fragment.as_ref()));
        // SAFETY: attachment index 0 is always valid.
        unsafe {
            descriptor
                .colorAttachments()
                .objectAtIndexedSubscript(0)
                .setPixelFormat(format.to_metal());
        }
        let pso = device
            .ctx()
            .shared()
            .device()
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|err| Error::Backend(format!("{err:?}")))?;
        Ok(Self { pso })
    }

    pub(crate) fn pso(&self) -> &ProtocolObject<dyn MTLRenderPipelineState> {
        &self.pso
    }
}
