//! egui → Vulkan renderer built on Lunar Mare (`moonfield-render`).
//!
//! The feature spec is egui-wgpu 0.33 (reference source cloned at
//! `target/egui-src/crates/egui-wgpu/`), ported to Vulkan idioms: a combined
//! image sampler replaces the separate texture/sampler binding pair, and
//! texture uploads go through a blocking staging copy instead of
//! `queue.write_texture`.
//!
//! Explicitly not supported (recorded in the Agent Note): MSAA, depth-stencil
//! attachments, `CallbackTrait` paint callbacks, multiple viewports. The
//! renderer's shape leaves room for callbacks — `render` records into the
//! caller's open render pass and [`CallbackResources`] is the reserved
//! shared-state bag.

use ash::vk;
use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, TextureId};
use egui::{TextureFilter, TextureOptions, TextureWrapMode};
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator};
use gpu_allocator::MemoryLocation;
use moonfield_render::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    BlendMode, Buffer, BufferUsage, CommandBuffer, CommandPool, Compiler, CullMode, Device,
    GraphicsPipeline, PipelineOptions, RenderPass, Sampler, ShaderModule, ShaderStage, TextureView,
    VertexAttribute, VertexBufferLayout, VertexFormat,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Initial vertex buffer capacity, in vertices (egui-wgpu parity).
const INITIAL_VERTEX_CAPACITY: usize = 1024;
/// Initial index buffer capacity, in indices (egui-wgpu parity).
const INITIAL_INDEX_CAPACITY: usize = 3072;

/// Configuration for [`EguiRenderer`], mirroring egui-wgpu's
/// `RendererOptions`. MSAA and depth-stencil options are intentionally
/// absent.
pub struct RendererOptions {
    /// Dither the output with interleaved gradient noise to reduce banding
    /// (egui-wgpu default: on).
    pub dithering: bool,
    /// Software bilinear filtering in the shader for deterministic snapshot
    /// output (egui-wgpu default: off).
    pub predictable_texture_filtering: bool,
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            dithering: true,
            predictable_texture_filtering: false,
        }
    }
}

/// Reserved shared-state bag for future `CallbackTrait` paint callbacks
/// (egui-wgpu parity: `Renderer::callback_resources`). Empty until callbacks
/// land; then it becomes a type map shared between the callbacks' prepare and
/// paint phases.
pub struct CallbackResources {
    _private: (),
}

/// Per-frame uniform block: screen size in points + shader option flags.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    screen_size_in_points: [f32; 2],
    dithering: u32,
    predictable_filtering: u32,
}

/// `egui::epaint::Vertex` laid out for upload: pos (f32×2), uv (f32×2),
/// packed sRGB color (u32, little-endian RGBA).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PodVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: u32,
}

/// One mesh's draw parameters within a frame slot's shared buffers.
#[derive(Clone, Copy)]
struct MeshDraw {
    index_offset: u32,
    index_count: u32,
    vertex_offset: i32,
}

/// Per-frame-in-flight GPU resources: vertex/index/uniform buffers plus the
/// uniform descriptor set. Buffers grow by doubling and are never shrunk.
struct FrameResources {
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    uniform_buffer: Buffer,
    uniform_set: BindGroup,
    vertex_capacity: usize,
    index_capacity: usize,
    mesh_draws: Vec<MeshDraw>,
}

impl FrameResources {
    fn new(device: &Device, uniform_layout: &BindGroupLayout) -> Result<Self, String> {
        let vertex_buffer = Buffer::new(
            device,
            (INITIAL_VERTEX_CAPACITY * std::mem::size_of::<PodVertex>()) as u64,
            BufferUsage::VERTEX,
            MemoryLocation::CpuToGpu,
        )
        .map_err(|e| e.to_string())?;
        let index_buffer = Buffer::new(
            device,
            (INITIAL_INDEX_CAPACITY * std::mem::size_of::<u32>()) as u64,
            BufferUsage::INDEX,
            MemoryLocation::CpuToGpu,
        )
        .map_err(|e| e.to_string())?;
        let uniform_buffer = Buffer::new(
            device,
            std::mem::size_of::<Uniform>() as u64,
            BufferUsage::UNIFORM,
            MemoryLocation::CpuToGpu,
        )
        .map_err(|e| e.to_string())?;
        let uniform_set = BindGroup::new(
            device,
            uniform_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: std::mem::size_of::<Uniform>() as u64,
                },
            }],
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            uniform_set,
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            index_capacity: INITIAL_INDEX_CAPACITY,
            mesh_draws: Vec::new(),
        })
    }
}

/// An egui-managed RGBA texture: one image per `TextureId::Managed`, no
/// mipmaps, no atlas packing (egui-wgpu parity).
struct ManagedTexture {
    set: BindGroup,
    image_view: vk::ImageView,
    image: vk::Image,
    allocation: Option<Allocation>,
    options: TextureOptions,
    device: ash::Device,
    allocator: Arc<Mutex<Allocator>>,
}

impl Drop for ManagedTexture {
    fn drop(&mut self) {
        // SAFETY: the editor defers frees past the in-flight fence, and
        // `EditorState::drop` waits for device idle before any field drops.
        unsafe {
            self.device.destroy_image_view(self.image_view, None);
            self.device.destroy_image(self.image, None);
        }
        if let Some(allocation) = self.allocation.take() {
            if let Err(e) = self
                .allocator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .free(allocation)
            {
                moonfield_log::error!("failed to free egui texture allocation: {e}");
            }
        }
    }
}

enum TextureEntry {
    Managed(Box<ManagedTexture>),
    /// An external image (e.g. the viewport's offscreen target). Only the
    /// descriptor set is owned; the image belongs to the caller.
    User(Box<BindGroup>),
}

impl TextureEntry {
    fn set(&self) -> vk::DescriptorSet {
        match self {
            Self::Managed(texture) => texture.set.raw_vk(),
            Self::User(set) => set.raw_vk(),
        }
    }
}

/// The egui renderer. Created against a specific render pass (the swapchain
/// pass in the editor, an offscreen pass in tests); `render` records into
/// whichever instance of that pass the caller has open.
///
/// Fields are ordered for Vulkan-safe destruction: textures and frame
/// resources (which reference the layouts) first, then the pipeline and
/// layouts, then the device/allocator clones.
pub struct EguiRenderer {
    textures: HashMap<TextureId, TextureEntry>,
    frames: Vec<FrameResources>,
    pipeline: GraphicsPipeline,
    /// Held to keep the set layouts alive for the lifetime of the bind
    /// groups allocated from them; read only at construction.
    #[allow(dead_code)]
    uniform_layout: BindGroupLayout,
    texture_layout: BindGroupLayout,
    upload_pool: CommandPool,
    samplers: HashMap<TextureOptions, vk::Sampler>,
    next_user_texture_id: u64,
    options: RendererOptions,
    /// Reserved for future paint callbacks; see [`CallbackResources`].
    pub callback_resources: CallbackResources,
    device: ash::Device,
    allocator: Arc<Mutex<Allocator>>,
    queue: vk::Queue,
}

impl EguiRenderer {
    /// Create the renderer. `srgb_framebuffer` selects the fragment entry
    /// point: an sRGB target needs the shader to convert gamma values to
    /// linear and let the hardware re-encode on write; an unorm target takes
    /// the shader's gamma values verbatim. `frames_in_flight` sizes the
    /// per-slot buffer ring and must match the frame loop (see
    /// [`moonfield_render::WindowRenderer::frames_in_flight`]).
    pub fn new(
        device: &Device,
        render_pass: &RenderPass,
        srgb_framebuffer: bool,
        frames_in_flight: usize,
        options: RendererOptions,
    ) -> Result<Self, String> {
        let compiler = Compiler::new().map_err(|e| e.to_string())?;
        let vertex_spirv = compiler
            .compile_source_to_spirv("egui_vk", SHADER_SOURCE, "vs_main")
            .map_err(|e| e.to_string())?;
        let fragment_entry = if srgb_framebuffer {
            "fs_linear"
        } else {
            "fs_gamma"
        };
        let fragment_spirv = compiler
            .compile_source_to_spirv("egui_vk", SHADER_SOURCE, fragment_entry)
            .map_err(|e| e.to_string())?;
        let vertex_shader =
            ShaderModule::from_spirv(device, &vertex_spirv).map_err(|e| e.to_string())?;
        let fragment_shader =
            ShaderModule::from_spirv(device, &fragment_spirv).map_err(|e| e.to_string())?;

        let uniform_layout = BindGroupLayout::new(
            device,
            &[BindGroupLayoutEntry {
                binding: 0,
                ty: BindingType::UniformBuffer,
                visibility: ShaderStage::All,
            }],
        )
        .map_err(|e| e.to_string())?;
        let texture_layout = BindGroupLayout::new(
            device,
            &[BindGroupLayoutEntry {
                binding: 0,
                ty: BindingType::SampledTexture,
                visibility: ShaderStage::Fragment,
            }],
        )
        .map_err(|e| e.to_string())?;

        let vertex_layout = VertexBufferLayout {
            stride: std::mem::size_of::<PodVertex>() as u32,
            attributes: vec![
                VertexAttribute {
                    location: 0,
                    format: VertexFormat::Float32x2,
                    offset: 0,
                },
                VertexAttribute {
                    location: 1,
                    format: VertexFormat::Float32x2,
                    offset: 8,
                },
                VertexAttribute {
                    location: 2,
                    format: VertexFormat::Uint32,
                    offset: 16,
                },
            ],
        };
        let pipeline = GraphicsPipeline::new_with_options(
            device,
            render_pass,
            &vertex_shader,
            &fragment_shader,
            &vertex_layout,
            &[],
            &PipelineOptions {
                blend: BlendMode::PremultipliedAlpha,
                cull_mode: CullMode::None,
                set_layouts: &[&uniform_layout, &texture_layout],
            },
        )
        .map_err(|e| e.to_string())?;

        let upload_pool = CommandPool::new(device, device.queue_family_indices().graphics)
            .map_err(|e| e.to_string())?;

        let mut frames = Vec::with_capacity(frames_in_flight);
        for _ in 0..frames_in_flight {
            frames.push(FrameResources::new(device, &uniform_layout)?);
        }

        Ok(Self {
            textures: HashMap::new(),
            frames,
            pipeline,
            uniform_layout,
            texture_layout,
            upload_pool,
            samplers: HashMap::new(),
            next_user_texture_id: 0,
            options,
            callback_resources: CallbackResources { _private: () },
            device: device.raw().clone(),
            allocator: device.allocator().clone(),
            queue: device.graphics_queue(),
        })
    }

    /// Create or update an egui-managed texture from an [`ImageDelta`]. A
    /// delta with `pos: None` (re)allocates the texture; a delta with a
    /// `pos` updates a sub-region of the existing one. Uploads are blocking
    /// one-shot submissions on the graphics queue.
    pub fn update_texture(
        &mut self,
        device: &Device,
        id: TextureId,
        delta: &ImageDelta,
    ) -> Result<(), String> {
        let egui::epaint::ImageData::Color(image) = &delta.image;
        let width = image.width() as u32;
        let height = image.height() as u32;
        if width == 0 || height == 0 {
            return Err("egui texture with zero extent".to_string());
        }
        let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
        for pixel in &image.pixels {
            bytes.extend_from_slice(&pixel.to_array());
        }

        if let Some(pos) = delta.pos {
            // Partial update of an existing managed texture.
            let (image, image_view, options_changed) = match self.textures.get(&id) {
                Some(TextureEntry::Managed(texture)) => (
                    texture.image,
                    texture.image_view,
                    texture.options != delta.options,
                ),
                Some(TextureEntry::User(_)) => {
                    return Err(format!("partial update of user texture {id:?}"));
                }
                None => {
                    return Err(format!("partial update of unknown texture {id:?}"));
                }
            };
            upload_pixels(
                device,
                &self.upload_pool,
                self.queue,
                image,
                Some([pos[0] as i32, pos[1] as i32]),
                vk::Extent2D { width, height },
                &bytes,
            )?;
            // Sampler options changed: rebuild the descriptor set against the
            // cached-or-new sampler (egui-wgpu parity).
            if options_changed {
                let sampler = self.sampler_for(delta.options)?;
                let set = texture_descriptor_set(
                    device,
                    &self.texture_layout,
                    image_view,
                    sampler,
                    self.device.clone(),
                )?;
                if let Some(TextureEntry::Managed(texture)) = self.textures.get_mut(&id) {
                    texture.set = set;
                    texture.options = delta.options;
                }
            }
            return Ok(());
        }

        let texture = self.create_managed_texture(device, width, height, delta.options)?;
        upload_pixels(
            device,
            &self.upload_pool,
            self.queue,
            texture.image,
            None,
            vk::Extent2D { width, height },
            &bytes,
        )?;
        self.textures
            .insert(id, TextureEntry::Managed(Box::new(texture)));
        Ok(())
    }

    /// Free a texture. Managed textures are destroyed; user textures only
    /// lose their descriptor set (the image belongs to the caller). The
    /// caller defers this call until the frames that sampled the texture
    /// have completed (the editor's free ring).
    pub fn free_texture(&mut self, id: &TextureId) {
        self.textures.remove(id);
    }

    /// Free a batch of textures; see [`free_texture`](Self::free_texture).
    pub fn free_textures(&mut self, ids: &[TextureId]) {
        for id in ids {
            self.free_texture(id);
        }
    }

    /// The descriptor set bound when a mesh references `id`, for inspection
    /// and future paint callbacks (egui-wgpu parity: `Renderer::texture`).
    pub fn texture(&self, id: &TextureId) -> Option<vk::DescriptorSet> {
        self.textures.get(id).map(TextureEntry::set)
    }

    /// Register an external image as a `TextureId::User`, sampling it with
    /// the given sampler (egui-wgpu parity: `register_native_texture`). The
    /// caller keeps the image alive until [`free_texture`](Self::free_texture).
    pub fn register_native_texture(
        &mut self,
        device: &Device,
        view: &TextureView,
        sampler: &Sampler,
    ) -> Result<TextureId, String> {
        let set = BindGroup::new(
            device,
            &self.texture_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Texture { view, sampler },
            }],
        )
        .map_err(|e| e.to_string())?;
        let id = TextureId::User(self.next_user_texture_id);
        self.next_user_texture_id += 1;
        self.textures.insert(id, TextureEntry::User(Box::new(set)));
        Ok(id)
    }

    /// Register an external image with a sampler created from egui sampler
    /// options (egui-wgpu parity: `register_native_texture_with_sampler_options`).
    pub fn register_native_texture_with_options(
        &mut self,
        device: &Device,
        view: &TextureView,
        options: TextureOptions,
    ) -> Result<TextureId, String> {
        let sampler = self.sampler_for(options)?;
        let sampler = Sampler::borrow_raw(sampler, self.device.clone());
        self.register_native_texture(device, view, &sampler)
    }

    /// Rebind an existing user texture id to a new image (egui-wgpu parity:
    /// `update_egui_texture_from_wgpu_texture`) — the path a resizable
    /// offscreen target uses to keep its `TextureId` stable.
    pub fn update_native_texture(
        &mut self,
        device: &Device,
        id: TextureId,
        view: &TextureView,
        sampler: &Sampler,
    ) -> Result<(), String> {
        let Some(entry) = self.textures.get_mut(&id) else {
            return Err(format!("unknown user texture {id:?}"));
        };
        if !matches!(entry, TextureEntry::User(_)) {
            return Err(format!("cannot rebind managed texture {id:?}"));
        }
        *entry = TextureEntry::User(Box::new(
            BindGroup::new(
                device,
                &self.texture_layout,
                &[BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Texture { view, sampler },
                }],
            )
            .map_err(|e| e.to_string())?,
        ));
        Ok(())
    }

    /// Rebind a user texture id with a sampler created from egui sampler
    /// options (egui-wgpu parity:
    /// `update_egui_texture_from_wgpu_texture_with_sampler_options`).
    pub fn update_native_texture_with_options(
        &mut self,
        device: &Device,
        id: TextureId,
        view: &TextureView,
        options: TextureOptions,
    ) -> Result<(), String> {
        let sampler = self.sampler_for(options)?;
        let sampler = Sampler::borrow_raw(sampler, self.device.clone());
        self.update_native_texture(device, id, view, &sampler)
    }

    /// Upload the uniform block and tessellated mesh data into the given
    /// frame slot's buffers, growing them by doubling when full. Must be
    /// called after the slot's fence has passed (i.e. inside the frame, after
    /// `begin_frame`) and before [`render`](Self::render) with the same
    /// `primitives`.
    pub fn update_buffers(
        &mut self,
        device: &Device,
        frame_slot: usize,
        primitives: &[ClippedPrimitive],
        screen_size_points: [f32; 2],
    ) -> Result<(), String> {
        let options = &self.options;
        let frame = self
            .frames
            .get_mut(frame_slot)
            .ok_or_else(|| format!("frame slot {frame_slot} out of range"))?;

        let uniform = Uniform {
            screen_size_in_points: screen_size_points,
            dithering: options.dithering as u32,
            predictable_filtering: options.predictable_texture_filtering as u32,
        };
        frame
            .uniform_buffer
            .upload(device, &[uniform])
            .map_err(|e| e.to_string())?;

        let mut vertices: Vec<PodVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut mesh_draws = Vec::new();
        for clipped in primitives {
            // Callback primitives carry user paint closures; not supported in
            // this slice, so they are skipped.
            let Primitive::Mesh(mesh) = &clipped.primitive else {
                continue;
            };
            mesh_draws.push(MeshDraw {
                index_offset: indices.len() as u32,
                index_count: mesh.indices.len() as u32,
                vertex_offset: vertices.len() as i32,
            });
            vertices.extend(mesh.vertices.iter().map(|v| PodVertex {
                pos: [v.pos.x, v.pos.y],
                uv: [v.uv.x, v.uv.y],
                color: u32::from_le_bytes(v.color.to_array()),
            }));
            indices.extend_from_slice(&mesh.indices);
        }

        if vertices.len() > frame.vertex_capacity {
            frame.vertex_capacity = vertices.len().next_power_of_two();
            frame.vertex_buffer = Buffer::new(
                device,
                (frame.vertex_capacity * std::mem::size_of::<PodVertex>()) as u64,
                BufferUsage::VERTEX,
                MemoryLocation::CpuToGpu,
            )
            .map_err(|e| e.to_string())?;
        }
        if indices.len() > frame.index_capacity {
            frame.index_capacity = indices.len().next_power_of_two();
            frame.index_buffer = Buffer::new(
                device,
                (frame.index_capacity * std::mem::size_of::<u32>()) as u64,
                BufferUsage::INDEX,
                MemoryLocation::CpuToGpu,
            )
            .map_err(|e| e.to_string())?;
        }
        if !vertices.is_empty() {
            frame
                .vertex_buffer
                .upload(device, &vertices)
                .map_err(|e| e.to_string())?;
        }
        if !indices.is_empty() {
            frame
                .index_buffer
                .upload(device, &indices)
                .map_err(|e| e.to_string())?;
        }
        frame.mesh_draws = mesh_draws;
        Ok(())
    }

    /// Record the draw commands for the primitives uploaded by
    /// [`update_buffers`](Self::update_buffers) into the caller's open render
    /// pass. `extent` is the render target's pixel extent; clip rects are in
    /// points and scaled by `pixels_per_point`.
    pub fn render(
        &self,
        command_buffer: &CommandBuffer,
        frame_slot: usize,
        extent: vk::Extent2D,
        pixels_per_point: f32,
        primitives: &[ClippedPrimitive],
    ) {
        let Some(frame) = self.frames.get(frame_slot) else {
            return;
        };
        command_buffer.bind_graphics_pipeline(self.pipeline.raw());
        command_buffer.bind_graphics_descriptor_sets(
            self.pipeline.layout(),
            0,
            &[frame.uniform_set.raw_vk()],
        );
        command_buffer.bind_vertex_buffers(0, &[frame.vertex_buffer.raw()], &[0]);
        command_buffer.bind_index_buffer(frame.index_buffer.raw(), 0, vk::IndexType::UINT32);

        let mut draws = frame.mesh_draws.iter();
        for clipped in primitives {
            let Primitive::Mesh(mesh) = &clipped.primitive else {
                continue;
            };
            let Some(draw) = draws.next() else {
                break;
            };
            let Some(scissor) = clip_rect_to_scissor(clipped.clip_rect, pixels_per_point, extent)
            else {
                continue; // zero-area clip: nothing visible
            };
            let Some(entry) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            command_buffer.set_scissor(scissor);
            command_buffer.bind_graphics_descriptor_sets(self.pipeline.layout(), 1, &[entry.set()]);
            command_buffer.draw_indexed(
                draw.index_count,
                1,
                draw.index_offset,
                draw.vertex_offset,
                0,
            );
        }
    }

    /// Create or fetch the cached sampler for the given egui options (egui
    /// mip levels are always 1, so the mipmap mode only selects the enum).
    fn sampler_for(&mut self, options: TextureOptions) -> Result<vk::Sampler, String> {
        if let Some(sampler) = self.samplers.get(&options) {
            return Ok(*sampler);
        }
        let filter = |f: TextureFilter| match f {
            TextureFilter::Nearest => vk::Filter::NEAREST,
            TextureFilter::Linear => vk::Filter::LINEAR,
        };
        let wrap = match options.wrap_mode {
            TextureWrapMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
            TextureWrapMode::Repeat => vk::SamplerAddressMode::REPEAT,
            TextureWrapMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        };
        let mipmap_mode = match options.mipmap_mode {
            Some(TextureFilter::Linear) => vk::SamplerMipmapMode::LINEAR,
            _ => vk::SamplerMipmapMode::NEAREST,
        };
        let info = vk::SamplerCreateInfo::default()
            .mag_filter(filter(options.magnification))
            .min_filter(filter(options.minification))
            .mipmap_mode(mipmap_mode)
            .address_mode_u(wrap)
            .address_mode_v(wrap)
            .address_mode_w(wrap)
            .max_lod(0.0);
        // SAFETY: the device is valid.
        let sampler = unsafe { self.device.create_sampler(&info, None) }
            .map_err(|e| format!("failed to create egui sampler: {e:?}"))?;
        self.samplers.insert(options, sampler);
        Ok(sampler)
    }

    fn create_managed_texture(
        &mut self,
        device: &Device,
        width: u32,
        height: u32,
        options: TextureOptions,
    ) -> Result<ManagedTexture, String> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: the device is valid and the create info describes a legal image.
        let image = unsafe { self.device.create_image(&image_info, None) }
            .map_err(|e| format!("failed to create egui texture: {e:?}"))?;
        // SAFETY: the image was just created and has no bound memory yet.
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let allocation = self
            .allocator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name: "egui-texture",
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| format!("failed to allocate egui texture memory: {e}"))?;
        // SAFETY: the allocation satisfies the image's memory requirements.
        unsafe {
            self.device
                .bind_image_memory(image, allocation.memory(), allocation.offset())
        }
        .map_err(|e| format!("failed to bind egui texture memory: {e:?}"))?;

        let view_info = vk::ImageViewCreateInfo::default()
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
        // SAFETY: the image is valid and outlives the view.
        let image_view = unsafe { self.device.create_image_view(&view_info, None) }
            .map_err(|e| format!("failed to create egui texture view: {e:?}"))?;

        let sampler = self.sampler_for(options)?;
        let set = texture_descriptor_set(
            device,
            &self.texture_layout,
            image_view,
            sampler,
            self.device.clone(),
        )?;

        Ok(ManagedTexture {
            set,
            image_view,
            image,
            allocation: Some(allocation),
            options,
            device: self.device.clone(),
            allocator: self.allocator.clone(),
        })
    }
}

impl Drop for EguiRenderer {
    fn drop(&mut self) {
        // SAFETY: the caller idled the device before dropping; descriptor
        // sets referencing these samplers are freed by their BindGroups.
        for (_, sampler) in self.samplers.drain() {
            unsafe {
                self.device.destroy_sampler(sampler, None);
            }
        }
    }
}

/// Build the per-texture descriptor set against the renderer's texture layout.
fn texture_descriptor_set(
    device: &Device,
    layout: &BindGroupLayout,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    raw_device: ash::Device,
) -> Result<BindGroup, String> {
    let view = TextureView::borrow_raw(image_view, raw_device.clone());
    let sampler = Sampler::borrow_raw(sampler, raw_device);
    BindGroup::new(
        device,
        layout,
        &[BindGroupEntry {
            binding: 0,
            resource: BindingResource::Texture {
                view: &view,
                sampler: &sampler,
            },
        }],
    )
    .map_err(|e| e.to_string())
}

/// Upload pixels into an image, transitioning layouts around the copy.
/// `offset: None` means a full upload into a fresh (UNDEFINED) image;
/// `Some([x, y])` is a partial update of an image already in
/// SHADER_READ_ONLY_OPTIMAL.
fn upload_pixels(
    device: &Device,
    upload_pool: &CommandPool,
    queue: vk::Queue,
    image: vk::Image,
    offset: Option<[i32; 2]>,
    extent: vk::Extent2D,
    bytes: &[u8],
) -> Result<(), String> {
    let staging = Buffer::new(
        device,
        bytes.len() as u64,
        BufferUsage::COPY_SRC,
        MemoryLocation::CpuToGpu,
    )
    .map_err(|e| e.to_string())?;
    staging.upload(device, bytes).map_err(|e| e.to_string())?;

    let mut command_buffer = upload_pool
        .allocate_command_buffer()
        .map_err(|e| e.to_string())?;
    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .map_err(|e| e.to_string())?;

    let subresource = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);
    let (old_layout, src_access, src_stage) = match offset {
        Some(_) => (
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        None => (
            vk::ImageLayout::UNDEFINED,
            vk::AccessFlags::empty(),
            vk::PipelineStageFlags::TOP_OF_PIPE,
        ),
    };
    let to_transfer = vk::ImageMemoryBarrier::default()
        .src_access_mask(src_access)
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(old_layout)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(subresource);
    command_buffer.pipeline_barrier(
        src_stage,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[to_transfer],
    );

    let (x, y) = offset.map(|[x, y]| (x, y)).unwrap_or((0, 0));
    let region = vk::BufferImageCopy::default()
        .buffer_offset(0)
        .buffer_row_length(0)
        .buffer_image_height(0)
        .image_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(0)
                .base_array_layer(0)
                .layer_count(1),
        )
        .image_offset(vk::Offset3D { x, y, z: 0 })
        .image_extent(vk::Extent3D {
            width: extent.width,
            height: extent.height,
            depth: 1,
        });
    // SAFETY: the staging buffer holds `bytes`, the image is in
    // TRANSFER_DST_OPTIMAL, and the region fits the image.
    unsafe {
        device.raw().cmd_copy_buffer_to_image(
            command_buffer.raw(),
            staging.raw(),
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            std::slice::from_ref(&region),
        );
    }

    let to_shader_read = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(subresource);
    command_buffer.pipeline_barrier(
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[to_shader_read],
    );
    command_buffer.end().map_err(|e| e.to_string())?;

    let command_buffers = [command_buffer.raw()];
    let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
    // SAFETY: the command buffer is fully recorded and the queue is valid.
    unsafe {
        device
            .raw()
            .queue_submit(queue, std::slice::from_ref(&submit_info), vk::Fence::null())
            .map_err(|e| format!("failed to submit egui texture upload: {e:?}"))?;
        device
            .raw()
            .queue_wait_idle(queue)
            .map_err(|e| format!("failed to wait for egui texture upload: {e:?}"))?;
    }
    Ok(())
}

/// Convert an egui clip rect (points) into a Vulkan scissor (physical
/// pixels), clamped to the target extent. `None` when the clip is empty.
fn clip_rect_to_scissor(
    clip_rect: egui::Rect,
    pixels_per_point: f32,
    extent: vk::Extent2D,
) -> Option<vk::Rect2D> {
    let min_x = (clip_rect.min.x * pixels_per_point)
        .round()
        .clamp(0.0, extent.width as f32) as i32;
    let min_y = (clip_rect.min.y * pixels_per_point)
        .round()
        .clamp(0.0, extent.height as f32) as i32;
    let max_x = (clip_rect.max.x * pixels_per_point)
        .round()
        .clamp(min_x as f32, extent.width as f32) as i32;
    let max_y = (clip_rect.max.y * pixels_per_point)
        .round()
        .clamp(min_y as f32, extent.height as f32) as i32;
    let width = max_x - min_x;
    let height = max_y - min_y;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(vk::Rect2D {
        offset: vk::Offset2D { x: min_x, y: min_y },
        extent: vk::Extent2D {
            width: width as u32,
            height: height as u32,
        },
    })
}

/// egui shaders, ported from egui-wgpu's `egui.wgsl` (0.33). One Slang module
/// with one vertex entry and two fragment entries (gamma vs. sRGB target).
const SHADER_SOURCE: &str = r#"
struct Locals
{
    float2 screen_size_in_points;
    uint dithering;
    uint predictable_filtering;
};

[[vk::binding(0, 0)]]
ConstantBuffer<Locals> locals;

[[vk::binding(0, 1)]]
Sampler2D tex;

struct VsInput
{
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
    uint color : COLOR0;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

float4 unpack_color(uint color)
{
    return float4(
        float(color & 255u),
        float((color >> 8u) & 255u),
        float((color >> 16u) & 255u),
        float((color >> 24u) & 255u)) / 255.0;
}

[shader("vertex")]
VsOutput vs_main(VsInput input)
{
    VsOutput output;
    output.uv = input.uv;
    output.color = unpack_color(input.color);
    // Vulkan with a positive-height viewport: +y in points maps to +y in NDC
    // (down the framebuffer), so the y axis needs no flip.
    output.position = float4(
        2.0 * input.pos.x / locals.screen_size_in_points.x - 1.0,
        2.0 * input.pos.y / locals.screen_size_in_points.y - 1.0,
        0.0,
        1.0);
    return output;
}

// Interleaved gradient noise (Jimenez 2014), as in egui-wgpu.
float interleaved_gradient_noise(float2 n)
{
    float f = 0.06711056 * n.x + 0.00583715 * n.y;
    return frac(52.9829189 * frac(f));
}

float3 dither_interleaved(float3 rgb, float levels, float2 frag_coord)
{
    float noise = interleaved_gradient_noise(frag_coord);
    // Scale the noise down slightly so flat colors stay flat.
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

float linear_from_gamma(float srgb)
{
    return srgb < 0.04045 ? srgb / 12.92 : pow((srgb + 0.055) / 1.055, 2.4);
}

float4 sample_texture(float2 uv)
{
    if (locals.predictable_filtering == 0)
    {
        // Hardware filtering: fast, but varies across GPUs and drivers.
        return tex.Sample(uv);
    }
    // Manual bilinear filtering with four taps at pixel centers, for
    // deterministic snapshot output (egui-wgpu parity).
    uint width, height;
    tex.GetDimensions(width, height);
    float2 texture_size = float2(float(width), float(height));
    float2 pixel_coord = uv * texture_size - 0.5;
    float2 pixel_fract = frac(pixel_coord);
    int2 pixel_floor = int2(floor(pixel_coord));
    int2 max_coord = int2(int(width) - 1, int(height) - 1);
    int2 p00 = clamp(pixel_floor + int2(0, 0), int2(0, 0), max_coord);
    int2 p10 = clamp(pixel_floor + int2(1, 0), int2(0, 0), max_coord);
    int2 p01 = clamp(pixel_floor + int2(0, 1), int2(0, 0), max_coord);
    int2 p11 = clamp(pixel_floor + int2(1, 1), int2(0, 0), max_coord);
    float4 tl = tex.Load(int3(p00, 0));
    float4 tr = tex.Load(int3(p10, 0));
    float4 bl = tex.Load(int3(p01, 0));
    float4 br = tex.Load(int3(p11, 0));
    float4 top = lerp(tl, tr, pixel_fract.x);
    float4 bottom = lerp(bl, br, pixel_fract.x);
    return lerp(top, bottom, pixel_fract.y);
}

float4 shade(VsOutput input)
{
    float4 tex_gamma = sample_texture(input.uv);
    float4 out_color_gamma = input.color * tex_gamma;
    if (locals.dithering == 1)
    {
        float3 rgb = dither_interleaved(out_color_gamma.rgb, 256.0, input.position.xy);
        out_color_gamma = float4(rgb, out_color_gamma.a);
    }
    return out_color_gamma;
}

[shader("fragment")]
float4 fs_gamma(VsOutput input) : SV_TARGET
{
    return shade(input);
}

[shader("fragment")]
float4 fs_linear(VsOutput input) : SV_TARGET
{
    float4 color_gamma = shade(input);
    return float4(
        linear_from_gamma(color_gamma.r),
        linear_from_gamma(color_gamma.g),
        linear_from_gamma(color_gamma.b),
        color_gamma.a);
}
"#;
