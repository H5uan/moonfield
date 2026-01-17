use crate::{types::*, Pipeline};

pub struct MetalPipeline {
    pub pipeline_state: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLRenderPipelineState>>,
}

impl std::any::Any for MetalPipeline {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalPipeline>()
    }
}

impl Pipeline for MetalPipeline {}