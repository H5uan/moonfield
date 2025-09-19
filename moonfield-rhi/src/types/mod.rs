mod enums;
mod errors;
mod basic_types;
mod basic_descriptors;
mod pipeline_descriptors;
mod traits;
mod device_traits;

pub use enums::*;
pub use errors::*;
pub use basic_types::*;
pub use basic_descriptors::*;
pub use pipeline_descriptors::*;
pub use traits::*;
pub use device_traits::*;

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
    mip_count: ALL_MIPS 
};
pub const ALL_SUBRESOURCES: SubresourceRange = SubresourceRange { 
    layer: 0, 
    layer_count: ALL_LAYERS, 
    mip: 0, 
    mip_count: ALL_MIPS 
};
