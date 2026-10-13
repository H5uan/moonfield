//! Vulkan synchronization primitives.

use super::device::{Device, DeviceContext};
use crate::Stage;
use crate::error::{Error, Result};
use ash::vk::{self, TaggedStructure as _};
use std::sync::Arc;

/// A Vulkan semaphore. Crate-internal: the public synchronization surface is
/// the device's frame [`Timeline`]; binary semaphores serve only the
/// swapchain's per-slot acquire/present pairs and the uploader's timeline.
pub(crate) struct Semaphore {
    semaphore: vk::Semaphore,
    /// Keeps the device alive until the semaphore is destroyed in `Drop`.
    ctx: DeviceContext,
}

impl Semaphore {
    /// Create a new binary semaphore.
    pub(crate) fn new(device: &Device) -> Result<Self> {
        Self::new_ctx(device.context())
    }

    /// Create a new binary semaphore from the shared device state, for
    /// constructors that hold a [`DeviceContext`] rather than a `&Device`
    /// (the swapchain's slot rebuild).
    pub(crate) fn new_ctx(ctx: DeviceContext) -> Result<Self> {
        let create_info = vk::SemaphoreCreateInfo::default();
        // SAFETY: the device is valid and the default create info describes a
        // legal binary semaphore.
        let semaphore = unsafe {
            ctx.raw()
                .create_semaphore(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create semaphore: {:?}", e)))?
        };

        Ok(Self { semaphore, ctx })
    }

    /// Access the raw `vk::Semaphore` handle.
    pub(crate) fn raw(&self) -> vk::Semaphore {
        self.semaphore
    }

    pub(crate) fn new_timeline(device: &Device, initial_value: u64) -> Result<Self> {
        Self::new_timeline_ctx(device.context(), initial_value)
    }

    pub(crate) fn new_timeline_ctx(ctx: DeviceContext, initial_value: u64) -> Result<Self> {
        let mut type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial_value);
        let create_info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
        // SAFETY: the device is valid and the type-chained create info
        // describes a legal timeline semaphore; `type_info` outlives the call.
        let semaphore = unsafe {
            ctx.raw()
                .create_semaphore(&create_info, None)
                .map_err(|e| {
                    Error::Backend(format!("failed to create timeline semaphore: {:?}", e))
                })?
        };
        Ok(Self { semaphore, ctx })
    }

    /// Block the CPU until the timeline counter reaches at least `value`.
    pub(crate) fn wait(&self, value: u64, timeout_ns: u64) -> Result<()> {
        let wait_info = vk::SemaphoreWaitInfo::default()
            .semaphores(std::slice::from_ref(&self.semaphore))
            .values(std::slice::from_ref(&value));
        // SAFETY: the semaphore is a live timeline semaphore owned by `self`,
        // and the wait info's semaphore and value slices have matching lengths.
        unsafe {
            self.ctx
                .raw()
                .wait_semaphores(&wait_info, timeout_ns)
                .map_err(|e| {
                    Error::Backend(format!("failed to wait for timeline semaphore: {:?}", e))
                })?;
        }
        Ok(())
    }

    /// The timeline counter's current value (`vkGetSemaphoreCounterValue`).
    pub(crate) fn counter_value(&self) -> Result<u64> {
        // SAFETY: the semaphore is a live timeline semaphore owned by `self`.
        unsafe { self.ctx.raw().get_semaphore_counter_value(self.semaphore) }
            .map_err(|e| Error::Backend(format!("failed to read timeline semaphore: {:?}", e)))
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        // SAFETY: the semaphore was created by this device and is destroyed
        // exactly once, here; `ctx` keeps the device alive.
        unsafe {
            self.ctx.raw().destroy_semaphore(self.semaphore, None);
        }
    }
}

/// The device's frame-submission timeline: a cheap, cloneable handle to the
/// timeline semaphore every
/// [`Device::submit_frame`](crate::Device::submit_frame) signals with the
/// frame number. Clones share the same timeline; the internal semaphore
/// keeps the device alive.
#[derive(Clone)]
pub struct Timeline {
    semaphore: Arc<Semaphore>,
}

impl Timeline {
    pub(crate) fn new(semaphore: Semaphore) -> Self {
        Self {
            semaphore: Arc::new(semaphore),
        }
    }

    /// Block the CPU until the timeline counter reaches at least `point`.
    pub fn wait(&self, point: u64) -> Result<()> {
        self.semaphore.wait(point, u64::MAX)
    }

    /// Poll: whether the timeline counter has reached `point`.
    pub fn completed(&self, point: u64) -> bool {
        // A dead device reports no progress; the next `wait` surfaces the
        // error.
        self.semaphore.counter_value().unwrap_or(0) >= point
    }

    /// Access the raw `vk::Semaphore` handle.
    pub(crate) fn raw(&self) -> vk::Semaphore {
        self.semaphore.raw()
    }
}

/// Map a [`Stage`] mask onto a sync2 (stage, access) pair.
///
/// Barriers are stage-only by contract: under bindless, shaders address
/// memory through pointers, so a barrier cannot name the resource whose
/// accesses it orders — the access masks are derived from the stage mask
/// instead. The derivation is conservative (widen, never narrow): a stage
/// gets every access its work could plausibly perform, so a stage pair that
/// covers the hazard is always a correct barrier.
///
/// `MEMORY_READ | MEMORY_WRITE` are legal alongside any stage per the Vulkan
/// spec, so the graphics-widening stages pair with them; shader stages get
/// the shader access triplet (reads, writes, and sampled reads through the
/// descriptor heap), and transfer gets transfer read/write. An empty mask
/// maps to an execution-only dependency.
pub(crate) fn stage_to_vk(stage: Stage) -> (vk::PipelineStageFlags2, vk::AccessFlags2) {
    if stage.contains(Stage::ALL) {
        return (
            vk::PipelineStageFlags2::ALL_COMMANDS,
            vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
        );
    }
    let mut stages = vk::PipelineStageFlags2::empty();
    let mut access = vk::AccessFlags2::NONE;
    if stage.contains(Stage::ALL_GRAPHICS) {
        // A render pass's work spans shader, fragment-test, and attachment
        // stages; the honest access mask for it is the spec-legal-anywhere
        // widest pair.
        stages |= vk::PipelineStageFlags2::ALL_GRAPHICS;
        access |= vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE;
    }
    const SHADER_ACCESS: vk::AccessFlags2 = vk::AccessFlags2::from_raw(
        vk::AccessFlags2::SHADER_READ.as_raw()
            | vk::AccessFlags2::SHADER_WRITE.as_raw()
            | vk::AccessFlags2::SHADER_SAMPLED_READ.as_raw(),
    );
    if stage.contains(Stage::VERTEX) {
        stages |= vk::PipelineStageFlags2::VERTEX_SHADER;
        access |= SHADER_ACCESS;
    }
    if stage.contains(Stage::FRAGMENT) {
        stages |= vk::PipelineStageFlags2::FRAGMENT_SHADER;
        access |= SHADER_ACCESS;
    }
    if stage.contains(Stage::COMPUTE) {
        stages |= vk::PipelineStageFlags2::COMPUTE_SHADER;
        access |= SHADER_ACCESS;
    }
    if stage.contains(Stage::TRANSFER) {
        stages |= vk::PipelineStageFlags2::TRANSFER;
        access |= vk::AccessFlags2::TRANSFER_READ | vk::AccessFlags2::TRANSFER_WRITE;
    }
    (stages, access)
}

/// A timestamp query pool: GPU timestamps recorded by
/// [`CommandBuffer::write_timestamp`](crate::CommandBuffer::write_timestamp)
/// and resolved straight to a GPU address by
/// [`CommandBuffer::resolve_timestamps`](crate::CommandBuffer::resolve_timestamps)
/// (`vkCmdCopyQueryPoolResultsToMemoryKHR`). Results are `u64` tick counts
/// the CPU reads from the resolved allocation's host mapping after the
/// submission's timeline point — no `vkGetQueryPoolResults` host sync.
/// Multiply tick deltas by [`timestamp_period_ns`](Self::timestamp_period_ns)
/// for nanoseconds.
///
/// Test-only for now: timestamps left the public surface with the sync
/// contract narrowing and return behind an explicit capability when a
/// consumer exists.
#[cfg(test)]
pub(crate) struct TimestampQueryPool {
    pool: vk::QueryPool,
    /// Keeps the device alive until the pool is destroyed in `Drop`.
    ctx: DeviceContext,
    count: u32,
    timestamp_period_ns: f32,
}

#[cfg(test)]
impl TimestampQueryPool {
    /// Create a pool of `count` timestamp queries.
    ///
    /// Requires `VK_KHR_device_address_commands` (query
    /// [`Device::device_address_commands`]): the pool's only read path,
    /// [`CommandBuffer::resolve_timestamps`](crate::CommandBuffer::resolve_timestamps),
    /// resolves straight to a GPU address through that extension, so creating
    /// a pool on a device without it fails instead of building a pool whose
    /// results can never be read.
    pub(crate) fn new(device: &Device, count: u32) -> Result<Self> {
        if !device.device_address_commands() {
            return Err(Error::Unsupported(
                "timestamp pools resolve through VK_KHR_device_address_commands, \
                 which this device does not support (see Device::device_address_commands)"
                    .to_string(),
            ));
        }
        let create_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(count);
        // SAFETY: the device is valid and the create info describes a legal
        // timestamp query pool.
        let pool = unsafe {
            device
                .raw()
                .create_query_pool(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create query pool: {:?}", e)))?
        };
        Ok(Self {
            pool,
            ctx: device.context(),
            count,
            timestamp_period_ns: device.timestamp_period_ns(),
        })
    }

    /// Nanoseconds per timestamp tick (`limits.timestampPeriod` of the
    /// physical device this pool was created on).
    pub(crate) fn timestamp_period_ns(&self) -> f32 {
        self.timestamp_period_ns
    }

    /// Number of queries in the pool.
    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    /// Access the raw `vk::QueryPool` handle.
    pub(crate) fn raw(&self) -> vk::QueryPool {
        self.pool
    }
}

#[cfg(test)]
impl Drop for TimestampQueryPool {
    fn drop(&mut self) {
        // SAFETY: the pool was created by this device and is destroyed exactly
        // once, here; `ctx` keeps the device alive.
        unsafe {
            self.ctx.raw().destroy_query_pool(self.pool, None);
        }
    }
}
