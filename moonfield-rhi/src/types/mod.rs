use crate::Api;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backend {
    Noop = 0,
    Vulkan = 1,
    Metal = 2,
}

#[derive(Debug, Clone)]
pub struct Capabilities {}

#[derive(Debug)]
pub struct ExposedAdapter<A: Api> {
    pub adapter: A::Adapter,
    pub capabilities: Capabilities,
}
