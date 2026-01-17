use crate::{types::*, *};
use ash::vk::Handle;
use std::any::Any;
use std::cell::Cell;
use std::sync::Arc;

use super::{VulkanSwapchain, VulkanPipeline, VulkanBuffer};

pub struct VulkanCommandBuffer {
    pub device: ash::Device,
    pub command_buffer: ash::vk::CommandBuffer,
    pub swapchain: Option<std::sync::Weak<VulkanSwapchain>>,
    pub current_image_index: Cell<Option<u32>>,
}

impl CommandBuffer for VulkanCommandBuffer {
    fn begin(&self) -> Result<(), RhiError> {
        unsafe {
            let begin_info = ash::vk::CommandBufferBeginInfo::default();
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)
                .map_err(|e| RhiError::InitializationFailed(format!("Failed to begin command buffer: {}", e)))
        }
    }

    fn end(&self) -> Result<(), RhiError> {
        unsafe {
            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| RhiError::InitializationFailed(format!("Failed to end command buffer: {}", e)))
        }
    }

    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage) {
        unsafe {
            let image_view = ash::vk::ImageView::from_raw(image.image_view as u64);
            let swapchain = self.swapchain.as_ref().and_then(|w| w.upgrade()).unwrap();
            let swapchain_image = swapchain.images[image.index as usize];

            self.current_image_index.set(Some(image.index));

            let mut layouts = swapchain.image_layouts.lock().unwrap();
            let old_layout = layouts[image.index as usize];

            let barrier = ash::vk::ImageMemoryBarrier2::default()
                .src_stage_mask(ash::vk::PipelineStageFlags2::TOP_OF_PIPE)
                .src_access_mask(ash::vk::AccessFlags2::empty())
                .dst_stage_mask(ash::vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(ash::vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(old_layout)
                .new_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(swapchain_image)
                .subresource_range(ash::vk::ImageSubresourceRange {
                    aspect_mask: ash::vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            let dependency_info = ash::vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&barrier));

            self.device.cmd_pipeline_barrier2(self.command_buffer, &dependency_info);

            layouts[image.index as usize] = ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            drop(layouts);

            let color_attachment = &desc.color_attachments[0];
            
            let clear_value = ash::vk::ClearValue {
                color: ash::vk::ClearColorValue {
                    float32: color_attachment.clear_value,
                },
            };

            let rendering_attachment = ash::vk::RenderingAttachmentInfo::default()
                .image_view(image_view)
                .image_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(match color_attachment.load_op {
                    LoadOp::Load => ash::vk::AttachmentLoadOp::LOAD,
                    LoadOp::Clear => ash::vk::AttachmentLoadOp::CLEAR,
                    LoadOp::DontCare => ash::vk::AttachmentLoadOp::DONT_CARE,
                })
                .store_op(match color_attachment.store_op {
                    StoreOp::Store => ash::vk::AttachmentStoreOp::STORE,
                    StoreOp::DontCare => ash::vk::AttachmentStoreOp::DONT_CARE,
                })
                .clear_value(clear_value);

            let extent = self.swapchain.as_ref()
                .and_then(|w| w.upgrade())
                .map(|s| s.extent)
                .unwrap_or(Extent2D { width: 800, height: 600 });

            let rendering_info = ash::vk::RenderingInfo::default()
                .render_area(ash::vk::Rect2D {
                    offset: ash::vk::Offset2D { x: 0, y: 0 },
                    extent: ash::vk::Extent2D { width: extent.width, height: extent.height },
                })
                .layer_count(1)
                .color_attachments(std::slice::from_ref(&rendering_attachment));

            self.device.cmd_begin_rendering(self.command_buffer, &rendering_info);
        }
    }

    fn end_render_pass(&self) {
        unsafe {
            self.device.cmd_end_rendering(self.command_buffer);

            let swapchain = self.swapchain.as_ref().and_then(|w| w.upgrade()).unwrap();
            let image_index = self.current_image_index.get().unwrap() as usize;
            let swapchain_image = swapchain.images[image_index];

            let barrier = ash::vk::ImageMemoryBarrier2::default()
                .src_stage_mask(ash::vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(ash::vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(ash::vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                .dst_access_mask(ash::vk::AccessFlags2::empty())
                .old_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(ash::vk::ImageLayout::PRESENT_SRC_KHR)
                .image(swapchain_image)
                .subresource_range(ash::vk::ImageSubresourceRange {
                    aspect_mask: ash::vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            let dependency_info = ash::vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&barrier));

            self.device.cmd_pipeline_barrier2(self.command_buffer, &dependency_info);

            let mut layouts = swapchain.image_layouts.lock().unwrap();
            layouts[image_index] = ash::vk::ImageLayout::PRESENT_SRC_KHR;
        }
    }

    fn set_viewport(&self, width: f32, height: f32) {
        unsafe {
            let viewport = ash::vk::Viewport {
                x: 0.0,
                y: 0.0,
                width,
                height,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            self.device.cmd_set_viewport(self.command_buffer, 0, &[viewport]);
        }
    }

    fn set_scissor(&self, width: u32, height: u32) {
        unsafe {
            let scissor = ash::vk::Rect2D {
                offset: ash::vk::Offset2D { x: 0, y: 0 },
                extent: ash::vk::Extent2D { width, height },
            };
            self.device.cmd_set_scissor(self.command_buffer, 0, &[scissor]);
        }
    }

    fn bind_pipeline(&self, pipeline: &dyn Pipeline) {
        let vk_pipeline = (pipeline as &dyn Any)
            .downcast_ref::<VulkanPipeline>()
            .expect("pipeline must be VulkanPipeline");

        unsafe {
            self.device.cmd_bind_pipeline(
                self.command_buffer,
                ash::vk::PipelineBindPoint::GRAPHICS,
                vk_pipeline.pipeline,
            );
        }
    }

    fn bind_vertex_buffer(&self, buffer: &dyn Buffer) {
        let vk_buffer = (buffer as &dyn Any)
            .downcast_ref::<VulkanBuffer>()
            .expect("buffer must be VulkanBuffer");

        unsafe {
            self.device.cmd_bind_vertex_buffers(
                self.command_buffer,
                0,
                &[vk_buffer.buffer],
                &[0],
            );
        }
    }

    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            self.device.cmd_draw(
                self.command_buffer,
                vertex_count,
                instance_count,
                first_vertex,
                first_instance,
            );
        }
    }
}