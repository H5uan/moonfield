use core::fmt;
use std::any::Any;

use crate::{
    Backend, adapter::Adapter, command_encoder::CommandEncoder, device::Device,
    instance::Instance, queue::Queue, surface::Surface,
};

pub(crate) mod adapter;
pub(crate) mod command_encoder;
pub(crate) mod device;
pub(crate) mod instance;
pub(crate) mod queue;
pub(crate) mod surface;

pub trait Api: fmt::Debug + Sized + 'static {
    const VARIANT: Backend;

    type Instance: Instance<A = Self>;
    type Surface: Surface<A = Self>;
    type Adapter: Adapter<A = Self>;
    type Device: Device<A = Self>;

    type Queue: Queue<A = Self>;
    type CommandEncoder: CommandEncoder<A = Self>;

    type Buffer: DynBuffer;

    type ShaderModule: DynShaderModule;

    type PipelineCache: DynPipelineCache;
    type PipelineLayout: DynPipelineLayout;
    type RenderPipeline: DynRenderPipeline;
}
pub trait DynResource: Any + 'static {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub trait DynBuffer: DynResource + fmt::Debug {}
pub trait DynCommandBuffer: DynResource + fmt::Debug {}
pub trait DynPipelineLayout: DynResource + fmt::Debug {}

pub trait DynPipelineCache: DynResource + fmt::Debug {}

pub trait DynRenderPipeline: DynResource + fmt::Debug {}

pub trait DynShaderModule: DynResource + fmt::Debug {}
