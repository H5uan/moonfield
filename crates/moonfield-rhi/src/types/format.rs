#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Undefined,

    R8Uint,
    R8Sint,
    R8Unorm,
    R8Snorm,

    RG8Uint,
    RG8Sint,
    RG8Unorm,
    RG8Snorm,

    RGBA8Uint,
    RGBA8Sint,
    RGBA8Unorm,
    RGBA8UnormSrgb,
    RGBA8Snorm,

    BGRA8Unorm,
    BGRA8UnormSrgb,
    BGRX8Unorm,
    BGRX8UnormSrgb,

    R16Uint,
    R16Sint,
    R16Unorm,
    R16Snorm,
    R16Float,

    RG16Uint,
    RG16Sint,
    RG16Unorm,
    RG16Snorm,
    RG16Float,

    RGBA16Uint,
    RGBA16Sint,
    RGBA16Unorm,
    RGBA16Snorm,
    RGBA16Float,

    R32Uint,
    R32Sint,
    R32Float,

    RG32Uint,
    RG32Sint,
    RG32Float,

    RGB32Uint,
    RGB32Sint,
    RGB32Float,

    RGBA32Uint,
    RGBA32Sint,
    RGBA32Float,

    R64Uint,
    R64Sint,

    BGRA4Unorm,
    B5G6R5Unorm,
    BGR5A1Unorm,

    RGB9E5Ufloat,
    RGB10A2Uint,
    RGB10A2Unorm,
    R11G11B10Float,

    // Depth/stencil formats
    D32Float,
    D16Unorm,
    D32FloatS8Uint,

    // Compressed formats
    BC1Unorm,
    BC1UnormSrgb,
    BC2Unorm,
    BC2UnormSrgb,
    BC3Unorm,
    BC3UnormSrgb,
    BC4Unorm,
    BC4Snorm,
    BC5Unorm,
    BC5Snorm,
    BC6HUfloat,
    BC6HSfloat,
    BC7Unorm,
    BC7UnormSrgb,
}
