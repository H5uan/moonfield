//! Vulkan descriptor and resource-view abstraction.
//!
//! The Vulkan backend owns a `vk::DescriptorSet` (with a per-set pool) and
//! exposes `raw_vk()` as a controlled escape hatch
//! for interop with libraries that take raw Vulkan handles (e.g.
//! `egui_ash_renderer`).
//!
//! The shape is intentionally minimal — a single combined-image-sampler or
//! buffer binding per layout, grown on demand — enough to back the editor
//! viewport today and the indirect/workgraph paths later without another
//! redesign.

use crate::types::BufferUsage;

use crate::vulkan::device::Device as VulkanDevice;

/// Shader stages that may access a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
    All,
}

impl ShaderStage {
    pub(crate) fn to_vk(self) -> ash::vk::ShaderStageFlags {
        match self {
            Self::Vertex => ash::vk::ShaderStageFlags::VERTEX,
            Self::Fragment => ash::vk::ShaderStageFlags::FRAGMENT,
            Self::Compute => ash::vk::ShaderStageFlags::COMPUTE,
            Self::All => ash::vk::ShaderStageFlags::ALL,
        }
    }
}

/// The kind of resource bound at a descriptor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    /// A combined texture + sampler, readable in shaders.
    SampledTexture,
    /// A read/write storage buffer.
    StorageBuffer,
    /// A read-only uniform buffer.
    UniformBuffer,
}

/// A single entry in a [`BindGroupLayout`].
#[derive(Debug, Clone, Copy)]
pub struct BindGroupLayoutEntry {
    /// The binding index in the shader.
    pub binding: u32,
    /// The kind of resource at this binding.
    pub ty: BindingType,
    /// Which shader stages may access it.
    pub visibility: ShaderStage,
}

/// A resource bound into a [`BindGroup`].
#[derive(Clone, Copy)]
pub enum BindingResource<'a> {
    /// A combined texture + sampler.
    Texture {
        view: &'a TextureView,
        sampler: &'a Sampler,
    },
    /// A buffer slice.
    Buffer {
        buffer: &'a dyn BufferRef,
        offset: u64,
        size: u64,
    },
}

/// Type-erased reference to a buffer, so `BindingResource` does not depend on
/// the concrete Vulkan `Buffer` type.
pub trait BufferRef {
    fn raw_vk(&self) -> ash::vk::Buffer;
}

/// A single binding in a [`BindGroup`].
#[derive(Clone, Copy)]
pub struct BindGroupEntry<'a> {
    /// The binding index in the shader.
    pub binding: u32,
    /// The resource at this binding.
    pub resource: BindingResource<'a>,
}

// ===========================================================================
// Vulkan (ash) implementation
// ===========================================================================

pub(crate) mod vulkan_impl {
    use super::*;
    use ash::vk;

    /// A Vulkan sampler wrapped for the cross-backend API.
    ///
    /// `owns` distinguishes a sampler created and owned by this wrapper
    /// (`from_raw`, Drop destroys it) from one borrowed from another owner
    /// (`borrow_raw`, Drop leaves it alone — the owner destroys it).
    pub struct Sampler {
        sampler: vk::Sampler,
        device: ash::Device,
        owns: bool,
    }

    impl Sampler {
        /// Wrap a sampler this wrapper owns; `Drop` destroys it.
        #[allow(dead_code)]
        pub(crate) fn from_raw(sampler: vk::Sampler, device: ash::Device) -> Self {
            Self {
                sampler,
                device,
                owns: true,
            }
        }
        /// Borrow a sampler owned elsewhere; `Drop` does not destroy it.
        pub(crate) fn borrow_raw(sampler: vk::Sampler, device: ash::Device) -> Self {
            Self {
                sampler,
                device,
                owns: false,
            }
        }
        /// Raw Vulkan handle, for interop with libraries taking raw handles.
        pub fn raw_vk(&self) -> vk::Sampler {
            self.sampler
        }
    }

    impl Drop for Sampler {
        fn drop(&mut self) {
            if self.owns {
                unsafe {
                    self.device.destroy_sampler(self.sampler, None);
                }
            }
        }
    }

    /// A Vulkan image view wrapped for the cross-backend API.
    ///
    /// See [`Sampler`] for the own/borrow distinction.
    pub struct TextureView {
        view: vk::ImageView,
        device: ash::Device,
        owns: bool,
    }

    impl TextureView {
        /// Wrap an image view this wrapper owns; `Drop` destroys it.
        #[allow(dead_code)]
        pub(crate) fn from_raw(view: vk::ImageView, device: ash::Device) -> Self {
            Self {
                view,
                device,
                owns: true,
            }
        }
        /// Borrow an image view owned elsewhere; `Drop` does not destroy it.
        pub(crate) fn borrow_raw(view: vk::ImageView, device: ash::Device) -> Self {
            Self {
                view,
                device,
                owns: false,
            }
        }
        /// Raw Vulkan handle, for interop with libraries taking raw handles.
        pub fn raw_vk(&self) -> vk::ImageView {
            self.view
        }
    }

    impl Drop for TextureView {
        fn drop(&mut self) {
            if self.owns {
                unsafe {
                    self.device.destroy_image_view(self.view, None);
                }
            }
        }
    }

    /// A Vulkan descriptor set layout.
    pub struct BindGroupLayout {
        layout: vk::DescriptorSetLayout,
        device: ash::Device,
    }

    impl BindGroupLayout {
        /// Create a layout from the given entries.
        pub fn new(device: &VulkanDevice, entries: &[BindGroupLayoutEntry]) -> crate::Result<Self> {
            let bindings: Vec<vk::DescriptorSetLayoutBinding> = entries
                .iter()
                .map(|e| {
                    let ty = match e.ty {
                        BindingType::SampledTexture => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        BindingType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
                        BindingType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
                    };
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(e.binding)
                        .descriptor_type(ty)
                        .descriptor_count(1)
                        .stage_flags(e.visibility.to_vk())
                })
                .collect();
            let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
            let layout = unsafe {
                device
                    .raw()
                    .create_descriptor_set_layout(&info, None)
                    .map_err(|e| {
                        crate::Error::Backend(format!(
                            "failed to create descriptor set layout: {:?}",
                            e
                        ))
                    })?
            };
            Ok(Self {
                layout,
                device: device.raw().clone(),
            })
        }

        /// Raw Vulkan handle.
        pub fn raw_vk(&self) -> vk::DescriptorSetLayout {
            self.layout
        }
    }

    impl Drop for BindGroupLayout {
        fn drop(&mut self) {
            unsafe {
                self.device.destroy_descriptor_set_layout(self.layout, None);
            }
        }
    }

    /// A Vulkan descriptor set plus its own pool (one set per pool, the simple
    /// model used by the editor viewport). Sufficient for the current
    /// single-binding use case; a shared pool can replace it when workgraph /
    /// multi-set paths land.
    pub struct BindGroup {
        set: vk::DescriptorSet,
        pool: vk::DescriptorPool,
        device: ash::Device,
    }

    impl BindGroup {
        /// Allocate and write a descriptor set for the given entries.
        pub fn new(
            device: &VulkanDevice,
            layout: &BindGroupLayout,
            entries: &[BindGroupEntry<'_>],
        ) -> crate::Result<Self> {
            let pool_sizes: Vec<vk::DescriptorPoolSize> = {
                let mut map: std::collections::HashMap<vk::DescriptorType, u32> =
                    std::collections::HashMap::new();
                for e in entries {
                    let ty = match resource_type(&e.resource) {
                        BindingType::SampledTexture => vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        BindingType::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
                        BindingType::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
                    };
                    *map.entry(ty).or_insert(0) += 1;
                }
                map.into_iter()
                    .map(|(ty, count)| {
                        vk::DescriptorPoolSize::default()
                            .ty(ty)
                            .descriptor_count(count)
                    })
                    .collect()
            };
            let pool_info = vk::DescriptorPoolCreateInfo::default()
                .pool_sizes(&pool_sizes)
                .max_sets(1)
                .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET);
            let pool = unsafe {
                device
                    .raw()
                    .create_descriptor_pool(&pool_info, None)
                    .map_err(|e| {
                        crate::Error::Backend(format!("failed to create descriptor pool: {:?}", e))
                    })?
            };

            let set_layouts = [layout.raw_vk()];
            let alloc_info = vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&set_layouts);
            let set = unsafe {
                device
                    .raw()
                    .allocate_descriptor_sets(&alloc_info)
                    .map_err(|e| {
                        crate::Error::Backend(format!("failed to allocate descriptor set: {:?}", e))
                    })?[0]
            };

            // Keep the backing image/buffer info alive in this scope while the
            // writes reference them, then submit before they drop. Collect all
            // infos first, then build writes referencing them, to avoid
            // interleaving a mutable push on a vec while a write borrows it.
            let mut image_infos: Vec<vk::DescriptorImageInfo> = Vec::new();
            let mut buffer_infos: Vec<vk::DescriptorBufferInfo> = Vec::new();
            // (binding, descriptor type, index into the matching infos vec)
            let mut pending: Vec<(u32, vk::DescriptorType, usize, bool)> = Vec::new();
            for e in entries {
                match e.resource {
                    BindingResource::Texture { view, sampler } => {
                        let idx = image_infos.len();
                        image_infos.push(vk::DescriptorImageInfo {
                            sampler: sampler.raw_vk(),
                            image_view: view.raw_vk(),
                            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                        });
                        pending.push((
                            e.binding,
                            vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                            idx,
                            true,
                        ));
                    }
                    BindingResource::Buffer {
                        buffer,
                        offset,
                        size,
                    } => {
                        let idx = buffer_infos.len();
                        buffer_infos.push(vk::DescriptorBufferInfo {
                            buffer: buffer.raw_vk(),
                            offset,
                            range: size,
                        });
                        pending.push((e.binding, vk::DescriptorType::STORAGE_BUFFER, idx, false));
                    }
                }
            }
            let writes: Vec<vk::WriteDescriptorSet> = pending
                .iter()
                .map(|&(binding, ty, idx, is_image)| {
                    let mut w = vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(binding)
                        .descriptor_type(ty);
                    if is_image {
                        w = w.image_info(std::slice::from_ref(&image_infos[idx]));
                    } else {
                        w = w.buffer_info(std::slice::from_ref(&buffer_infos[idx]));
                    }
                    w
                })
                .collect();
            unsafe { device.raw().update_descriptor_sets(&writes, &[]) };

            Ok(Self {
                set,
                pool,
                device: device.raw().clone(),
            })
        }

        /// Raw Vulkan handle, for interop (e.g. `egui_ash_renderer`).
        pub fn raw_vk(&self) -> vk::DescriptorSet {
            self.set
        }
    }

    impl Drop for BindGroup {
        fn drop(&mut self) {
            // SAFETY: the set was allocated from this pool.
            unsafe {
                let _ = self
                    .device
                    .free_descriptor_sets(self.pool, std::slice::from_ref(&self.set));
                self.device.destroy_descriptor_pool(self.pool, None);
            }
        }
    }

    fn resource_type(resource: &BindingResource<'_>) -> BindingType {
        match resource {
            BindingResource::Texture { .. } => BindingType::SampledTexture,
            BindingResource::Buffer { .. } => BindingType::StorageBuffer,
        }
    }
}

pub use vulkan_impl::{BindGroup, BindGroupLayout, Sampler, TextureView};

// Keep BufferUsage referenced so the module compiles even when only the
// texture path is exercised; storage/uniform buffer bindings will use it.
const _: fn() = || {
    let _ = BufferUsage::empty();
};
