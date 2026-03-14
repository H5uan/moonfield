use std::sync::Arc;

use moonfield_shader::SlangStage as Stage;

use crate::types::format::Format;
use crate::types::vertex::VertexInputDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage {
    Vertex,
    Fragment,
}

pub struct ShaderModuleDescriptor<'a> {
    pub code: &'a [u8],
    pub stage: Stage,
}

pub struct GraphicsPipelineDescriptor {
    pub vertex_shader: Arc<dyn crate::ShaderProgram>,
    pub fragment_shader: Arc<dyn crate::ShaderProgram>,
    pub vertex_input: VertexInputDescriptor,
    pub render_pass_format: Format,
}
