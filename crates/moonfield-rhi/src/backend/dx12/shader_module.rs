use crate::{types::*, ShaderModule, ShaderStage, RhiError};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12ShaderModule {
    #[allow(dead_code)]
    pub bytecode: Vec<u8>,
    pub stage: ShaderStage,
}

impl Dx12ShaderModule {
    pub fn new(desc: &ShaderModuleDescriptor) -> StdResult<Self, RhiError> {
        // In a real implementation, we would validate the shader bytecode
        // For now, we'll just store the bytecode
        let bytecode = desc.code.to_vec();
        
        Ok(Dx12ShaderModule {
            bytecode,
            stage: desc.stage.clone(),
        })
    }
    
    pub fn get_bytecode(&self) -> &[u8] {
        &self.bytecode
    }
    
    pub fn get_stage(&self) -> &ShaderStage {
        &self.stage
    }
}

impl ShaderModule for Dx12ShaderModule {}