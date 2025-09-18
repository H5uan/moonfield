mod enums;
mod errors;

pub use enums::*;
pub use errors::*;

pub type DeviceAddress = u64;
pub type Size = usize;
pub type Offset = usize;

#[allow(unused)]
pub const TIMEOUT_INFINITE: u64 = u64::MAX;




