//! Frame-paged GPU scratch: per-frame allocations out of a bump-allocator
//! ring.
//!
//! Passes and draw commands write their per-frame data (view uniforms, per-
//! draw records) into the arena instead of per-frame dedicated buffers: each
//! frames-in-flight slot is a bump allocator, [`FrameDrawArena::begin_frame`]
//! frees the slot's previous allocations once the [frame
//! loop](crate::window) guarantees the GPU is done with that slot, and
//! allocation within a frame is pointer bumping.

use crate::window::MAX_FRAMES_IN_FLIGHT;
use moonfield_app::prelude::World;
use moonfield_log::error;
use moonfield_rhi::{BumpAlloc, Device, GpuBumpAllocator, RenderDevice, Result};
use std::sync::Mutex;

/// Byte size of one frame slot's scratch block.
pub const DRAW_ARENA_BLOCK: u64 = 1024 * 1024;

/// GPU scratch for one frame, one bump allocator per frames-in-flight slot.
pub struct FrameDrawArena {
    inner: Mutex<ArenaInner>,
}

struct ArenaInner {
    arenas: Vec<GpuBumpAllocator>,
    current: usize,
}

impl FrameDrawArena {
    /// Create the arena on the shared device, one block per frame slot.
    pub fn new(device: &Device) -> Result<Self> {
        let mut arenas = Vec::with_capacity(MAX_FRAMES_IN_FLIGHT);
        for _ in 0..MAX_FRAMES_IN_FLIGHT {
            arenas.push(GpuBumpAllocator::new(device, DRAW_ARENA_BLOCK)?);
        }
        Ok(Self {
            inner: Mutex::new(ArenaInner { arenas, current: 0 }),
        })
    }

    /// Free the slot's previous allocations and make it current. The frame
    /// loop calls this after its timeline wait guarantees the GPU is done
    /// with the slot.
    pub fn begin_frame(&self, slot: usize) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.arenas[slot].free_all();
        g.current = slot;
    }

    /// Allocate one `T` record from the current frame slot.
    pub fn alloc<T>(&self) -> Result<BumpAlloc> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = g.current;
        g.arenas[slot].alloc_typed::<T>(1)
    }
}

/// `PrepareViews` system: create the frame draw arena on first use and begin
/// its frame slot for this frame's allocations.
pub fn begin_frame_draw_arena(world: &mut World) {
    if !world.contains_resource::<FrameDrawArena>()
        && let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone())
    {
        match FrameDrawArena::new(render_device.device()) {
            Ok(arena) => {
                world.insert_resource(arena);
            }
            Err(e) => error!("failed to create frame draw arena: {e}"),
        }
    }
    let Some(frame_context) = world.get_resource::<crate::window::FrameContext>() else {
        return;
    };
    if !frame_context.frame_in_progress() {
        return;
    }
    let slot = frame_context.current_slot();
    if let Some(arena) = world.get_resource::<FrameDrawArena>() {
        arena.begin_frame(slot);
    }
}
