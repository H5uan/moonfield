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

/// The kind of shader resource a [`HeapMapping`] maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapMappingResource {
    /// A sampled image (`OpTypeImage` with Sampled=1), indexed into the
    /// resource heap.
    SampledImage,
    /// A standalone sampler (`OpTypeSampler`), indexed into the sampler heap.
    Sampler,
    /// A combined image sampler (`OpTypeSampledImage`): the image index comes
    /// from `push_offset` (resource heap) and the sampler index from
    /// `sampler_push_offset` (sampler heap).
    CombinedImageSampler,
}

/// A binding→heap mapping for descriptor-heap pipelines
/// (`VkDescriptorSetAndBindingMappingEXT` with
/// `HEAP_WITH_PUSH_INDEX`): the shader's `DescriptorSet`/`Binding`-decorated
/// variable at (set, binding) reads its descriptor from the bound heap at
/// `slot_index * slot_stride`, where the slot index is the `u32` in push data
/// at `push_offset`. Lets shaders keep classic binding declarations while all
/// descriptors live in the descriptor heap and indices flow through push
/// data — no descriptor sets are ever allocated.
#[derive(Debug, Clone, Copy)]
pub struct HeapMapping {
    /// The `DescriptorSet` decoration value the mapping applies to.
    pub set: u32,
    /// The `Binding` decoration value the mapping applies to.
    pub binding: u32,
    /// The resource kind at that binding.
    pub resource: HeapMappingResource,
    /// Byte offset into push data holding the resource-heap slot index (the
    /// sampler-heap index for [`HeapMappingResource::Sampler`]).
    pub push_offset: u32,
    /// Combined image samplers only: byte offset into push data holding the
    /// sampler-heap slot index. Unused otherwise.
    pub sampler_push_offset: u32,
}

/// Optional pipeline configuration beyond the [`GraphicsPipeline::new`]
/// defaults: binding→heap mappings. Rasterizer state (blend, cull, depth) is
/// dynamic and applied per draw through the command buffer.
///
/// Every pipeline is a descriptor-heap pipeline
/// (`VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT` with a null layout), so
/// descriptor-set bindings and push constants are unavailable — shaders read
/// the bound heaps and push data
/// ([`CommandBuffer::push_data`](crate::CommandBuffer::push_data)) instead.
#[derive(Default)]
pub struct PipelineOptions<'a> {
    /// Binding→heap mappings for pipelines whose shaders declare
    /// `DescriptorSet`/`Binding` resources (the driver resolves them against
    /// the bound heaps; each shader stage gets the same list, which the spec
    /// explicitly allows to overspecify).
    pub heap_mappings: &'a [HeapMapping],
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
            &PipelineOptions::default(),
        )
    }

    /// Create a graphics pipeline with explicit attachment formats and
    /// [`PipelineOptions`] (binding→heap mappings).
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
        options: &PipelineOptions,
    ) -> Result<Self> {
        let vertex_entry = std::ffi::CString::new("main").unwrap();
        let fragment_entry = std::ffi::CString::new("main").unwrap();

        // Binding→heap mappings for descriptor-heap pipelines whose shaders
        // keep DescriptorSet/Binding decorations: chained into each shader
        // stage (the spec explicitly allows mappings a stage does not
        // declare). Slot strides mirror DescriptorHeap's slot sizing
        // (descriptor size rounded up to its alignment).
        let heap_props = device.descriptor_heap_properties();
        let image_stride = heap_props
            .image_descriptor_size
            .max(heap_props.image_descriptor_alignment) as u32;
        let sampler_stride = heap_props
            .sampler_descriptor_size
            .max(heap_props.sampler_descriptor_alignment) as u32;
        let mappings_vk: Vec<vk::DescriptorSetAndBindingMappingEXT> = options
            .heap_mappings
            .iter()
            .map(|mapping| {
                let (resource_mask, index_stride) = match mapping.resource {
                    HeapMappingResource::SampledImage => {
                        (vk::SpirvResourceTypeFlagsEXT::SAMPLED_IMAGE, image_stride)
                    }
                    HeapMappingResource::Sampler => {
                        (vk::SpirvResourceTypeFlagsEXT::SAMPLER, sampler_stride)
                    }
                    HeapMappingResource::CombinedImageSampler => (
                        vk::SpirvResourceTypeFlagsEXT::COMBINED_SAMPLED_IMAGE,
                        image_stride,
                    ),
                };
                let push_index = vk::DescriptorMappingSourcePushIndexEXT::default()
                    .heap_offset(0)
                    .push_offset(mapping.push_offset)
                    .heap_index_stride(index_stride)
                    .heap_array_stride(0)
                    .use_combined_image_sampler_index(false)
                    .sampler_heap_offset(0)
                    .sampler_push_offset(mapping.sampler_push_offset)
                    .sampler_heap_index_stride(sampler_stride)
                    .sampler_heap_array_stride(0);
                vk::DescriptorSetAndBindingMappingEXT::default()
                    .descriptor_set(mapping.set)
                    .first_binding(mapping.binding)
                    .binding_count(1)
                    .resource_mask(resource_mask)
                    .source(vk::DescriptorMappingSourceEXT::HEAP_WITH_PUSH_INDEX)
                    .source_data(vk::DescriptorMappingSourceDataEXT { push_index })
            })
            .collect();
        let mut mapping_info_vs =
            vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings_vk);
        let mut mapping_info_fs =
            vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings_vk);

        let mut vertex_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader.raw())
            .name(&vertex_entry);
        let mut fragment_stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader.raw())
            .name(&fragment_entry);
        if !mappings_vk.is_empty() {
            vertex_stage = vertex_stage.push(&mut mapping_info_vs);
            fragment_stage = fragment_stage.push(&mut mapping_info_fs);
        }
        let shader_stages = [vertex_stage, fragment_stage];

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
