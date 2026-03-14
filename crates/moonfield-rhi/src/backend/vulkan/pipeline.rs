use std::sync::Arc;

use ash::vk::Handle;

use crate::{types::*, *};

pub struct VulkanPipeline {
    pub device: ash::Device,
    pub pipeline: ash::vk::Pipeline,
    pub pipeline_layout: ash::vk::PipelineLayout,
}

impl Pipeline for VulkanPipeline {}

impl Drop for VulkanPipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}
