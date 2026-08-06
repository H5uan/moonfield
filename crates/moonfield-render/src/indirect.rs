//! Cross-backend indirect-draw/dispatch argument layouts.
//!
//! These `#[repr(C)]` structs are the **neutral canonical layout** for
//! GPU-driven indirect commands: their field order and sizes match both the
//! Vulkan command structs (`vk::DrawIndirectCommand`, etc.) and the wgpu arg
//! structs (`wgpu::DrawIndirectArgs`, etc.), so a single buffer populated via
//! `bytemuck::bytes_of` is valid on either backend without per-backend
//! conversion. The RHI does not expose raw transmutation to backend types —
//! callers write these structs into a [`Buffer`](crate::native::Buffer) with
//! [`BufferUsage::INDIRECT`](crate::BufferUsage::INDIRECT) and pass it to the
//! command buffer's indirect draw methods.
//!
//! Compute indirect dispatch is reserved (`DispatchIndirectArgs` is defined
//! here for forward compatibility) but not yet wired into a command — there is
//! no `ComputePipeline` in the RHI yet.

use crate::types::BufferUsage;

/// Argument buffer layout for non-indexed `draw_indirect` commands.
///
/// 16 bytes; matches `vk::DrawIndirectCommand` and `wgpu::DrawIndirectArgs`.
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
/// 20 bytes; matches `vk::DrawIndexedIndirectCommand` and
/// `wgpu::DrawIndexedIndirectArgs`.
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
/// 12 bytes; matches `vk::DispatchIndirectCommand` and
/// `wgpu::DispatchIndirectArgs`.
///
/// *Reserved for compute:* defined here for forward compatibility, but not yet
/// wired into a RHI command because there is no `ComputePipeline` yet.
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

#[cfg(feature = "native")]
impl IndexFormat {
    /// Convert to the equivalent Vulkan index type.
    #[allow(dead_code)] // used by tests/indirect_draw via raw vk::IndexType; kept for API symmetry with the web backend
    pub(crate) fn to_vk(self) -> ash::vk::IndexType {
        match self {
            Self::Uint16 => ash::vk::IndexType::UINT16,
            Self::Uint32 => ash::vk::IndexType::UINT32,
        }
    }
}

#[cfg(feature = "web")]
impl IndexFormat {
    /// Convert to the equivalent wgpu index format.
    pub(crate) fn to_wgpu(self) -> wgpu::IndexFormat {
        match self {
            Self::Uint16 => wgpu::IndexFormat::Uint16,
            Self::Uint32 => wgpu::IndexFormat::Uint32,
        }
    }
}

// Keep `BufferUsage` referenced so the module compiles even when only the
// arg structs are exercised; the INDIRECT flag is consumed by the indirect
// command paths.
const _: fn() = || {
    let _ = BufferUsage::INDIRECT;
};
