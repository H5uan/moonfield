//! wgpu graphics pipeline abstraction.

use crate::error::Result;
use crate::types::{Format, VertexBufferLayout};
use crate::web::device::Device;
use crate::web::shader_module::ShaderModule;

/// A wgpu render pipeline.
///
/// Unlike the native backend there is no `RenderPass` parameter — wgpu has
/// no render pass objects. Instead the pipeline is created against a concrete
/// color target [`Format`], which is the intended asymmetry between the two
/// backends.
pub struct GraphicsPipeline(wgpu::RenderPipeline);

impl GraphicsPipeline {
    /// Create a basic graphics pipeline for the given color target format.
    ///
    /// Mirrors the native pipeline: entry points `"main"`, triangle list,
    /// back-face culling, clockwise front face, no blending, no depth, and an
    /// empty pipeline layout (no bind groups or push constants).
    ///
    /// Note on front-face semantics: `wgpu::FrontFace::Cw` classifies
    /// clockwise-in-framebuffer triangles as front-facing, matching the
    /// native `vk::FrontFace::CLOCKWISE`; any viewport Y-flip difference is
    /// left to the caller's projection setup, as on the native side.
    pub fn new(
        device: &Device,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
        target: Format,
    ) -> Result<Self> {
        let attributes: Vec<wgpu::VertexAttribute> = vertex_layout
            .attributes
            .iter()
            .map(|attribute| wgpu::VertexAttribute {
                format: attribute.format.to_wgpu(),
                offset: attribute.offset as u64,
                shader_location: attribute.location,
            })
            .collect();
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: vertex_layout.stride as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];

        let layout = device
            .device()
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("moonfield-pipeline-layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });

        let pipeline = device
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("moonfield-graphics-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: vertex_shader.raw(),
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_shader.raw(),
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target.to_wgpu(),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self(pipeline))
    }

    /// Access the raw `wgpu::RenderPipeline` handle.
    pub fn raw(&self) -> &wgpu::RenderPipeline {
        &self.0
    }
}
