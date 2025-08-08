use crate::Api;

/// Logical GPU
pub trait Device {
    type A: Api;

    fn create_buffer(&self);
    fn destroy_buffer(&self);

    fn crate_command_encoder(&self);

    fn create_shader_module(&self);
    fn destroy_shader_module(&self);

    fn create_pipeline_layout(&self);
    fn destroy_pipeline_layout(&self);

    fn create_render_pipeline(&self);
    fn destroy_render_pipeline(&self);

    fn create_pipeline_cache(&self);
    fn destroy_pipeline_cache(&self);

    fn create_surface(&self);
}
