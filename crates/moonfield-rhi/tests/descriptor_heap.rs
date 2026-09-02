//! Headless integration tests for the bindless 2.0 [`DescriptorHeap`].
//!
//! Covers the slot allocator (alloc/reuse/exhaustion) and the descriptor
//! write paths against a real device's `VK_EXT_descriptor_heap` CPU-visible
//! heap: view create infos are encoded straight into the heap mapping and
//! sampler create infos likewise. The `cmd_bind_graphics` binding step is
//! exercised by the texture-integration tests once textures carry slots
//! (phase 2.3).

mod common;

use ash::vk;
use gpu_allocator::MemoryLocation;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme};
use moonfield_rhi::{
    DescriptorHeap, Device, Instance, SamplerDesc, TextureHandle, TextureSlotDesc,
};
use std::sync::{Arc, Mutex};

/// Create a headless instance + device, skipping on machines without one
/// (mirrors `bump_allocator.rs`).
fn setup() -> Option<(Instance, Device)> {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return None;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return None;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return None;
        }
    };
    Some((instance, device))
}

/// A minimal owned image + view, created so the test can hand a real image
/// view's *create info* to [`TextureSlotDesc`] (the heap encodes the create
/// info into heap memory, and the driver resolves it against a real image).
///
/// Field order matters for drop safety: view, then image, then allocation.
struct TestImage {
    view_create_info: vk::ImageViewCreateInfo<'static>,
    #[allow(dead_code)]
    // the create info is what the heap encodes; the handle exists to keep the view alive
    view: vk::ImageView,
    image: vk::Image,
    allocation: Option<Allocation>,
    device: ash::Device,
    allocator: Arc<Mutex<gpu_allocator::vulkan::Allocator>>,
}

impl TestImage {
    fn new(device: &Device) -> Self {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width: 4,
                height: 4,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: the create info describes a legal image.
        let image =
            unsafe { device.raw().create_image(&image_info, None) }.expect("create test image");
        // SAFETY: requirements for the just-created image.
        let requirements = unsafe { device.raw().get_image_memory_requirements(image) };
        let allocator = device.allocator().clone();
        let allocation = allocator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name: "descriptor heap test image",
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .expect("allocate test image memory");
        // SAFETY: the allocation satisfies the image's memory requirements.
        unsafe {
            device
                .raw()
                .bind_image_memory(image, allocation.memory(), allocation.offset())
        }
        .expect("bind test image memory");

        let view_create_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        // SAFETY: the image outlives the view.
        let view = unsafe { device.raw().create_image_view(&view_create_info, None) }
            .expect("create test image view");

        Self {
            view_create_info,
            view,
            image,
            allocation: Some(allocation),
            device: device.raw().clone(),
            allocator,
        }
    }
}

impl Drop for TestImage {
    fn drop(&mut self) {
        // SAFETY: no frames are in flight in these tests.
        unsafe {
            self.device.destroy_image_view(self.view, None);
            self.device.destroy_image(self.image, None);
        }
        if let Some(allocation) = self.allocation.take() {
            self.allocator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .free(allocation)
                .expect("free test image allocation");
        }
    }
}

#[test]
fn slots_alloc_reuse_and_exhaust() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 4, 4).expect("descriptor heap");

    // Bump allocation hands out ascending slots.
    let a = heap.alloc_image_slot().expect("slot 0");
    assert_eq!(a, TextureHandle(0));
    let b = heap.alloc_image_slot().expect("slot 1");
    assert_eq!(b, TextureHandle(1));
    assert_eq!(heap.alloc_image_slot().expect("slot 2"), TextureHandle(2));

    // Freeing makes the slot the next allocation returns.
    heap.free_image_slot(b).expect("free slot 1");
    assert_eq!(
        heap.alloc_image_slot().expect("reused slot"),
        TextureHandle(1)
    );

    // Exhaustion errors instead of overflowing.
    heap.alloc_image_slot().expect("slot 3");
    assert!(
        heap.alloc_image_slot().is_err(),
        "heap is full (capacity 4)"
    );

    // Double-free and out-of-range frees are rejected.
    heap.free_image_slot(TextureHandle(1))
        .expect("free slot 1 again");
    assert!(
        heap.free_image_slot(TextureHandle(1)).is_err(),
        "double free"
    );
    assert!(
        heap.free_image_slot(TextureHandle(99)).is_err(),
        "never-allocated slot"
    );

    // Sampler slots have the same contract.
    let s = heap.alloc_sampler_slot().expect("sampler slot 0");
    assert_eq!(s.0, 0);
    heap.free_sampler_slot(s).expect("free sampler slot");
}

#[test]
fn write_resource_descriptors_accepts_real_views() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 16, 8).expect("descriptor heap");
    let image = TestImage::new(&device);

    let handle = heap.alloc_image_slot().expect("slot");
    heap.write_resource_descriptors(&[(
        handle,
        TextureSlotDesc {
            view_create_info: &image.view_create_info,
            layout: vk::ImageLayout::GENERAL,
        },
    )])
    .expect("write image descriptor");

    // Overwrite in place — the bump-arena contract: reallocate, then write
    // again before referencing.
    let handle2 = heap.alloc_image_slot().expect("slot 2");
    heap.free_image_slot(handle).expect("free slot");
    heap.write_resource_descriptors(&[(
        handle,
        TextureSlotDesc {
            view_create_info: &image.view_create_info,
            layout: vk::ImageLayout::GENERAL,
        },
    )])
    .expect("rewrite image descriptor");
    assert_ne!(handle, handle2);
}

#[test]
fn write_samplers_encodes_slots() {
    let Some((_instance, device)) = setup() else {
        return;
    };
    let heap = DescriptorHeap::new(&device, 16, 8).expect("descriptor heap");
    let desc = SamplerDesc::default();

    let s = heap.alloc_sampler_slot().expect("sampler slot");
    heap.write_samplers(&[(s, desc)]).expect("write sampler");
    // Same slot, same descriptor: re-encoding in place must not error.
    heap.write_samplers(&[(s, desc)]).expect("rewrite sampler");
    let s2 = heap.alloc_sampler_slot().expect("sampler slot 2");
    assert_ne!(s, s2, "slots are distinct until freed");
}
