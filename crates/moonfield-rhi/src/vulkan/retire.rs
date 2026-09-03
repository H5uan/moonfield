//! Deferred GPU resource retirement, keyed by frame slot.
//!
//! [`RetirementRing`] is the RHI's single lifetime mechanism for resources an
//! in-flight frame may still reference: textures, buffers, allocations, and
//! descriptor-heap slots. Dropping such a resource does not destroy it — the
//! resource's `Drop` pushes teardown steps into the current frame slot's
//! queue, and the frame loop drains that queue `RETIRE_RING` frames later,
//! after its in-flight wait has passed. `Device::drop` drains whatever
//! remains once the device is idle.
//!

use crate::vulkan::descriptor_heap::{DescriptorHeap, SamplerHandle, TextureHandle};
use ash::vk;
use gpu_allocator::vulkan::{Allocation, Allocator};
use std::sync::{Arc, Mutex};

/// Retirement queue depth, in frame slots. Equal to the frame loop's
/// frames-in-flight (render-core's `MAX_FRAMES_IN_FLIGHT`).
pub const RETIRE_RING: usize = 2;

/// One atomic teardown step, executed at drain time. A resource's `Drop`
/// composes its teardown out of these; nothing else runs at drain.
///
/// Crate-internal: constructed by the resource `Drop` implementations in
/// `memory`, `buffer`, `texture`, and `offscreen`.
// TODO(M4): remove once every variant has a constructor.
#[allow(dead_code)]
pub(crate) enum RetireAction {
    /// Return an image slot to the heap's freelist. Carries the view's
    /// create info — the heap encodes it by pointer (see `TextureSlotDesc`),
    /// so it must stay alive until the slot is freed.
    ImageSlot {
        heap: Arc<DescriptorHeap>,
        handle: TextureHandle,
        view_create_info: vk::ImageViewCreateInfo<'static>,
    },
    /// Return a sampler slot to the heap's freelist.
    SamplerSlot {
        heap: Arc<DescriptorHeap>,
        handle: SamplerHandle,
    },
    /// Destroy an image view and image, then free the allocation.
    Image {
        device: ash::Device,
        view: vk::ImageView,
        image: vk::Image,
        allocation: Option<Allocation>,
        allocator: Arc<Mutex<Allocator>>,
    },
    /// Destroy a buffer, then free the allocation.
    Buffer {
        device: ash::Device,
        buffer: vk::Buffer,
        allocation: Option<Allocation>,
        allocator: Arc<Mutex<Allocator>>,
    },
}

impl RetireAction {
    /// Execute the teardown step. Failures are logged, not propagated —
    /// drain runs inside the frame loop, which has no error channel.
    fn run(self) {
        match self {
            Self::ImageSlot { heap, handle, .. } => {
                // The view create info drops with this binding, after the
                // slot is returned: the pointer it lends stays valid for the
                // descriptor's whole encoded lifetime.
                if let Err(e) = heap.free_image_slot(handle) {
                    moonfield_log::error!("failed to free retired image slot: {e}");
                }
            }
            Self::SamplerSlot { heap, handle } => {
                if let Err(e) = heap.free_sampler_slot(handle) {
                    moonfield_log::error!("failed to free retired sampler slot: {e}");
                }
            }
            Self::Image {
                device,
                view,
                image,
                allocation,
                allocator,
            } => {
                // SAFETY: no in-flight work references the image —
                // `begin_frame` drains after the slot's fence, `drain_all`
                // requires an idle device.
                unsafe {
                    device.destroy_image_view(view, None);
                    device.destroy_image(image, None);
                }
                if let Some(allocation) = allocation
                    && let Err(e) = allocator
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .free(allocation)
                {
                    moonfield_log::error!("failed to free retired image allocation: {e}");
                }
            }
            Self::Buffer {
                device,
                buffer,
                allocation,
                allocator,
            } => {
                // SAFETY: as `Image`.
                unsafe {
                    device.destroy_buffer(buffer, None);
                }
                if let Some(allocation) = allocation
                    && let Err(e) = allocator
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .free(allocation)
                {
                    moonfield_log::error!("failed to free retired buffer allocation: {e}");
                }
            }
        }
    }
}

struct Inner {
    /// One queue per frame slot.
    slots: Vec<Vec<RetireAction>>,
    /// The frame slot `push` targets; set by `begin_frame`.
    current: usize,
}

pub(crate) struct RetirementRing {
    inner: Mutex<Inner>,
}

impl RetirementRing {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                slots: (0..RETIRE_RING).map(|_| Vec::new()).collect(),
                current: 0,
            }),
        }
    }

    pub(crate) fn begin_frame(&self, slot: usize) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = slot % RETIRE_RING;
        for action in std::mem::take(&mut inner.slots[slot]) {
            action.run();
        }
        inner.current = slot;
    }

    pub(crate) fn push(&self, action: RetireAction) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let current = inner.current;
        inner.slots[current].push(action);
    }

    pub(crate) fn drain_all(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for slot in &mut inner.slots {
            for action in std::mem::take(slot) {
                action.run();
            }
        }
    }
}

impl Default for RetirementRing {
    fn default() -> Self {
        Self::new()
    }
}
