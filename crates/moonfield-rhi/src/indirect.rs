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
//! takes a [`GpuAllocation`](crate::vulkan::memory::GpuAllocation) holding
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

// Keep `BufferUsage` referenced so the module compiles even when only the
// arg structs are exercised; the INDIRECT flag is consumed by the indirect
// command paths.
const _: fn() = || {
    let _ = BufferUsage::INDIRECT;
};
