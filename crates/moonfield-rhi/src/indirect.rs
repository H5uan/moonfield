//! Vulkan indirect-draw/dispatch argument layouts.
//!
//! These `#[repr(C)]` structs mirror Vulkan's command structs
//! (`vk::DrawIndirectCommand`, etc.), so a buffer populated via
//! `bytemuck::bytes_of` can be submitted directly. The RHI does not expose raw
//! transmutation to Vulkan types — callers write these structs into a
//! [`Buffer`](crate::vulkan::Buffer) with
//! [`BufferUsage::INDIRECT`](crate::BufferUsage::INDIRECT) and pass it to the
//! command buffer's indirect draw methods.
//!
//! Compute indirect dispatch is wired through the bindless path:
//! [`CommandBuffer::dispatch_indirect`](crate::CommandBuffer::dispatch_indirect)
//! takes a [`GpuAllocation`](crate::vulkan::bindless::GpuAllocation) holding
//! these arguments.

use crate::types::BufferUsage;

/// Argument buffer layout for non-indexed `draw_indirect` commands.
///
/// 16 bytes; matches `vk::DrawIndirectCommand`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndirectArgs {
    /// The number of vertices to draw.
    pub vertex_count: u32,
    /// The number of instances to draw.
    pub instance_count: u32,
    /// The index of the first vertex to draw.
    pub first_vertex: u32,
    /// The instance ID of the first instance. Must be 0 unless the backend's
    /// `INDIRECT_FIRST_INSTANCE` capability is enabled.
    pub first_instance: u32,
}

/// Argument buffer layout for indexed `draw_indexed_indirect` commands.
///
/// 20 bytes; matches `vk::DrawIndexedIndirectCommand`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndexedIndirectArgs {
    /// The number of indices to draw.
    pub index_count: u32,
    /// The number of instances to draw.
    pub instance_count: u32,
    /// The first index within the index buffer.
    pub first_index: u32,
    /// The value added to the vertex index before indexing into the vertex
    /// buffer.
    pub base_vertex: i32,
    /// The instance ID of the first instance. Must be 0 unless the backend's
    /// `INDIRECT_FIRST_INSTANCE` capability is enabled.
    pub first_instance: u32,
}

/// Argument buffer layout for `dispatch_workgroups_indirect` commands.
///
/// 12 bytes; matches `vk::DispatchIndirectCommand`. Consumed by
/// [`CommandBuffer::dispatch_indirect`](crate::CommandBuffer::dispatch_indirect).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DispatchIndirectArgs {
    /// The number of work groups in the X dimension.
    pub x: u32,
    /// The number of work groups in the Y dimension.
    pub y: u32,
    /// The number of work groups in the Z dimension.
    pub z: u32,
}

/// The element width of an index buffer, used by indexed indirect draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFormat {
    /// 16-bit unsigned indices.
    Uint16,
    /// 32-bit unsigned indices.
    Uint32,
}

impl IndexFormat {
    /// Convert to the equivalent Vulkan index type.
    pub(crate) fn to_vk(self) -> ash::vk::IndexType {
        match self {
            Self::Uint16 => ash::vk::IndexType::UINT16,
            Self::Uint32 => ash::vk::IndexType::UINT32,
        }
    }
}

// Keep `BufferUsage` referenced so the module compiles even when only the
// arg structs are exercised; the INDIRECT flag is consumed by the indirect
// command paths.
const _: fn() = || {
    let _ = BufferUsage::INDIRECT;
};
