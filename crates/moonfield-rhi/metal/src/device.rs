//! Metal 4 device: command queue, device-wide residency set, command
//! allocator, and the `MTLSharedEvent` timeline that drives CPU-GPU
//! synchronization.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use moonfield_rhi_core::{Error, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTL4CommandAllocator, MTL4CommandQueue, MTLAllocation, MTLDevice, MTLEvent, MTLResidencySet,
    MTLResidencySetDescriptor, MTLSharedEvent,
};

use crate::command::CommandBuffer;
use crate::instance::Instance;

/// Teardown-critical device state, shared by every GPU object through
/// [`DeviceContext`] (the same shared-ownership shape as the Vulkan
/// backend's `DeviceShared`).
pub(crate) struct DeviceShared {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    /// The single graphics/transfer queue; Metal queues are cheap but the
    /// RHI's frame loop assumes one submission stream.
    queue: Retained<ProtocolObject<dyn MTL4CommandQueue>>,
    /// Device-wide residency set: every allocation registers itself here on
    /// creation, so all live resources are resident for every command buffer
    /// without per-pass bookkeeping. The set lives for the device's lifetime.
    residency_set: Retained<ProtocolObject<dyn MTLResidencySet>>,
    /// Serves one command buffer at a time; recreated per pool allocation.
    command_allocator: Retained<ProtocolObject<dyn MTL4CommandAllocator>>,
    /// Timeline event: the queue signals monotonically increasing values
    /// after committed work completes; the CPU waits on a value to know the
    /// work finished. Held as `MTLSharedEvent` for the CPU wait; the queue
    /// signal takes the base `MTLEvent` view (see [`Device::submit_and_wait`]).
    event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    next_event_value: AtomicU64,
    residency_lock: Mutex<()>,
}

impl DeviceShared {
    pub(crate) fn device(&self) -> &ProtocolObject<dyn MTLDevice> {
        &self.device
    }

    pub(crate) fn queue(&self) -> &ProtocolObject<dyn MTL4CommandQueue> {
        &self.queue
    }

    pub(crate) fn command_allocator(&self) -> &ProtocolObject<dyn MTL4CommandAllocator> {
        &self.command_allocator
    }

    /// Add an allocation to the device-wide residency set and make the
    /// addition visible to Metal.
    pub(crate) fn register_allocation(&self, allocation: &ProtocolObject<dyn MTLAllocation>) {
        let _guard = self.residency_lock.lock().unwrap();
        self.residency_set.addAllocation(allocation);
        self.residency_set.commit();
    }

    /// Reserve the next timeline value.
    pub(crate) fn next_value(&self) -> u64 {
        self.next_event_value.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn event(&self) -> &ProtocolObject<dyn MTLSharedEvent> {
        &self.event
    }
}

/// Cloneable handle to the device's teardown-critical state.
#[derive(Clone)]
pub(crate) struct DeviceContext {
    shared: Arc<DeviceShared>,
}

impl DeviceContext {
    pub(crate) fn shared(&self) -> &DeviceShared {
        &self.shared
    }
}

/// A Metal 4 device: queue, residency set, and timeline in one entry point.
#[derive(Clone)]
pub struct Device {
    ctx: DeviceContext,
}

impl Device {
    /// Create the queue, residency set, command allocator, and timeline
    /// event on a gated Metal 4 device.
    pub fn new(instance: &Instance) -> Result<Self> {
        let device = instance.device();

        let queue = device
            .newMTL4CommandQueue()
            .ok_or_else(|| Error::AdapterRequest("failed to create an MTL4CommandQueue".into()))?;

        let residency_set = {
            let descriptor = MTLResidencySetDescriptor::new();
            device
                .newResidencySetWithDescriptor_error(&descriptor)
                .map_err(|err| Error::Backend(format!("{err:?}")))?
        };
        queue.addResidencySet(&residency_set);

        let command_allocator = device.newCommandAllocator().ok_or_else(|| {
            Error::AdapterRequest("failed to create an MTL4CommandAllocator".into())
        })?;

        let event = device
            .newSharedEvent()
            .ok_or_else(|| Error::AdapterRequest("failed to create an MTLSharedEvent".into()))?;

        Ok(Self {
            ctx: DeviceContext {
                // The Arc gives shared ownership and drop-time teardown, not
                // cross-thread access: GPU objects live on the main thread
                // (the RHI's standing ownership rule).
                #[allow(clippy::arc_with_non_send_sync)]
                shared: Arc::new(DeviceShared {
                    device: device.clone(),
                    queue,
                    residency_set,
                    command_allocator,
                    event,
                    next_event_value: AtomicU64::new(0),
                    residency_lock: Mutex::new(()),
                }),
            },
        })
    }

    pub(crate) fn ctx(&self) -> &DeviceContext {
        &self.ctx
    }

    /// Submit one command buffer and block until its GPU work completes,
    /// via the timeline event (queue signal + CPU wait).
    pub fn submit_and_wait(&self, mut buffer: CommandBuffer) -> Result<()> {
        let shared = self.ctx.shared();
        buffer.end();
        let value = shared.next_value();
        let buffers = [buffer.raw()];
        // The queue signal takes the base `MTLEvent` view of the shared event.
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(shared.event());
        // SAFETY: `buffers` is a valid array of live command buffers with
        // matching count 1; each element layout matches the expected
        // `NonNull<ProtocolObject>` (Retained is a NonNull wrapper).
        unsafe {
            shared
                .queue()
                .commit_count(NonNull::from(&buffers).cast(), 1);
            shared.queue().signalEvent_value(event, value);
        }
        if shared
            .event()
            .waitUntilSignaledValue_timeoutMS(value, 10_000)
        {
            Ok(())
        } else {
            Err(Error::Backend("timed out waiting for GPU work".into()))
        }
    }
}
