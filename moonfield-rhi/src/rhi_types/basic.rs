//! Basic types and structures used throughout the RHI

use crate::{Backend, Format, FormatKind};

pub type BufferAddress = u64;

pub type BufferSize = core::num::NonZeroU64;

pub type ShaderLocation = u32;

pub type DynamicOffset = u32;

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

pub struct FormatInfo {
    format: Format,
    name: &'static str,
    slang_name: Option<&'static str>,
    kind: FormatKind,
    channel_count: u8,
    channel_type: u8,
    block_size_in_bytes: u8,
    pixel_per_block: u8,
    block_width: u8,
    block_height: u8,

    has_read: bool,
    has_green: bool,
    has_blue: bool,
    has_alpha: bool,
    has_depth: bool,
    has_stencil: bool,
    is_signed: bool,
    is_srgb: bool,
    is_compressed: bool,
    supports_non_power_of_two: bool,
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

/// Window handle for different platforms using raw-window-handle
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowHandle {
    handle: raw_window_handle::RawWindowHandle,
}

impl WindowHandle {
    /// Create a new WindowHandle from any object that implements HasRawWindowHandle
    pub fn new<W: raw_window_handle::HasRawWindowHandle>(
        window: &W,
    ) -> Result<Self, raw_window_handle::HandleError> {
        Ok(Self { handle: window.raw_window_handle()? })
    }

    /// Get the raw window handle
    pub fn get_handle(&self) -> raw_window_handle::RawWindowHandle {
        self.handle
    }
}

impl Default for WindowHandle {
    fn default() -> Self {
        Self {
            handle: raw_window_handle::RawWindowHandle::Web(
                raw_window_handle::WebWindowHandle::new(0),
            ),
        }
    }
}

unsafe impl raw_window_handle::HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(
        &self,
    ) -> Result<
        raw_window_handle::RawWindowHandle,
        raw_window_handle::HandleError,
    > {
        Ok(self.handle)
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

/// Clear value for depth/stencil attachments
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthStencilClearValue {
    pub depth: f32,
    pub stencil: u32,
}

impl Default for DepthStencilClearValue {
    fn default() -> Self {
        Self { depth: 1.0, stencil: 0 }
    }
}

/// Clear value for color attachments
#[derive(Clone, Copy)]
pub union ColorClearValue {
    pub float_values: [f32; 4],
    pub uint_values: [u32; 4],
    pub int_values: [i32; 4],
}

impl Default for ColorClearValue {
    fn default() -> Self {
        Self { float_values: [0.0, 0.0, 0.0, 0.0] }
    }
}

impl std::fmt::Debug for ColorClearValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            f.debug_struct("ColorClearValue")
                .field("float_values", &self.float_values)
                .finish()
        }
    }
}

impl PartialEq for ColorClearValue {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.float_values == other.float_values }
    }
}

/// Combined clear value for any attachment type
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClearValue {
    pub color: ColorClearValue,
    pub depth_stencil: DepthStencilClearValue,
}

/// Supported physical device types.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DeviceType {
    /// Other or Unknown.
    Other,
    /// Integrated GPU with shared CPU/GPU memory.
    IntegratedGpu,
    /// Discrete GPU with separate CPU/GPU memory.
    DiscreteGpu,
    /// Cpu / Software Rendering.
    Cpu,
}

/// Information about an adapter (GPU/CPU).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AdapterInfo {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: DeviceType,
    pub driver: String,
    pub driver_info: String,
    pub backend: Backend,
    pub transient_saves_memory: bool,
}

bitflags_array! {
    #[repr(C)]
    #[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
    pub struct Features: [u64; 2];
}

/// Represents the sets of limits an adapter/device supports.
#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Limits {}

#[derive(Clone, Debug,)]
pub struct Alignments {
    pub buffer_copy_offset: BufferSize,
    pub buffer_copy_pitch: BufferSize,
    pub uniform_bounds_check_alignment: BufferSize,
    pub raw_tlas_instance_size: usize,
    pub ray_tracing_scratch_buffer_alignment: u32,
}

#[derive(Clone, Debug)]
pub struct Capabilities {
    pub limits: Limits,
    pub alignments: Alignments,
}