//! Basic types and structures used throughout the RHI

/// 3D offset coordinates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Offset3D {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl Offset3D {
    pub fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    pub fn is_zero(&self) -> bool {
        self.x == 0 && self.y == 0 && self.z == 0
    }
}

/// 3D extent/dimensions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Extent3D {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels (if 2d or 3d).
    pub height: u32,
    /// Depth (if 3d).
    pub depth: u32,
}

impl Extent3D {
    pub fn new(width: u32, height: u32, depth: u32) -> Self {
        Self { width, height, depth }
    }

    pub fn is_whole_texture(&self) -> bool {
        *self == Self::WHOLE_TEXTURE
    }

    pub const WHOLE_TEXTURE: Self = Self {
        width: crate::REMAINING_TEXTURE_SIZE,
        height: crate::REMAINING_TEXTURE_SIZE,
        depth: crate::REMAINING_TEXTURE_SIZE,
    };
}


/// Buffer range specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BufferRange {
    /// Offset in bytes.
    pub offset: u64,
    /// Size in bytes.
    pub size: u64,
}

impl BufferRange {
    pub fn new(offset: u64, size: u64) -> Self {
        Self { offset, size }
    }
}

/// Subresource range specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SubresourceRange {
    /// First layer to use.
    /// For cube textures this should be a multiple of 6.
    pub layer: u32,
    /// Number of layers to use.
    /// For cube textures this should be a multiple of 6.
    /// Use ALL_LAYERS to use all remaining layers.
    pub layer_count: u32,
    /// First mip level to use.
    pub mip: u32,
    /// Number of mip levels to use
    /// Use ALL_MIPS to use all remaining mip levels.
    pub mip_count: u32,
}

impl SubresourceRange {
    pub fn new(layer: u32, layer_count: u32, mip: u32, mip_count: u32) -> Self {
        Self { layer, layer_count, mip, mip_count }
    }
}

/// Native handle for platform-specific objects
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NativeHandle {
    pub handle_type: crate::NativeHandleType,
    pub value: u64,
}

impl NativeHandle {
    pub fn new(handle_type: crate::NativeHandleType, value: u64) -> Self {
        Self { handle_type, value }
    }

    pub fn is_valid(&self) -> bool {
        self.handle_type != crate::NativeHandleType::Undefined
    }
}

/// Descriptor handle for bindless resources
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorHandle {
    pub handle_type: crate::DescriptorHandleType,
    pub value: u64,
}

impl DescriptorHandle {
    pub fn new(handle_type: crate::DescriptorHandleType, value: u64) -> Self {
        Self { handle_type, value }
    }

    pub fn is_valid(&self) -> bool {
        self.handle_type != crate::DescriptorHandleType::Undefined
    }
}

impl Default for DescriptorHandle {
    fn default() -> Self {
        Self {
            handle_type: crate::DescriptorHandleType::Undefined,
            value: 0,
        }
    }
}

/// Window handle for different platforms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle {
    pub handle_type: crate::WindowHandleType,
    pub handle_values: [u64; 2],
}

impl WindowHandle {
    pub fn from_hwnd(hwnd: *mut std::ffi::c_void) -> Self {
        Self {
            handle_type: crate::WindowHandleType::HWND,
            handle_values: [hwnd as u64, 0],
        }
    }

    pub fn from_ns_window(ns_window: *mut std::ffi::c_void) -> Self {
        Self {
            handle_type: crate::WindowHandleType::NSWindow,
            handle_values: [ns_window as u64, 0],
        }
    }

    pub fn from_xlib_window(x_display: *mut std::ffi::c_void, x_window: u32) -> Self {
        Self {
            handle_type: crate::WindowHandleType::XlibWindow,
            handle_values: [x_display as u64, x_window as u64],
        }
    }
}

impl Default for WindowHandle {
    fn default() -> Self {
        Self {
            handle_type: crate::WindowHandleType::Undefined,
            handle_values: [0, 0],
        }
    }
}

/// Adapter LUID (Locally Unique Identifier)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AdapterLUID {
    pub luid: [u8; 16],
}

impl AdapterLUID {
    pub fn new(luid: [u8; 16]) -> Self {
        Self { luid }
    }
}

/// Color for debug markers
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl MarkerColor {
    pub fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    pub const RED: Self = Self { r: 1.0, g: 0.0, b: 0.0 };
    pub const GREEN: Self = Self { r: 0.0, g: 1.0, b: 0.0 };
    pub const BLUE: Self = Self { r: 0.0, g: 0.0, b: 1.0 };
    pub const WHITE: Self = Self { r: 1.0, g: 1.0, b: 1.0 };
    pub const BLACK: Self = Self { r: 0.0, g: 0.0, b: 0.0 };
}

impl Default for MarkerColor {
    fn default() -> Self {
        Self::WHITE
    }
}

/// Sample position for multisampling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SamplePosition {
    pub x: i8,
    pub y: i8,
}

impl SamplePosition {
    pub fn new(x: i8, y: i8) -> Self {
        Self { x, y }
    }
}
