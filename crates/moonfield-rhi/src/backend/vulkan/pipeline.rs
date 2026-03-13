use crate::{types::*, *};
use ash::vk::Handle;
use std::sync::Arc;

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