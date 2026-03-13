#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    Undefined,
    Buffer,
    BufferWithCounter,
    Texture,
    Sampler,
    CombinedTextureSampler,
    AccelerationStructure,
}
