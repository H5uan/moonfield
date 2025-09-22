mod basic;
mod descriptors;
mod enums;
mod errors;

use std::fmt::Debug;

pub use basic::*;
pub use descriptors::*;
pub use enums::*;
pub use errors::*;

use crate::dynamic::{
    DynBuffer, DynCommandBuffer, DynFence, DynPipelineCache, DynRenderPipeline,
    DynShaderModule, DynSurfaceTexture, DynTexture, DynTextureView,
    DyncQuerySet,
};

pub type DeviceAddress = u64;
pub type Size = usize;
pub type Offset = usize;

pub const TIMEOUT_INFINITE: u64 = 0xFFFFFFFFFFFFFFFF;
pub const DEFAULT_ALIGNMENT: usize = 0xffffffff;
pub const REMAINING_TEXTURE_SIZE: u32 = 0xffffffff;
pub const ALL_LAYERS: u32 = 0xffffffff;
pub const ALL_MIPS: u32 = 0xffffffff;
pub const MAX_ACCELERATION_STRUCTURE_MOTION_KEY_COUNT: u32 = 2;

pub const ENTIRE_BUFFER: BufferRange = BufferRange { offset: 0, size: !0 };
pub const ENTIRE_TEXTURE: SubresourceRange = SubresourceRange {
    layer: 0,
    layer_count: ALL_LAYERS,
    mip: 0,
    mip_count: ALL_MIPS,
};
pub const ALL_SUBRESOURCES: SubresourceRange = SubresourceRange {
    layer: 0,
    layer_count: ALL_LAYERS,
    mip: 0,
    mip_count: ALL_MIPS,
};

pub type Label<'a> = Option<&'a str>;

pub trait Api: Clone + Debug + Sized + 'static {
    const VARIANT: Backend;

    type Instance: Instance<A = Self>;
    type Surface: Surface<A = Self>;
    type Adapter: Adapter<A = Self>;
    type Device: Device<A = Self>;

    type Queue: Queue<A = Self>;
    type CommandEncoder: CommandEncoder<A = Self>;

    type CommandBuffer: DynCommandBuffer;

    type Buffer: DynBuffer;
    type Texture: DynTexture;
    type SurfaceTexture: DynSurfaceTexture + core::borrow::Borrow<Self::Texture>;
    type TextureView: DynTextureView;
    type QuerySet: DyncQuerySet;

    type Fence: DynFence;

    type ShaderModule: DynShaderModule;
    type RenderPipeline: DynRenderPipeline;
    type PipelineCache: DynPipelineCache;
}

pub trait Instance: Sized {
    type A: Api;

    unsafe fn init(desc: &InstanceDesc) -> Result<Self, InstanceError>;
    unsafe fn create_surface(
        &self, display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
    ) -> Result<<Self::A as Api>::Surface, InstanceError>;
    unsafe fn enumerate_adapters(
        &self, surface_hint: Option<&<Self::A as Api>::Surface>,
    );
}

pub trait Surface {
    type A: Api;

    unsafe fn configure(
        &self, device: &<Self::A as Api>::Device,
    ) -> Result<(), SurfaceError>;

    unsafe fn unconfigure(&self, device: &<Self::A as Api>::Device);

    unsafe fn acquire_texture(
        &self, timeout: Option<core::time::Duration>,
    ) -> Result<Option<<Self::A as Api>::SurfaceTexture>, SurfaceError>;

    unsafe fn discard_texture(
        &self, texture: &<Self::A as Api>::SurfaceTexture,
    );
}

pub trait Adapter {
    type A: Api;

    unsafe fn open(&self) -> Result<(), DeviceError>;

    unsafe fn texture_format_capabilities();

    unsafe fn surface_capabilities(&self, surface: &<Self::A as Api>::Surface);
}

pub trait Device {
    type A: Api;

    unsafe fn create_buffer(
        &self, desc: &BufferDesc,
    ) -> Result<<Self::A as Api>::Buffer, DeviceError>;
    unsafe fn destroy_buffer(&self, buffer: <Self::A as Api>::Buffer);

    unsafe fn create_texture(
        &self, desc: &TextureDesc,
    ) -> Result<<Self::A as Api>::Texture, DeviceError>;
    unsafe fn destroy_texture(&self, texture: <Self::A as Api>::Texture);

    unsafe fn create_texture_view(
        &self, desc: &TextureViewDesc,
    ) -> Result<<Self::A as Api>::TextureView, DeviceError>;
    unsafe fn destroy_texture_view(
        &self, texture_view: <Self::A as Api>::TextureView,
    );

    unsafe fn create_command_encoder();
}

pub trait Queue {
    type A: Api;

    unsafe fn submit(
        &self, command_buffers: &[<Self::A as Api>::CommandBuffer],
        surface_textures: &[<Self::A as Api>::SurfaceTexture],
    );

    unsafe fn present(
        &self, surface: &<Self::A as Api>::Surface,
        surface_textures: &[<Self::A as Api>::SurfaceTexture],
    );
}

pub trait CommandEncoder {
    type A: Api;

    unsafe fn begin_encoding(&mut self) -> Result<(), DeviceError>;

    unsafe fn discard_encoding(&mut self);

    unsafe fn end_encoding(
        &mut self,
    ) -> Result<<Self::A as Api>::CommandBuffer, DeviceError>;

    unsafe fn begin_render_pass(
        &mut self, desc: &RenderPipelineDesc,
    ) -> Result<(), DeviceError>;

    unsafe fn end_render_pass(&mut self);

    unsafe fn set_render_pipeline(
        &mut self, pipeline: <Self::A as Api>::RenderPipeline,
    ) -> Result<(), DeviceError>;

    unsafe fn set_index_buffer(&mut self);

    unsafe fn set_vertex_buffer(&mut self);

    unsafe fn set_viewport(&mut self);

    unsafe fn set_scissor(&mut self);

    unsafe fn set_stencil_reference(&mut self);

    unsafe fn set_blend_constants(&mut self);

    unsafe fn draw(&mut self);

    unsafe fn draw_indexed(&mut self);
}
