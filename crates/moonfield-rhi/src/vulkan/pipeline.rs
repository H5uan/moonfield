//! Vulkan graphics pipeline abstraction.

use crate::error::{Error, Result};
use crate::types::{Format, VertexBufferLayout};
use crate::vulkan::device::Device;
use crate::vulkan::shader_module::ShaderModule;
use ash::vk;
use ash::vk::TaggedStructure as _;

/// How the pipeline's single color attachment blends with existing pixels.
///
/// Blend is now a dynamic state: [`CommandBuffer::set_blend_state`] applies
/// it per draw, so this type only names the preset used by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Blending disabled; the fragment color overwrites the target.
    #[default]
    Off,
    /// Premultiplied-alpha blending: color `One, OneMinusSrcAlpha, Add`;
    /// alpha `OneMinusDstAlpha, One, Add`. What egui expects.
    PremultipliedAlpha,
}

/// One pipeline stage: a compiled shader module bound to a stage slot.
///
/// The stage and entry-point name come from the module itself (recorded by
/// [`ShaderModule::from_compiled`]), so a stage is described by pointing at a
/// compiled module — the caller never names `VERTEX`/`FRAGMENT` by hand and
/// cannot mislabel a shader. Two modules compiled from the same file (e.g.
/// `egui.slang`'s `vs_main` and `fs_gamma`) are two entries of this type.
pub struct ShaderStageDesc<'a> {
    /// The compiled module.
    pub module: &'a ShaderModule,
}

/// A Vulkan graphics pipeline.
pub struct GraphicsPipeline {
    pipeline: vk::Pipeline,
    device: ash::Device,
}

impl GraphicsPipeline {
    /// Create a basic graphics pipeline targeting a single color attachment.
    ///
    /// The pipeline uses the provided vertex/fragment shaders. Viewport,
    /// scissor, blend, cull, and depth state are all dynamic — they are set
    /// per draw through the command buffer (see
    /// [`CommandBuffer::begin_rendering`](crate::CommandBuffer::begin_rendering)),
    /// so the pipeline is independent of the target extent. Attachment
    /// formats are baked into the pipeline (they affect shader microcode);
    /// `color_format` declares the single color target.
    pub fn new(
        device: &Device,
        color_format: Format,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
    ) -> Result<Self> {
        Self::new_with_options(
            device,
            &[color_format],
            None,
            vertex_shader,
            fragment_shader,
            vertex_layout,
        )
    }

    /// Create a graphics pipeline with explicit attachment formats.
    ///
    /// `color_formats` are the color attachment formats the pipeline will be
    /// used against (typically one; multiple for MRT); `depth_format` is the
    /// optional depth attachment format (reverse-Z `D32Sfloat`). These feed
    /// `VkPipelineRenderingCreateInfo`, the dynamic-rendering replacement for
    /// a compatible render pass.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_options(
        device: &Device,
        color_formats: &[Format],
        depth_format: Option<Format>,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
    ) -> Result<Self> {
        Self::new_with_stages(
            device,
            color_formats,
            depth_format,
            &[
                ShaderStageDesc {
                    module: vertex_shader,
                },
                ShaderStageDesc {
                    module: fragment_shader,
                },
            ],
            vertex_layout,
        )
    }

    /// Create a graphics pipeline from an explicit stage list.
    ///
    /// Every module's stage (from its `[shader("...")]` compilation) and
    /// emitted entry-point name are read off the module; the pipeline names
    /// exactly those stages. This is the general form — the two-stage
    /// `new_with_options` is a special case, and mesh-shader or tessellation
    /// pipelines are just longer stage lists. Modules without stage
    /// information (raw `from_spirv`) are rejected: slot guessing is exactly
    /// the bug this API removes.
    pub fn new_with_stages(
        device: &Device,
        color_formats: &[Format],
        depth_format: Option<Format>,
        stages: &[ShaderStageDesc<'_>],
        vertex_layout: &VertexBufferLayout,
    ) -> Result<Self> {
        // Entry names and the create infos that point at them must live for the
        // whole pipeline construction; collect the names first so the infos
        // can borrow them safely.
        let entries: Result<Vec<_>> = stages
            .iter()
            .map(|desc| {
                std::ffi::CString::new(desc.module.entry().ok_or_else(|| {
                    Error::Validation(
                        "shader module has no entry-point name; compile through \
                             `Compiler` + `ShaderModule::from_compiled`"
                            .to_string(),
                    )
                })?)
                .map_err(|e| Error::Validation(format!("entry point name is not valid C: {e}")))
            })
            .collect();
        let entries = entries?;
        let shader_stages: Result<Vec<_>> = stages
            .iter()
            .zip(&entries)
            .map(|(desc, entry)| {
                let stage = desc.module.stage().ok_or_else(|| {
                    Error::Validation(
                        "shader module has no stage information; compile through \
                         `Compiler` + `ShaderModule::from_compiled`"
                            .to_string(),
                    )
                })?;
                Ok(vk::PipelineShaderStageCreateInfo::default()
                    .stage(stage)
                    .module(desc.module.raw())
                    .name(entry))
            })
            .collect();
        let shader_stages = shader_stages?;

        let binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(vertex_layout.stride)
            .input_rate(vk::VertexInputRate::VERTEX);
        let attributes: Vec<vk::VertexInputAttributeDescription> = vertex_layout
            .attributes
            .iter()
            .map(|attribute| {
                vk::VertexInputAttributeDescription::default()
                    .binding(0)
                    .location(attribute.location)
                    .format(attribute.format.to_vk())
                    .offset(attribute.offset)
            })
            .collect();

        let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(std::slice::from_ref(&binding))
            .vertex_attribute_descriptions(&attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        // Viewport, scissor, cull, blend, and depth state are dynamic;
        // only their attachment counts are fixed here. Dynamic states that
        // are merely resets (topology stays TRIANGLE_LIST) can be omitted.
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
            vk::DynamicState::CULL_MODE,
            vk::DynamicState::FRONT_FACE,
            vk::DynamicState::DEPTH_TEST_ENABLE,
            vk::DynamicState::DEPTH_WRITE_ENABLE,
            vk::DynamicState::DEPTH_COMPARE_OP,
            vk::DynamicState::COLOR_BLEND_ENABLE_EXT,
            vk::DynamicState::COLOR_BLEND_EQUATION_EXT,
            vk::DynamicState::COLOR_WRITE_MASK_EXT,
        ];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        // Rasterizer state is entirely dynamic; the values in the create info
        // are ignored for the marked states (kept at sane defaults).
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        // Depth state is dynamic; the static values are ignored, so depth is
        // left disabled here to make depth-less pipelines valid.
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);

        // Blend state is dynamic; the per-attachment array still declares the
        // attachment count (one entry per color format) with inert values.
        let color_blend_attachments: Vec<_> = color_formats
            .iter()
            .map(|_| {
                vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)
            })
            .collect();
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        // Descriptor-heap pipelines have no pipeline layout at all (per the
        // extension: the layout must be NULL when the flag is set).
        let layout = vk::PipelineLayout::null();

        // Dynamic rendering replaces the compatible render pass: formats are
        // baked in through VkPipelineRenderingCreateInfo, and the pipeline
        // names no VkRenderPass (render_pass = VK_NULL_HANDLE, subpass 0).
        let color_formats_vk: Vec<vk::Format> = color_formats.iter().map(|f| f.to_vk()).collect();
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats_vk)
            .depth_attachment_format(depth_format.map_or(vk::Format::UNDEFINED, |f| f.to_vk()));

        // Pipelines are flagged through the flags2 struct
        // (VK_KHR_maintenance5), chained next to the rendering info.
        let mut flags2_info = vk::PipelineCreateFlags2CreateInfo::default()
            .flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_info)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .subpass(0)
            .render_pass(vk::RenderPass::null())
            .push(&mut rendering_info)
            .push(&mut flags2_info);

        let pipelines = unsafe {
            device
                .raw()
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    std::slice::from_ref(&pipeline_info),
                    None,
                )
                .map_err(|e| {
                    Error::Backend(format!("failed to create graphics pipeline: {:?}", e))
                })?
        };

        Ok(Self {
            pipeline: pipelines[0],
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Pipeline` handle.
    pub fn raw(&self) -> vk::Pipeline {
        self.pipeline
    }
}

impl Drop for GraphicsPipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
        }
    }
}
