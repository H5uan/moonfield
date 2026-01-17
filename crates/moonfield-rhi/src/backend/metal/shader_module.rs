use crate::{types::*, ShaderModule, ShaderStage};

pub struct MetalShaderModule {
    pub library: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLLibrary>>,
    pub stage: ShaderStage,
}

impl std::any::Any for MetalShaderModule {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalShaderModule>()
    }
}

impl ShaderModule for MetalShaderModule {}