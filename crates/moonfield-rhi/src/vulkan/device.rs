//! Vulkan logical device abstraction.

use crate::error::{Error, Result};
use crate::retire::RetirementRing;
use crate::vulkan::instance::{Instance, InstanceShared};
use crate::vulkan::shader::ShaderCache;
use crate::vulkan::swapchain::Surface;
use crate::vulkan::sync::Semaphore;
use crate::{DESCRIPTOR_HEAP_IMAGE_CAPACITY, DESCRIPTOR_HEAP_SAMPLER_CAPACITY, DescriptorHeap};
use crate::{FrameUploader, UPLOAD_ARENA_SIZE};
use ash::vk::{self, TaggedStructure as _};
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use std::ffi::{CStr, c_char};
use std::sync::{Arc, Mutex, OnceLock};

// Required extensions are demanded unconditionally: the RHI targets recent
// drivers (current NVIDIA and AMD proprietary both expose them), so there is
// no fallback when one is missing — device creation fails with the missing
// names listed, instead of a bare `ERROR_EXTENSION_NOT_PRESENT`.
const REQUIRED_DEVICE_EXTENSIONS: &[&CStr] = &[
    ash::khr::swapchain::NAME,
    ash::ext::descriptor_heap::NAME,
    // `VkPipelineCreateFlags2CreateInfo` — the only way to flag a pipeline as
    // descriptor-heap-backed (`VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT`).
    ash::khr::maintenance5::NAME,
    // Shader-side descriptor-heap access (`ResourceDescriptorHeap[]` +
    // `spvDescriptorHeapEXT` lowers to untyped pointer chains that read the
    // bound heap directly). The RHI's bindless sampling path requires it.
    ash::khr::shader_untyped_pointers::NAME,
    ash::ext::extended_dynamic_state3::NAME,
    ash::ext::mesh_shader::NAME,
    // GPU-driven + bindless helpers. `mutable_descriptor_type` lets one
    // binding reuse a descriptor slot across types (fewer layouts, cheaper
    // binds); `vertex_input_dynamic_state` decouples vertex layouts from the
    // pipeline so a small pipeline set can serve many draw shapes.
    ash::ext::mutable_descriptor_type::NAME,
    ash::ext::vertex_input_dynamic_state::NAME,
    ash::ext::device_generated_commands::NAME,
];

// Optional extensions are performance enhancements or whole feature stacks,
// not prerequisites: they are enabled when the physical device exposes them,
// skipped with a warning otherwise. Callers query
// [`Device::optional_extension_enabled`] before relying on the feature.
//
// The ray-tracing stack is optional as a group: mesh rendering and the
// editor's core passes do not need it, and some real cards (Turing-class
// NVIDIA, e.g. T1000) do not expose the KHR RT extensions at all while
// software renderers (llvmpipe) do. `invocation_reorder` additionally needs
// Ampere-or-newer RT cores.
const OPTIONAL_DEVICE_EXTENSIONS: &[&CStr] = &[
    // The BVH container backing all RT work.
    ash::khr::acceleration_structure::NAME,
    ash::khr::ray_tracing_pipeline::NAME,
    ash::khr::ray_query::NAME,
    ash::khr::ray_tracing_position_fetch::NAME,
    // Shared prerequisites of the RT pipeline extensions.
    ash::khr::pipeline_library::NAME,
    ash::khr::deferred_host_operations::NAME,
    ash::ext::ray_tracing_invocation_reorder::NAME,
    // Address-based commands and the GPU-address timestamp resolve
    // (`VK_KHR_device_address_commands`). Optional as a group: indirect
    // draws/dispatches, `cmd_memcpy`, and query-pool resolves consume device
    // addresses (`GpuPtr`) with no handle-based fallback, and the extension
    // is absent on some real drivers (NVIDIA entry-Turing such as the dev
    // machine's T1000). Callers gate on
    // [`Device::device_address_commands`] before relying on those paths.
    ash::khr::device_address_commands::NAME,
    // Float32 atomic adds into storage buffers (`OpAtomicFAddEXT`) for the
    // ml gradient path. Enabled only when the driver also supports the
    // `shaderBufferFloat32AtomicAdd` feature bit — see the probe in
    // [`Device::from_physical_device`].
    ash::ext::shader_atomic_float::NAME,
    // With `unifiedImageLayouts` enabled, `VK_IMAGE_LAYOUT_GENERAL` — the
    // layout every non-swapchain image already lives in — is a
    // spec-guaranteed-optimal layout for nearly every use. Enabled only when
    // the driver also supports the `unifiedImageLayouts` feature bit; see the
    // probe in [`Device::from_physical_device`]. Swapchain images are
    // unaffected: `PRESENT_SRC_KHR` is exempt from the extension.
    ash::khr::unified_image_layouts::NAME,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorHeapProperties {
    pub max_resource_heap_size: u64,
    pub resource_heap_alignment: u64,
    pub image_descriptor_size: u64,
    pub image_descriptor_alignment: u64,
    /// Buffer descriptors share the resource heap with image descriptors;
    /// a resource slot is sized for the larger of the two.
    pub buffer_descriptor_size: u64,
    pub buffer_descriptor_alignment: u64,
    pub max_sampler_heap_size: u64,
    pub sampler_heap_alignment: u64,
    pub sampler_descriptor_size: u64,
    pub sampler_descriptor_alignment: u64,
    pub min_resource_heap_reserved_range: u64,
    pub min_sampler_heap_reserved_range: u64,
}

/// Queue family indices selected for graphics and presentation.
#[derive(Debug, Clone, Copy)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub present: u32,
    pub compute: u32,
}

impl QueueFamilyIndices {
    /// Find suitable queue families for a physical device.
    ///
    /// If `surface` is `None`, presentation support is not checked and
    /// `present` is set to the graphics index.
    pub(crate) fn find(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Self> {
        let properties = instance.queue_family_properties2(physical_device);

        let mut graphics = None;
        let mut present = None;
        let mut compute = None;

        for (index, props) in properties.iter().enumerate() {
            let index = index as u32;
            let flags = props.queue_family_properties.queue_flags;

            if graphics.is_none() && flags.contains(vk::QueueFlags::GRAPHICS) {
                graphics = Some(index);
            }

            if compute.is_none()
                && flags.contains(vk::QueueFlags::COMPUTE)
                && !flags.contains(vk::QueueFlags::GRAPHICS)
            {
                compute = Some(index);
            }

            if let Some(surface) = surface
                && present.is_none()
                && instance.get_physical_device_surface_support(physical_device, index, surface)
            {
                present = Some(index);
            }
        }

        let graphics =
            graphics.ok_or_else(|| Error::Unsupported("no graphics queue family".to_string()))?;
        let present = present.unwrap_or(graphics);
        let compute = compute.unwrap_or(graphics);

        Ok(Self {
            graphics,
            present,
            compute,
        })
    }

    /// Returns the unique queue family indices needed to create the device.
    pub fn unique_indices(&self) -> Vec<u32> {
        let mut indices = vec![self.graphics, self.present, self.compute];
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

/// The teardown-critical half of the device, shared by `Arc`: everything a
/// GPU object needs to destroy itself safely — the logical device handle, the
/// memory allocator, the retirement ring, and the extension loaders.
///
/// Every resource object (`Semaphore`, `CommandPool`, pipelines, textures,
/// allocations, …) holds an [`Arc<DeviceShared>`] through [`DeviceContext`],
/// so the logical device outlives every object created from it by
/// construction: `destroy_device` runs in this struct's `Drop`, which the
/// last resource drop triggers. The `instance` keepalive chains the same
/// guarantee one level up — the Vulkan instance outlives the device.
pub(crate) struct DeviceShared {
    /// Logical device handle. Only core commands; extension entry points are
    /// in `extension_fns`.
    device: ash::Device,
    /// Aggregated device-extension loaders (blend dynamic state etc.), built
    /// once at device creation. Command buffers reach them through
    /// [`DeviceContext`] — no per-command-buffer copies of the
    /// function-pointer tables.
    extension_fns: crate::vulkan::DeviceExtensionFunctions,
    /// Deferred GPU resource teardown, keyed by frame slot. Not lazy: every
    /// resource's `Drop` enqueues into it, so it exists from construction.
    retirement_ring: RetirementRing,
    /// Shared GPU memory allocator for buffers and images. Behind an
    /// `Arc<Mutex>` so retire actions can free their allocations at drain
    /// time; `Option` so `Drop` can take it out and destroy it while the
    /// device handle is still valid.
    allocator: Option<Arc<Mutex<Allocator>>>,
    /// Keepalive: the instance must outlive the logical device. Destroying
    /// an instance with live devices is invalid; holding the Arc makes that
    /// order unrepresentable instead of guarded against.
    #[allow(dead_code)]
    instance: Arc<InstanceShared>,
}

impl DeviceShared {
    /// Access the raw `ash::Device`.
    pub(crate) fn raw(&self) -> &ash::Device {
        &self.device
    }

    /// The shared aggregated device-extension loaders.
    pub(crate) fn extension_fns(&self) -> &crate::vulkan::DeviceExtensionFunctions {
        &self.extension_fns
    }

    /// The device-level retirement ring.
    pub(crate) fn ring(&self) -> &RetirementRing {
        &self.retirement_ring
    }

    /// Shared GPU memory allocator for buffers and images. Resources allocate
    /// through this and free their allocations on drop.
    pub(crate) fn allocator(&self) -> &Arc<Mutex<Allocator>> {
        self.allocator
            .as_ref()
            .expect("allocator taken only during device teardown")
    }
}

impl Drop for DeviceShared {
    fn drop(&mut self) {
        // This drop runs when the last `Device`/`DeviceContext` referent goes
        // away, so every resource object created from the device is already
        // destroyed and the remaining work is final teardown: idle the GPU
        // (in-flight frames may still reference retired-but-undrained
        // resources), drain the retirement ring, free the allocator's memory
        // blocks, then destroy the device. The instance Arc drops after this
        // body, so the instance outlives the destroy.
        // SAFETY: the device handle is valid; nothing is submitted after the
        // last referent is gone.
        if let Err(e) = unsafe { self.device.device_wait_idle() } {
            tracing::warn!("device idle wait failed during teardown: {e:?}");
        }
        self.retirement_ring.drain_all();
        // The allocator's memory blocks are freed through the device
        // (vkFreeMemory), so the allocator must drop before
        // `destroy_device`. Every allocation Arc was held by a resource that
        // also held a `DeviceContext`, so this Arc is uniquely held here;
        // the defensive branch leaks rather than destroying out from under a
        // hypothetical remaining referent (a leak is recoverable;
        // use-after-destroy is not).
        if let Some(allocator) = self.allocator.take() {
            match Arc::try_unwrap(allocator) {
                Ok(allocator) => drop(allocator),
                Err(allocator) => {
                    tracing::error!(
                        "device torn down while the allocator is still referenced; \
                         leaking the allocator and the device"
                    );
                    std::mem::forget(allocator);
                    return;
                }
            }
        }
        // SAFETY: the GPU is idle, all resources and allocations are gone,
        // and the destroy happens exactly once, here.
        unsafe {
            self.device.destroy_device(None);
        }
    }
}

/// A cloneable handle to the device's shared teardown state — the
/// crate-internal constructor argument and field type for every GPU object.
///
/// Cloning one keeps the logical device (and, through it, the instance)
/// alive, so an object that outlives the `Device` value it was created from
/// still destroys itself against a live device. This replaces the per-struct
/// `device + allocator + retirement ring` field triples.
#[derive(Clone)]
pub(crate) struct DeviceContext {
    shared: Arc<DeviceShared>,
}

impl std::ops::Deref for DeviceContext {
    type Target = DeviceShared;
    fn deref(&self) -> &DeviceShared {
        &self.shared
    }
}

/// Vulkan logical device and its primary queues.
pub struct Device {
    /// Shared teardown state; resources keep it alive through
    /// [`DeviceContext`]. Declared first so the OnceLock singletons below
    /// drop (and retire into the ring) before the shared state can.
    shared: Arc<DeviceShared>,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
    /// `VK_EXT_descriptor_heap` limits (its CPU-visible heap semantics). The
    /// RHI requires descriptor-heap support unconditionally — device creation
    /// fails where the driver does not implement it, like any missing
    /// required extension.
    descriptor_heap_properties: DescriptorHeapProperties,
    /// Nanoseconds per timestamp tick (`limits.timestampPeriod`), cached at
    /// creation for `TimestampQueryPool`.
    timestamp_period_ns: f32,
    /// Optional extensions that were actually enabled at creation (a subset
    /// of [`OPTIONAL_DEVICE_EXTENSIONS`]); empty on cards that lack them.
    optional_extensions: Vec<&'static CStr>,
    /// Lazily-built shared frame uploader serving GPU-only staging uploads.
    /// The uploader keeps the shared device state alive through its own
    /// [`DeviceContext`]; `Drop` releases this device's Arc early so a
    /// last-referent teardown destroys the arenas while the ring still
    /// drains.
    uploader: OnceLock<Arc<Mutex<FrameUploader>>>,
    /// Lazily-built shared descriptor heap serving bindless resources. Same
    /// shape as `uploader`: built once, shared by `Arc`, keeps the shared
    /// device state alive through its allocations' [`DeviceContext`]s.
    descriptor_heap: OnceLock<Arc<DescriptorHeap>>,
    /// Lazily-built shared shader cache: memoized Slang compiles (SPIR-V
    /// and reflection) keyed by the compile inputs, so repeated pipeline
    /// builds (and tests) compile each shader once per device.
    shader_cache: OnceLock<Arc<ShaderCache>>,
    /// Lazily-created Vulkan pipeline cache, seeded from disk and written
    /// back on drop. Passed to every pipeline create call so the driver
    /// skips recompiling pipelines it has already built in earlier runs.
    pipeline_cache: OnceLock<vk::PipelineCache>,
}

impl Device {
    /// Create a logical device for the first suitable physical device.
    ///
    /// If `surface` is provided, presentation support is required.
    pub fn new(instance: &Instance, surface: Option<&Surface>) -> Result<Self> {
        let physical_devices = instance.enumerate_physical_devices()?;
        if physical_devices.is_empty() {
            return Err(Error::Backend(
                "no Vulkan-capable physical devices found".to_string(),
            ));
        }

        // Prefer discrete GPU, then integrated, then any.
        let physical_device = physical_devices
            .iter()
            .copied()
            .min_by_key(|pd| {
                let mut props = vk::PhysicalDeviceProperties2::default();
                instance.physical_device_properties2(*pd, &mut props);
                match props.properties.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                    _ => 2,
                }
            })
            .ok_or_else(|| Error::Unsupported("no suitable physical device".to_string()))?;

        Self::from_physical_device(instance, physical_device, surface.map(Surface::raw))
    }

    /// Create a logical device from a specific physical device.
    pub(crate) fn from_physical_device(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Self> {
        let queue_family_indices = QueueFamilyIndices::find(instance, physical_device, surface)?;

        let unique_indices = queue_family_indices.unique_indices();
        let queue_priorities = [1.0f32];
        let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = unique_indices
            .iter()
            .map(|index| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*index)
                    .queue_priorities(&queue_priorities)
            })
            .collect();

        // Enumerate what the physical device actually exposes, so required
        // extensions fail with their names listed and optional ones are
        // skipped with a warning instead of a bare ERROR_EXTENSION_NOT_PRESENT.
        // SAFETY: the physical device and instance are valid; this is a read-only
        // enumeration of the device's extension list.
        let supported_extensions = unsafe {
            instance
                .raw()
                .enumerate_device_extension_properties(physical_device)
        }
        .map_err(|e| Error::Backend(format!("failed to enumerate device extensions: {e:?}")))?;
        let mut supported: Vec<&CStr> = supported_extensions
            .iter()
            .map(|props| unsafe { CStr::from_ptr(props.extension_name.as_ptr()) })
            .collect();

        let missing: Vec<&CStr> = REQUIRED_DEVICE_EXTENSIONS
            .iter()
            .copied()
            .filter(|name| !supported.contains(name))
            .collect();
        if !missing.is_empty() {
            return Err(Error::DeviceRequest(format!(
                "physical device is missing required extensions: {missing:?}"
            )));
        }

        // `VK_EXT_descriptor_heap` exposes a CPU-visible descriptor heap: the CPU
        // writes descriptor data straight into host-visible heap buffers
        // (`write_resource_descriptors`, `cmd_bind_resource_heap`). Some
        // drivers (e.g. NVIDIA) ship this implementation while reporting the
        // extension's original spec_version, so support is detected by the
        // property query itself: a driver that implements the heap fills
        // these fields, one that does not leaves them zero — and fails device
        // creation, matching the hard-requirement stance of
        // `REQUIRED_DEVICE_EXTENSIONS`.
        let props2 = vk::PhysicalDeviceProperties2::default();
        let mut heap_props = vk::PhysicalDeviceDescriptorHeapPropertiesEXT::default();
        // `TaggedStructure::push` consumes `self` and returns the chained
        // struct — the return value is the one that carries the pNext link,
        // so it must be rebound, not discarded.
        let mut props2 = props2.push(&mut heap_props);
        instance.physical_device_properties2(physical_device, &mut props2);
        // Nanoseconds per timestamp tick, for converting `TimestampQueryPool`
        // results to durations.
        let timestamp_period_ns = props2.properties.limits.timestamp_period;
        let descriptor_heap_properties =
            if heap_props.max_resource_heap_size > 0 && heap_props.image_descriptor_size > 0 {
                DescriptorHeapProperties {
                    max_resource_heap_size: heap_props.max_resource_heap_size,
                    resource_heap_alignment: heap_props.resource_heap_alignment,
                    image_descriptor_size: heap_props.image_descriptor_size,
                    image_descriptor_alignment: heap_props.image_descriptor_alignment,
                    buffer_descriptor_size: heap_props.buffer_descriptor_size,
                    buffer_descriptor_alignment: heap_props.buffer_descriptor_alignment,
                    max_sampler_heap_size: heap_props.max_sampler_heap_size,
                    sampler_heap_alignment: heap_props.sampler_heap_alignment,
                    sampler_descriptor_size: heap_props.sampler_descriptor_size,
                    sampler_descriptor_alignment: heap_props.sampler_descriptor_alignment,
                    min_resource_heap_reserved_range: heap_props.min_resource_heap_reserved_range,
                    min_sampler_heap_reserved_range: heap_props.min_sampler_heap_reserved_range,
                }
            } else {
                return Err(Error::DeviceRequest(
                    "physical device does not implement the VK_EXT_descriptor_heap \
                     CPU-visible descriptor heap (properties all zero)"
                        .to_string(),
                ));
            };

        // `VK_EXT_shader_atomic_float` is dropped from the optional candidates
        // when the driver lacks the buffer float32 atomic-add feature bit: the
        // extension alone does not imply the bit (llvmpipe exposes the
        // extension with every add bit false), and requesting an unsupported
        // feature would fail device creation. The generic optional loop below
        // then skips it with its standard warning.
        if supported.contains(&ash::ext::shader_atomic_float::NAME) {
            let features2 = vk::PhysicalDeviceFeatures2::default();
            let mut atomic_float = vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT::default();
            // `TaggedStructure::push` consumes `self` and returns the chained
            // struct — the return value is the one that carries the pNext
            // link, so it must be rebound, not discarded.
            let mut features2 = features2.push(&mut atomic_float);
            instance.physical_device_features2(physical_device, &mut features2);
            if atomic_float.shader_buffer_float32_atomic_add != vk::TRUE {
                supported.retain(|name| name != &ash::ext::shader_atomic_float::NAME);
            }
        }

        // `VK_KHR_unified_image_layouts` is dropped from the optional
        // candidates when the driver lacks the `unifiedImageLayouts` feature
        // bit: the extension name alone does not imply it, and requesting an
        // unsupported feature would fail device creation. Same probe shape as
        // the atomic-float check above.
        if supported.contains(&ash::khr::unified_image_layouts::NAME) {
            let features2 = vk::PhysicalDeviceFeatures2::default();
            let mut unified_layouts = vk::PhysicalDeviceUnifiedImageLayoutsFeaturesKHR::default();
            let mut features2 = features2.push(&mut unified_layouts);
            instance.physical_device_features2(physical_device, &mut features2);
            if unified_layouts.unified_image_layouts != vk::TRUE {
                supported.retain(|name| name != &ash::khr::unified_image_layouts::NAME);
            }
        }

        let mut optional_enabled: Vec<&'static CStr> = Vec::new();
        for name in OPTIONAL_DEVICE_EXTENSIONS {
            if supported.contains(name) {
                optional_enabled.push(name);
            } else {
                tracing::warn!("device extension {name:?} not supported; its feature is disabled");
            }
        }

        // The final enable list points into the `'static` constants above,
        // so the `*const c_char` array outlives the local `supported` list.
        // `VK_KHR_surface` is deliberately not listed: it is an *instance*
        // extension and NVIDIA rejects instance extensions in the device
        // enable list with ERROR_EXTENSION_NOT_PRESENT, even though the
        // validation layer's VUID 01387 wants swapchain's dependency named.
        let enabled_extensions: Vec<&'static CStr> = REQUIRED_DEVICE_EXTENSIONS
            .iter()
            .chain(optional_enabled.iter())
            .copied()
            .collect();
        let device_extension_names: Vec<*const c_char> = enabled_extensions
            .iter()
            .map(|name| name.as_ptr())
            .collect();

        let mut vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default()
            .buffer_device_address(true)
            .timeline_semaphore(true)
            .descriptor_indexing(true)
            .descriptor_binding_sampled_image_update_after_bind(true)
            .descriptor_binding_partially_bound(true)
            .descriptor_binding_variable_descriptor_count(true)
            .runtime_descriptor_array(true)
            .shader_sampled_image_array_non_uniform_indexing(true);
        let mut vulkan_13_features = vk::PhysicalDeviceVulkan13Features::default()
            .synchronization2(true)
            .dynamic_rendering(true);
        let mut vulkan_14_features =
            vk::PhysicalDeviceVulkan14Features::default().dynamic_rendering_local_read(true);
        let mut descriptor_heap_features =
            vk::PhysicalDeviceDescriptorHeapFeaturesEXT::default().descriptor_heap(true);
        let mut shader_untyped_pointers_features =
            vk::PhysicalDeviceShaderUntypedPointersFeaturesKHR::default()
                .shader_untyped_pointers(true);
        let mut extended_dynamic_state3_features =
            vk::PhysicalDeviceExtendedDynamicState3FeaturesEXT::default()
                .extended_dynamic_state3_color_blend_enable(true)
                .extended_dynamic_state3_color_blend_equation(true)
                .extended_dynamic_state3_color_write_mask(true);

        // Mesh shader features (VK_EXT_mesh_shader). Only `mesh_shader` is
        // requested: `task_shader` covers the separate task (amplification)
        // stage, which the RHI does not use yet.
        let mut mesh_shader_features =
            vk::PhysicalDeviceMeshShaderFeaturesEXT::default().mesh_shader(true);

        // Ray tracing stack (VK_KHR_acceleration_structure / ray tracing
        // pipeline / ray query / position fetch / EXT invocation reorder).
        // Only the core feature bit of each extension is requested; optional
        // bits (host commands, capture/replay, indirect build, …) stay off so
        // a device that exposes the extension but not the optional subfeature
        // can still create the device.
        let mut acceleration_structure_features =
            vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
                .acceleration_structure(true);
        let mut ray_tracing_pipeline_features =
            vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default().ray_tracing_pipeline(true);
        let mut ray_query_features =
            vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
        let mut position_fetch_features =
            vk::PhysicalDeviceRayTracingPositionFetchFeaturesKHR::default()
                .ray_tracing_position_fetch(true);
        let mut invocation_reorder_features =
            vk::PhysicalDeviceRayTracingInvocationReorderFeaturesEXT::default()
                .ray_tracing_invocation_reorder(true);

        // GPU-driven + bindless: mutable descriptor bindings and dynamic
        // vertex input (see the extension comment in `DEVICE_EXTENSIONS`).
        let mut mutable_descriptor_type_features =
            vk::PhysicalDeviceMutableDescriptorTypeFeaturesEXT::default()
                .mutable_descriptor_type(true);
        let mut vertex_input_dynamic_state_features =
            vk::PhysicalDeviceVertexInputDynamicStateFeaturesEXT::default()
                .vertex_input_dynamic_state(true);
        let mut device_generate_commands_features =
            vk::PhysicalDeviceDeviceGeneratedCommandsFeaturesEXT::default()
                .device_generated_commands(true);
        // Address-based commands (VK_KHR_device_address_commands): the whole
        // feature is one bit — address forms of indirect draw/dispatch,
        // memory copies, and query resolves.
        let mut device_address_commands_features =
            vk::PhysicalDeviceDeviceAddressCommandsFeaturesKHR::default()
                .device_address_commands(true);

        // Float32 atomic adds into storage buffers. Only the buffer add bit
        // is requested — the RHI uses no other operation from the extension.
        let mut shader_atomic_float_features =
            vk::PhysicalDeviceShaderAtomicFloatFeaturesEXT::default()
                .shader_buffer_float32_atomic_add(true);

        // `VK_KHR_unified_image_layouts`: only the `unifiedImageLayouts` bit
        // is requested — `unifiedImageLayoutsVideo` covers video coding,
        // which the RHI does not use. With the bit set, `GENERAL` (the layout
        // every non-swapchain image already lives in) is a
        // spec-guaranteed-optimal layout for nearly every use.
        let mut unified_image_layouts_features =
            vk::PhysicalDeviceUnifiedImageLayoutsFeaturesKHR::default().unified_image_layouts(true);

        // Core features. Storage-image access without a format qualifier:
        // heap-indexed `RWTexture2D` (the gaussian-splatting intermediate)
        // has no declaration site to annotate a format, so untyped storage
        // reads and writes are both requested.
        let mut features2 = vk::PhysicalDeviceFeatures2::default().features(
            vk::PhysicalDeviceFeatures::default()
                .shader_storage_image_write_without_format(true)
                .shader_storage_image_read_without_format(true),
        );
        // Feature structures of optional extensions are requested only when the
        // extension was enabled, so the request matches the enable list
        // exactly (drivers ignore structures whose extension they never saw).
        // The RT feature structs are gated as a stack; see
        // [`OPTIONAL_DEVICE_EXTENSIONS`].
        if optional_enabled.contains(&ash::khr::acceleration_structure::NAME) {
            features2 = features2.push(&mut acceleration_structure_features);
        }
        if optional_enabled.contains(&ash::khr::ray_tracing_pipeline::NAME) {
            features2 = features2.push(&mut ray_tracing_pipeline_features);
        }
        if optional_enabled.contains(&ash::khr::ray_query::NAME) {
            features2 = features2.push(&mut ray_query_features);
        }
        if optional_enabled.contains(&ash::khr::ray_tracing_position_fetch::NAME) {
            features2 = features2.push(&mut position_fetch_features);
        }
        if optional_enabled.contains(&ash::ext::ray_tracing_invocation_reorder::NAME) {
            features2 = features2.push(&mut invocation_reorder_features);
        }
        if optional_enabled.contains(&ash::ext::shader_atomic_float::NAME) {
            features2 = features2.push(&mut shader_atomic_float_features);
        }
        if optional_enabled.contains(&ash::khr::device_address_commands::NAME) {
            features2 = features2.push(&mut device_address_commands_features);
        }
        if optional_enabled.contains(&ash::khr::unified_image_layouts::NAME) {
            features2 = features2.push(&mut unified_image_layouts_features);
        }
        // `TaggedStructure::push` consumes `self` and returns the chained struct —
        // the return value is the one that carries the pNext link, so the
        // whole core chain must be rebound, not discarded (the optional RT
        // structs above already reassign).
        features2 = features2
            .push(&mut vulkan_12_features)
            .push(&mut vulkan_13_features)
            .push(&mut vulkan_14_features)
            .push(&mut descriptor_heap_features)
            .push(&mut shader_untyped_pointers_features)
            .push(&mut extended_dynamic_state3_features)
            .push(&mut mesh_shader_features)
            .push(&mut mutable_descriptor_type_features)
            .push(&mut vertex_input_dynamic_state_features)
            .push(&mut device_generate_commands_features);

        // `push` requires a chainless `next`, but `features2` heads the whole
        // feature chain built above — merge that chain with `extend` instead.
        let mut create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extension_names);
        // SAFETY: `features2` and the structs behind it are valid, writable
        // Vulkan feature structures for the lifetime of the create call.
        unsafe {
            create_info = create_info.extend(&mut features2);
        }

        let device = unsafe {
            instance
                .raw()
                .create_device(physical_device, &create_info, None)
        }
        .map_err(|e| Error::Backend(format!("failed to create logical device: {:?}", e)))?;

        let graphics_queue = unsafe { device.get_device_queue(queue_family_indices.graphics, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_family_indices.present, 0) };

        let extension_fns = crate::vulkan::DeviceExtensionFunctions {
            extended_dynamic_state3: ash::ext::extended_dynamic_state3::Device::load(
                instance.raw(),
                &device,
            ),
            descriptor_heap: ash::ext::descriptor_heap::Device::load(instance.raw(), &device),
            // Loaded only when the extension was enabled (see
            // [`OPTIONAL_DEVICE_EXTENSIONS`]); the commands behind it panic
            // with a clear message when `None`, so callers gate on
            // [`Device::device_address_commands`].
            device_address_commands: optional_enabled
                .contains(&ash::khr::device_address_commands::NAME)
                .then(|| ash::khr::device_address_commands::Device::load(instance.raw(), &device)),
        };

        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.raw().clone(),
            device: device.clone(),
            physical_device,
            debug_settings: Default::default(),
            // Device enables `bufferDeviceAddress` (Vulkan 1.2 core) for the
            // bindless GPU pointer model; the allocator must match or
            // allocations cannot back a buffer device address.
            buffer_device_address: true,
            allocation_sizes: Default::default(),
        })
        .map_err(|e| Error::Backend(format!("failed to create GPU allocator: {e}")))?;

        Ok(Self {
            shared: Arc::new(DeviceShared {
                device,
                extension_fns,
                retirement_ring: RetirementRing::new(),
                allocator: Some(Arc::new(Mutex::new(allocator))),
                instance: instance.shared(),
            }),
            physical_device,
            graphics_queue,
            present_queue,
            queue_family_indices,
            descriptor_heap_properties,
            timestamp_period_ns,
            optional_extensions: optional_enabled,
            uploader: OnceLock::new(),
            descriptor_heap: OnceLock::new(),
            shader_cache: OnceLock::new(),
            pipeline_cache: OnceLock::new(),
        })
    }

    /// Access the raw `ash::Device`.
    pub(crate) fn raw(&self) -> &ash::Device {
        self.shared.raw()
    }

    /// A handle to the device's shared teardown state. Crate-internal: every
    /// GPU object's constructor clones one into itself, keeping the device
    /// (and instance) alive for the object's whole lifetime.
    pub(crate) fn context(&self) -> DeviceContext {
        DeviceContext {
            shared: self.shared.clone(),
        }
    }

    /// Whether the optional extension `name` was enabled at device creation.
    /// False when the physical device does not expose it — the skip is
    /// surfaced as a warning during creation, so callers can degrade the
    /// feature instead of failing.
    pub fn optional_extension_enabled(&self, name: &CStr) -> bool {
        self.optional_extensions.contains(&name)
    }

    /// Whether shaders can perform float32 atomic adds into storage buffers
    /// (`OpAtomicFAddEXT` behind `VK_EXT_shader_atomic_float`; in Slang the
    /// `__atomic_add` intrinsic). The ml gradient-accumulation path requires
    /// it; `gpu_tests::float_atomics` probes it end to end.
    pub fn buffer_float32_atomic_add(&self) -> bool {
        self.optional_extensions
            .contains(&ash::ext::shader_atomic_float::NAME)
    }

    /// Whether address-based GPU commands are available
    /// (`VK_KHR_device_address_commands`): indirect draw/dispatch from device
    /// addresses, `cmd_memcpy`, and GPU-address timestamp resolves. The
    /// extension is optional — absent on some real drivers (NVIDIA
    /// entry-Turing such as the dev machine's T1000) — so
    /// [`CommandBuffer`](crate::CommandBuffer)'s address-command methods
    /// panic and [`TimestampQueryPool`](crate::TimestampQueryPool) creation
    /// fails when it is missing; gate callers on this query.
    pub fn device_address_commands(&self) -> bool {
        self.optional_extensions
            .contains(&ash::khr::device_address_commands::NAME)
    }

    /// The shared aggregated device-extension loaders (see
    /// [`DeviceExtensionFunctions`](crate::vulkan::DeviceExtensionFunctions)).
    /// Command buffers reach them through their [`DeviceContext`], never by
    /// copying the function-pointer tables.
    pub(crate) fn extension_fns(&self) -> &crate::vulkan::DeviceExtensionFunctions {
        self.shared.extension_fns()
    }

    /// Access the underlying physical device handle.
    pub(crate) fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    /// Access the graphics queue.
    pub(crate) fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    /// Submit recorded command buffers to the graphics queue and block until
    /// they complete. Test and upload-path convenience — frame loops use the
    /// window systems' semaphores/fences instead.
    ///
    /// When the shared uploader has submitted batches, this submission waits
    /// on its latest one: same-queue submission order sequences execution
    /// but creates no memory dependency, so staged uploads need the timeline
    /// wait to be visible to shader reads.
    pub fn submit_and_wait(&self, command_buffers: &[&crate::CommandBuffer]) -> Result<()> {
        let command_infos: Vec<vk::CommandBufferSubmitInfo> = command_buffers
            .iter()
            .map(|buffer| vk::CommandBufferSubmitInfo::default().command_buffer(buffer.raw()))
            .collect();
        let wait_infos: Vec<vk::SemaphoreSubmitInfo> = self
            .uploader
            .get()
            .map(|uploader| {
                uploader
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pending_signal()
                    .into_iter()
                    .map(|(semaphore, value)| {
                        vk::SemaphoreSubmitInfo::default()
                            .semaphore(semaphore.raw())
                            .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                            .value(value)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let submit_info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&command_infos);
        // SAFETY: the command buffers are fully recorded; the queue, the
        // uploader's timeline semaphore (kept alive by the `uploader`
        // singleton), and the info arrays are valid and outlive the call.
        unsafe {
            self.shared
                .raw()
                .queue_submit2(
                    self.graphics_queue,
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit command buffers: {e:?}")))?;
            self.shared
                .raw()
                .queue_wait_idle(self.graphics_queue)
                .map_err(|e| Error::Backend(format!("failed to wait for queue: {e:?}")))?;
        }
        Ok(())
    }

    /// Submit the frame's command buffer to the graphics queue: wait on
    /// `wait_semaphores` (binary acquire signals, at the color-attachment
    /// stage) and `timeline_waits` (the uploader's latest batch and other
    /// producer timelines, at every stage), signal `signal_semaphores`
    /// (binary present signals) plus `timeline` with `signal_value`. An
    /// offscreen-only frame passes empty semaphore slices.
    pub fn submit_frame_timeline(
        &self,
        command_buffer: &crate::CommandBuffer,
        wait_semaphores: &[&Semaphore],
        signal_semaphores: &[&Semaphore],
        timeline_waits: &[(&Semaphore, u64)],
        timeline: &Semaphore,
        signal_value: u64,
    ) -> Result<()> {
        let mut wait_infos: Vec<vk::SemaphoreSubmitInfo> = wait_semaphores
            .iter()
            .map(|semaphore| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(semaphore.raw())
                    .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            })
            .collect();
        wait_infos.extend(timeline_waits.iter().map(|(semaphore, value)| {
            vk::SemaphoreSubmitInfo::default()
                .semaphore(semaphore.raw())
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .value(*value)
        }));
        // Binary semaphores ignore the value field (placeholder 0); the
        // timeline's value is the signal value.
        let mut signal_infos: Vec<vk::SemaphoreSubmitInfo> = signal_semaphores
            .iter()
            .map(|semaphore| {
                vk::SemaphoreSubmitInfo::default()
                    .semaphore(semaphore.raw())
                    .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .value(0)
            })
            .collect();
        signal_infos.push(
            vk::SemaphoreSubmitInfo::default()
                .semaphore(timeline.raw())
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .value(signal_value),
        );
        let command_infos =
            [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.raw())];
        let submit_info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&command_infos)
            .signal_semaphore_infos(&signal_infos);
        // SAFETY: the queue, command buffer, and semaphores are valid handles;
        // the info arrays outlive the submit call.
        unsafe {
            self.shared
                .raw()
                .queue_submit2(
                    self.graphics_queue,
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit frame: {e:?}")))?;
        }
        Ok(())
    }

    /// Block until the device is idle (all queued work complete).
    pub fn wait_idle(&self) -> Result<()> {
        // SAFETY: the device is valid.
        unsafe {
            self.shared
                .raw()
                .device_wait_idle()
                .map_err(|e| Error::Backend(format!("failed to wait for device idle: {e:?}")))
        }
    }

    /// Access the presentation queue.
    pub(crate) fn present_queue(&self) -> vk::Queue {
        self.present_queue
    }

    /// Access the selected queue family indices.
    pub fn queue_family_indices(&self) -> QueueFamilyIndices {
        self.queue_family_indices
    }

    /// The physical device's `VK_EXT_descriptor_heap` limits, which the RHI
    /// requires unconditionally (device creation fails without them).
    /// `DescriptorHeap` sizes its heaps and computes slot strides from these.
    pub fn descriptor_heap_properties(&self) -> DescriptorHeapProperties {
        self.descriptor_heap_properties
    }

    /// Nanoseconds per timestamp tick (`limits.timestampPeriod`), the
    /// conversion factor for `TimestampQueryPool` results.
    pub(crate) fn timestamp_period_ns(&self) -> f32 {
        self.timestamp_period_ns
    }

    /// The shared frame-scoped uploader, built on first use. GPU-only
    /// targets stage through it (`FrameUploader::upload_alloc`); the uploader
    /// keeps the device alive through its own `DeviceContext`, so callers
    /// may hold the returned `Arc` past this `&Device` borrow.
    pub fn uploader(&self) -> Arc<Mutex<FrameUploader>> {
        self.uploader
            .get_or_init(|| {
                Arc::new(Mutex::new(
                    FrameUploader::new(self, UPLOAD_ARENA_SIZE)
                        .expect("failed to create the shared frame uploader"),
                ))
            })
            .clone()
    }

    pub fn descriptor_heap(&self) -> Arc<DescriptorHeap> {
        self.descriptor_heap
            .get_or_init(|| {
                Arc::new(
                    DescriptorHeap::new(
                        self,
                        DESCRIPTOR_HEAP_IMAGE_CAPACITY,
                        DESCRIPTOR_HEAP_SAMPLER_CAPACITY,
                    )
                    .expect("failed to create the shared descriptor heap"),
                )
            })
            .clone()
    }

    /// The shared shader cache, built on first use. Pipeline constructors
    /// compile through it; the cache memoizes SPIR-V and reflection by the
    /// compile inputs, so repeated builds compile each shader once.
    pub fn shader_cache(&self) -> Arc<ShaderCache> {
        self.shader_cache
            .get_or_init(|| {
                Arc::new(ShaderCache::new().expect("failed to create the shared shader cache"))
            })
            .clone()
    }

    /// The shared Vulkan pipeline cache (crate-internal: pipeline
    /// constructors pass it to their create calls). Seeded from
    /// `<cache dir>/moonfield/pipeline_cache.bin`; `Drop` writes the merged
    /// data back. A stale or corrupt file only costs a cold start.
    pub(crate) fn pipeline_cache(&self) -> vk::PipelineCache {
        *self.pipeline_cache.get_or_init(|| {
            let initial = std::fs::read(pipeline_cache_path()).unwrap_or_default();
            let create_info = vk::PipelineCacheCreateInfo::default().initial_data(&initial);
            match unsafe { self.shared.raw().create_pipeline_cache(&create_info, None) } {
                Ok(cache) => cache,
                Err(e) => {
                    tracing::warn!("pipeline cache data rejected ({e:?}); starting cold");
                    unsafe {
                        self.shared
                            .raw()
                            .create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None)
                            .expect("creating an empty pipeline cache cannot fail")
                    }
                }
            }
        })
    }

    /// Frame-loop boundary: the caller has waited the in-flight timeline,
    /// so the slot's previous submission completed — drain its retirements
    /// and mark it the push target. Call once per acquired frame, before
    /// recording.
    pub fn begin_gpu_frame(&self, frame_slot: usize) {
        self.shared.ring().begin_frame(frame_slot);
    }
    /// Drain every retirement now. The GPU must be idle (device teardown,
    /// or a test after submit-and-wait); in-flight work must not reference
    /// retired resources.
    pub fn flush_retirements(&self) {
        self.shared.ring().drain_all();
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // The logical device itself is destroyed in `DeviceShared::drop`,
        // when the last `DeviceContext` referent goes away — resources that
        // outlive this `Device` value keep it alive by construction, so no
        // leak guard is needed here anymore. What remains is teardown that
        // belongs to this handle: persisting the pipeline cache and dropping
        // the lazy singletons early, so a last-referent drop destroys their
        // arenas and heap backing while the ring can still drain them.
        if let Some(cache) = self.pipeline_cache.get() {
            match unsafe { self.shared.raw().get_pipeline_cache_data(*cache) } {
                Ok(data) => {
                    let path = pipeline_cache_path();
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Err(e) = std::fs::write(&path, &data) {
                        tracing::warn!("failed to write the pipeline cache: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to read the pipeline cache data: {e:?}")
                }
            }
            // SAFETY: the cache was created by this device and is destroyed
            // exactly once, here; the device is alive (held by `shared`).
            unsafe {
                self.shared.raw().destroy_pipeline_cache(*cache, None);
            }
        }
        // Dropping the Arcs destroys the singletons only when this device is
        // their last referent; otherwise their own `DeviceContext` keeps the
        // shared state alive until they go away.
        if let Some(uploader) = self.uploader.take() {
            drop(uploader);
        }
        if let Some(heap) = self.descriptor_heap.take() {
            drop(heap);
        }
    }
}

/// Where the pipeline cache lives on disk: the per-user cache dir —
/// `%LOCALAPPDATA%` (falling back to `%APPDATA%`) on Windows,
/// `$XDG_CACHE_HOME` (falling back to `~/.cache`) elsewhere, and the temp dir
/// when no cache root is set — plus `moonfield/pipeline_cache.bin`. The
/// temp-dir fallback keeps the path absolute: an empty root would otherwise
/// resolve against the process's working directory.
fn pipeline_cache_path() -> std::path::PathBuf {
    #[cfg(windows)]
    let dir = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(not(windows))]
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir);
    dir.join("moonfield").join("pipeline_cache.bin")
}
