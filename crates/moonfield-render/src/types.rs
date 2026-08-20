//! Public resource descriptions shared by Moonfield's Vulkan renderer.
//!
//! The descriptions remain independent of raw Vulkan handles so higher-level
//! renderer code does not need to construct ash types directly.

/// Pixel/color formats supported by the Vulkan renderer. Grow as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 8-bit BGRA unorm; the preferred swapchain and offscreen format.
    B8G8R8A8Unorm,
    /// 8-bit RGBA unorm.
    R8G8B8A8Unorm,
}

impl Format {
    /// Convert to the equivalent Vulkan format.
    pub(crate) fn to_vk(self) -> ash::vk::Format {
        match self {
            Self::B8G8R8A8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
            Self::R8G8B8A8Unorm => ash::vk::Format::R8G8B8A8_UNORM,
        }
    }

    /// Convert from a Vulkan format, returning `None` for formats without a
    /// neutral equivalent.
    pub(crate) fn from_vk(format: ash::vk::Format) -> Option<Self> {
        match format {
            ash::vk::Format::B8G8R8A8_UNORM => Some(Self::B8G8R8A8Unorm),
            ash::vk::Format::R8G8B8A8_UNORM => Some(Self::R8G8B8A8Unorm),
            _ => None,
        }
    }
}

/// Buffer usage flags. Const-fn combinable, no external deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferUsage(u32);

impl BufferUsage {
    /// The buffer can be bound as a vertex buffer.
    pub const VERTEX: Self = Self(1 << 0);
    /// The buffer can be bound as an index buffer.
    pub const INDEX: Self = Self(1 << 1);
    /// The buffer can be bound as a uniform buffer.
    pub const UNIFORM: Self = Self(1 << 2);
    /// The buffer can be bound as a storage buffer.
    pub const STORAGE: Self = Self(1 << 3);
    /// The buffer can be the destination of a copy.
    pub const COPY_DST: Self = Self(1 << 4);
    /// The buffer can be the source of a copy.
    pub const COPY_SRC: Self = Self(1 << 5);
    /// The buffer can back an indirect draw/dispatch command.
    pub const INDIRECT: Self = Self(1 << 6);

    /// An empty set of usage flags.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The union of two usage sets.
    pub const fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether all flags in `other` are set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for BufferUsage {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.bitor(rhs)
    }
}

impl BufferUsage {
    /// Convert to the equivalent Vulkan usage flags.
    pub(crate) fn to_vk(self) -> ash::vk::BufferUsageFlags {
        let mut flags = ash::vk::BufferUsageFlags::empty();
        if self.contains(Self::VERTEX) {
            flags |= ash::vk::BufferUsageFlags::VERTEX_BUFFER;
        }
        if self.contains(Self::INDEX) {
            flags |= ash::vk::BufferUsageFlags::INDEX_BUFFER;
        }
        if self.contains(Self::UNIFORM) {
            flags |= ash::vk::BufferUsageFlags::UNIFORM_BUFFER;
        }
        if self.contains(Self::STORAGE) {
            flags |= ash::vk::BufferUsageFlags::STORAGE_BUFFER;
        }
        if self.contains(Self::COPY_DST) {
            flags |= ash::vk::BufferUsageFlags::TRANSFER_DST;
        }
        if self.contains(Self::COPY_SRC) {
            flags |= ash::vk::BufferUsageFlags::TRANSFER_SRC;
        }
        if self.contains(Self::INDIRECT) {
            flags |= ash::vk::BufferUsageFlags::INDIRECT_BUFFER;
        }
        flags
    }
}

/// Vertex attribute formats supported by the Vulkan renderer. Grow as needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexFormat {
    /// Two 32-bit floats.
    Float32x2,
    /// Three 32-bit floats.
    Float32x3,
    /// Four 32-bit floats.
    Float32x4,
    /// One 32-bit unsigned integer (e.g. a packed RGBA color).
    Uint32,
}

impl VertexFormat {
    /// Convert to the equivalent Vulkan format.
    pub(crate) fn to_vk(self) -> ash::vk::Format {
        match self {
            Self::Float32x2 => ash::vk::Format::R32G32_SFLOAT,
            Self::Float32x3 => ash::vk::Format::R32G32B32_SFLOAT,
            Self::Float32x4 => ash::vk::Format::R32G32B32A32_SFLOAT,
            Self::Uint32 => ash::vk::Format::R32_UINT,
        }
    }
}

/// A single vertex attribute within a [`VertexBufferLayout`].
#[derive(Debug, Clone, Copy)]
pub struct VertexAttribute {
    /// The shader input location.
    pub location: u32,
    /// The attribute format.
    pub format: VertexFormat,
    /// Byte offset of the attribute within a vertex.
    pub offset: u32,
}

/// The layout of a single vertex buffer (binding 0, per-vertex input rate).
#[derive(Debug, Clone)]
pub struct VertexBufferLayout {
    /// Byte stride between consecutive vertices.
    pub stride: u32,
    /// The vertex attributes.
    pub attributes: Vec<VertexAttribute>,
}
