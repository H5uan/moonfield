//! Graphics pipeline: vertex + fragment functions with a color target.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLComputePipelineState, MTLDevice, MTLRenderPipelineDescriptor, MTLRenderPipelineState,
    MTLSize,
};

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
    /// Build a vertex+fragment pipeline from one library.
    pub fn new(
        device: &Device,
        module: &ShaderModule,
        vertex_entry: &str,
        fragment_entry: &str,
        format: Format,
    ) -> Result<Self> {
        Self::from_modules(device, module, vertex_entry, module, fragment_entry, format)
    }

    /// Build a vertex+fragment pipeline from two libraries — the shape Slang
    /// compiles into (one `.metallib` per entry point).
    pub fn from_modules(
        device: &Device,
        vertex: &ShaderModule,
        vertex_entry: &str,
        fragment: &ShaderModule,
        fragment_entry: &str,
        format: Format,
    ) -> Result<Self> {
        let vertex = vertex.function(vertex_entry)?;
        let fragment = fragment.function(fragment_entry)?;
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

/// A compute pipeline (one function).
#[derive(Clone)]
pub struct ComputePipeline {
    pso: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    threads_per_group: MTLSize,
}

impl ComputePipeline {
    /// Build a pipeline from a compiled Slang shader's Metal library.
    pub fn from_compiled(
        device: &Device,
        compiled: &moonfield_rhi_core::shader::CompiledShader,
    ) -> Result<Self> {
        let module = ShaderModule::from_compiled(device, compiled)?;
        Self::new(device, &module, &compiled.entry)
    }

    /// Build a pipeline from a library entry point. Threads per threadgroup
    /// come from the device (`threadExecutionWidth × maxTotalThreads` capped
    /// at the pipeline's limit).
    pub fn new(device: &Device, module: &ShaderModule, entry: &str) -> Result<Self> {
        let function = module.function(entry)?;
        let pso = device
            .ctx()
            .shared()
            .device()
            .newComputePipelineStateWithFunction_error(&function)
            .map_err(|err| Error::Backend(format!("{err:?}")))?;
        let threads = pso.threadExecutionWidth();
        let max = pso.maxTotalThreadsPerThreadgroup();
        let width = threads.min(max);
        Ok(Self {
            pso,
            threads_per_group: MTLSize {
                width,
                height: 1,
                depth: 1,
            },
        })
    }

    pub(crate) fn pso(&self) -> &ProtocolObject<dyn MTLComputePipelineState> {
        &self.pso
    }

    pub(crate) fn threads_per_group(&self) -> MTLSize {
        self.threads_per_group
    }
}
