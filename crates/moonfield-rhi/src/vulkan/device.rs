//! Vulkan logical device abstraction.

use crate::error::{Error, Result};
use crate::retire::RetirementRing;
use crate::vulkan::instance::Instance;
use crate::vulkan::shader::ShaderCache;
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
    pub fn find(
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

/// Vulkan logical device and its primary queues.
pub struct Device {
    physical_device: vk::PhysicalDevice,
    /// Logical device handle. only have core command.
    device: ash::Device,
    graphics_queue: vk::Queue,
    compute_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
    /// `VK_EXT_descriptor_heap` limits (its CPU-visible heap semantics). The
    /// RHI requires descriptor-heap support unconditionally — device creation
    /// fails where the driver does not implement it, like any missing
    /// required extension.
    descriptor_heap_properties: DescriptorHeapProperties,
    /// Aggregated device-extension loaders (blend dynamic state etc.), built
    /// once at device creation and shared with command buffers by `Arc` — no
    /// per-command-buffer copies of the function-pointer tables.
    extension_fns: Arc<crate::vulkan::DeviceExtensionFunctions>,
    /// Optional extensions that were actually enabled at creation (a subset
    /// of [`OPTIONAL_DEVICE_EXTENSIONS`]); empty on cards that lack them.
    optional_extensions: Vec<&'static CStr>,
    /// Lazily-built shared frame uploader serving GPU-only staging uploads.
    /// Declared before `allocator` so it drops first: its arenas free chunks
    /// through the allocator's `Arc` while the device is still alive, then
    /// the allocator itself is torn down (see `Drop`).
    uploader: OnceLock<Arc<Mutex<FrameUploader>>>,
    /// Lazily-built shared descriptor heap serving bindless resources. Same shape
    /// as `uploader`: built once, shared by `Arc`, outlives `&Device`.
    descriptor_heap: OnceLock<Arc<DescriptorHeap>>,
    /// Lazily-built shared shader cache: memoized Slang compiles (SPIR-V
    /// and reflection) keyed by the compile inputs, so repeated pipeline
    /// builds (and tests) compile each shader once per device.
    shader_cache: OnceLock<Arc<ShaderCache>>,
    /// Lazily-created Vulkan pipeline cache, seeded from disk and written
    /// back on drop. Passed to every pipeline create call so the driver
    /// skips recompiling pipelines it has already built in earlier runs.
    pipeline_cache: OnceLock<vk::PipelineCache>,
    /// Deferred GPU resource teardown, keyed by frame slot. Not lazy: every
    /// resource's `Drop` enqueues into it, so it exists from construction.
    retirement_ring: Arc<RetirementRing>,
    /// Shared GPU memory allocator for buffers and images. Wrapped in
    /// `Arc<Mutex>` so resources can hold clones and free their allocations
    /// on drop without a borrow on the device. `Option` so `Drop` can take it
    /// out and destroy it while the device handle is still valid.
    allocator: Option<Arc<Mutex<Allocator>>>,
    /// The instance's live-device counter, shared with the `Instance` that
    /// created this device. Decremented exactly when `Drop` destroys the
    /// handle; the leak guard (see `Drop`) deliberately keeps the count so
    /// `Instance::drop` leaks instead of destroying around a live device.
    live_devices: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Device {
    /// Create a logical device for the first suitable physical device.
    ///
    /// If `surface` is provided, presentation support is required.
    pub fn new(instance: &Instance, surface: Option<vk::SurfaceKHR>) -> Result<Self> {
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

        Self::from_physical_device(instance, physical_device, surface)
    }

    /// Create a logical device from a specific physical device.
    pub fn from_physical_device(
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
        let supported: Vec<&CStr> = supported_extensions
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

        let mut optional_enabled: Vec<&'static CStr> = Vec::new();
        for name in OPTIONAL_DEVICE_EXTENSIONS {
            if supported.contains(name) {
                optional_enabled.push(name);
            } else {
                moonfield_log::warn!(
                    "device extension {name:?} not supported; its feature is disabled"
                );
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

        let mut features2 =
            vk::PhysicalDeviceFeatures2::default().features(vk::PhysicalDeviceFeatures::default());
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
        let compute_queue = unsafe { device.get_device_queue(queue_family_indices.compute, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_family_indices.present, 0) };

        let extension_fns = Arc::new(crate::vulkan::DeviceExtensionFunctions {
            extended_dynamic_state3: ash::ext::extended_dynamic_state3::Device::load(
                instance.raw(),
                &device,
            ),
            descriptor_heap: ash::ext::descriptor_heap::Device::load(instance.raw(), &device),
        });

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

        // Register with the instance's live-device counter: `Instance::drop`
        // refuses to destroy an instance whose devices are still alive, and
        // this device's `Drop` decrements exactly when it destroys its
        // handle.
        let live_devices = instance.live_devices();
        live_devices.fetch_add(1, std::sync::atomic::Ordering::Release);

        Ok(Self {
            physical_device,
            device,
            graphics_queue,
            compute_queue,
            present_queue,
            queue_family_indices,
            descriptor_heap_properties,
            extension_fns,
            optional_extensions: optional_enabled,
            uploader: OnceLock::new(),
            descriptor_heap: OnceLock::new(),
            shader_cache: OnceLock::new(),
            pipeline_cache: OnceLock::new(),
            retirement_ring: Arc::new(RetirementRing::new()),
            live_devices,
            allocator: Some(Arc::new(Mutex::new(allocator))),
        })
    }

    /// Access the raw `ash::Device`.
    pub fn raw(&self) -> &ash::Device {
        &self.device
    }

    /// Whether the optional extension `name` was enabled at device creation.
    /// False when the physical device does not expose it — the skip is
    /// surfaced as a warning during creation, so callers can degrade the
    /// feature instead of failing.
    pub fn optional_extension_enabled(&self, name: &CStr) -> bool {
        self.optional_extensions.contains(&name)
    }

    /// The shared aggregated device-extension loaders (see
    /// [`DeviceExtensionFunctions`](crate::vulkan::DeviceExtensionFunctions)).
    /// Command buffers clone the `Arc`, never the function-pointer tables.
    pub(crate) fn extension_fns(&self) -> Arc<crate::vulkan::DeviceExtensionFunctions> {
        self.extension_fns.clone()
    }

    /// Access the underlying physical device handle.
    pub fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    /// Access the graphics queue.
    pub fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    /// Submit recorded command buffers to the graphics queue and block until
    /// they complete. Test and upload-path convenience — frame loops use the
    /// window systems' semaphores/fences instead.
    pub fn submit_and_wait(&self, command_buffers: &[&crate::CommandBuffer]) -> Result<()> {
        let raw: Vec<vk::CommandBuffer> =
            command_buffers.iter().map(|buffer| buffer.raw()).collect();
        let submit_info = vk::SubmitInfo::default().command_buffers(&raw);
        // SAFETY: the command buffers are fully recorded and the queue is valid.
        unsafe {
            self.device
                .queue_submit(
                    self.graphics_queue,
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit command buffers: {e:?}")))?;
            self.device
                .queue_wait_idle(self.graphics_queue)
                .map_err(|e| Error::Backend(format!("failed to wait for queue: {e:?}")))?;
        }
        Ok(())
    }

    pub fn submit_frame_timeline(
        &self,
        command_buffer: &crate::CommandBuffer,
        wait_semaphore: &Semaphore,
        signal_semaphore: &Semaphore,
        timeline: &Semaphore,
        signal_value: u64,
    ) -> Result<()> {
        let wait_infos = [vk::SemaphoreSubmitInfo::default()
            .semaphore(wait_semaphore.raw())
            .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
        // binary 的 value 字段被忽略，占位 0；timeline 的 value 就是 signal 值。
        let signal_infos = [
            vk::SemaphoreSubmitInfo::default()
                .semaphore(signal_semaphore.raw())
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .value(0),
            vk::SemaphoreSubmitInfo::default()
                .semaphore(timeline.raw())
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .value(signal_value),
        ];
        let command_infos =
            [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.raw())];
        let submit_info = vk::SubmitInfo2::default()
            .wait_semaphore_infos(&wait_infos)
            .command_buffer_infos(&command_infos)
            .signal_semaphore_infos(&signal_infos);
        unsafe {
            self.device
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
            self.device
                .device_wait_idle()
                .map_err(|e| Error::Backend(format!("failed to wait for device idle: {e:?}")))
        }
    }

    /// Access the compute queue.
    pub fn compute_queue(&self) -> vk::Queue {
        self.compute_queue
    }

    /// Access the presentation queue.
    pub fn present_queue(&self) -> vk::Queue {
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

    /// The shared frame-scoped uploader, built on first use. GPU-only
    /// targets stage through it (`FrameUploader::upload_alloc`); the uploader
    /// owns a copy of the device handle, so callers may hold the returned
    /// `Arc` past this `&Device` borrow.
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
            match unsafe { self.device.create_pipeline_cache(&create_info, None) } {
                Ok(cache) => cache,
                Err(e) => {
                    moonfield_log::warn!("pipeline cache data rejected ({e:?}); starting cold");
                    unsafe {
                        self.device
                            .create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None)
                            .expect("creating an empty pipeline cache cannot fail")
                    }
                }
            }
        })
    }

    /// The device-level retirement ring. Crate-internal: resources obtain it
    /// at construction so their `Drop` can enqueue teardown.
    pub(crate) fn retirement_ring(&self) -> Arc<RetirementRing> {
        self.retirement_ring.clone()
    }

    /// Frame-loop boundary: the caller has waited the in-flight timeline,
    /// so the slot's previous submission completed — drain its retirements
    /// and mark it the push target. Call once per acquired frame, before
    /// recording.
    pub fn begin_gpu_frame(&self, frame_slot: usize) {
        self.retirement_ring.begin_frame(frame_slot);
    }
    /// Drain every retirement now. The GPU must be idle (device teardown,
    /// or a test after submit-and-wait); in-flight work must not reference
    /// retired resources.
    pub fn flush_retirements(&self) {
        self.retirement_ring.drain_all();
    }

    /// Shared GPU memory allocator for buffers and images. Resources allocate
    /// through this and free their allocations on drop. Exposed so downstream
    /// code (e.g. the editor's egui backend) can share the same allocator.
    pub fn allocator(&self) -> &Arc<Mutex<Allocator>> {
        self.allocator
            .as_ref()
            .expect("allocator taken only during device drop")
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // Retired teardown runs while the logical device is alive, and the
        // GPU must be idle first — resources dropped by render-world
        // teardown wait here for the last RenderDevice referent to go away.
        let _ = self.wait_idle();
        // Persist the pipeline cache: the merged data saves driver-side
        // pipeline compilation on the next run. Before the teardown paths
        // below, while the device is still fully functional.
        if let Some(cache) = self.pipeline_cache.get() {
            match unsafe { self.device.get_pipeline_cache_data(*cache) } {
                Ok(data) => {
                    let path = pipeline_cache_path();
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if let Err(e) = std::fs::write(&path, &data) {
                        moonfield_log::warn!("failed to write the pipeline cache: {e}");
                    }
                }
                Err(e) => {
                    moonfield_log::warn!("failed to read the pipeline cache data: {e:?}")
                }
            }
            unsafe {
                self.device.destroy_pipeline_cache(*cache, None);
            }
        }
        // Drop the lazy singletons while the device is alive: their arenas
        // and heap backing retire into the ring and drain below, instead of
        // tearing down after vkDestroyDevice during field teardown. Dropping
        // the Arc destroys them only when this device is the last referent —
        // the same invariant the allocator Arc relies on.
        if let Some(uploader) = self.uploader.take() {
            drop(uploader);
        }
        if let Some(heap) = self.descriptor_heap.take() {
            drop(heap);
        }
        self.retirement_ring.drain_all();
        // The allocator's memory blocks must be freed while the logical
        // device is still alive (they call vkFreeMemory through it), so the
        // device is destroyed only after the last allocation Arc is gone.
        // Resources (`Buffer`, `GpuAllocation`) still alive at this point —
        // a teardown order where GPU state outlives the `RenderDevice` —
        // would call into a destroyed device when their retirement actions
        // later run; leak the device instead. A teardown-time leak is
        // recoverable; use-after-destroy is not.
        if let Some(allocator) = self.allocator.take()
            && let Ok(allocator) = Arc::try_unwrap(allocator)
        {
            drop(allocator);
        } else if self.allocator.is_some() {
            moonfield_log::error!(
                "device dropped while GPU resources are still alive; \
                 leaking the device and its allocator"
            );
            // Deliberately keep the live-device count registered so
            // `Instance::drop` leaks rather than destroying around the
            // still-referenced device.
            std::mem::forget(self.allocator.take());
            return;
        }
        self.live_devices
            .fetch_sub(1, std::sync::atomic::Ordering::Release);
        unsafe {
            self.device.destroy_device(None);
        }
    }
}

/// Where the pipeline cache lives on disk: `<XDG_CACHE_HOME or
/// ~/.cache>/moonfield/pipeline_cache.bin`.
fn pipeline_cache_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })
        .unwrap_or_default();
    dir.join("moonfield").join("pipeline_cache.bin")
}
