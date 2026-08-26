//! egui → Vulkan drawing, as plain data resources plus a recording function.
//!
//! There is no "renderer" object. Persistent GPU state lives in three
//! resources the render world owns — [`EguiPipeline`] (shader modules,
//! graphics pipeline, descriptor layouts, cached samplers), [`EguiTextures`]
//! (egui-managed textures, user-texture registrations, the deferred-free
//! ring), and [`EguiFrameResources`] (per-frame-in-flight vertex/index/
//! uniform buffers) — while [`record_egui`] records the draw commands into a
//! render pass the caller has open. The editor's `prepare_egui_frame` /
//! `egui_pass` systems drive them; tests drive them directly.
//!
//! The feature spec is egui-wgpu 0.36 (reference source cloned at
//! `target/egui-src/crates/egui-wgpu/`), ported to Vulkan idioms: a combined
//! image sampler replaces the separate texture/sampler binding pair, and
//! texture uploads go through a blocking staging copy instead of
//! `queue.write_texture`.
//!
//! Explicitly not supported (recorded in the Agent Note): MSAA, depth-stencil
//! attachments, `CallbackTrait` paint callbacks, multiple viewports. The
//! split leaves room for callbacks — `record_egui` records into the caller's
//! open render pass and [`CallbackResources`] is the reserved shared-state
//! bag.

use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, TextureId};
use egui::{TextureFilter, TextureOptions, TextureWrapMode};
use gpu_allocator::MemoryLocation;
use moonfield_render_core::MAX_FRAMES_IN_FLIGHT;
use moonfield_rhi::types::WrapMode;
use moonfield_rhi::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    BlendMode, Buffer, BufferUsage, CommandBuffer, CommandPool, CompareOp, Compiler, CullMode,
    CullState, DepthState, Device, Extent2d, Filter, Format, FrontFace, GraphicsPipeline,
    IndexFormat, Offset2d, PipelineOptions, Rect2d, RenderDevice, Sampler, SamplerDesc,
    ShaderModule, ShaderStage, Texture, TextureView, VertexAttribute, VertexBufferLayout,
    VertexFormat,
};
use std::collections::HashMap;
use std::path::PathBuf;

/// Initial vertex buffer capacity, in vertices (egui-wgpu parity).
const INITIAL_VERTEX_CAPACITY: usize = 1024;
/// Initial index buffer capacity, in indices (egui-wgpu parity).
const INITIAL_INDEX_CAPACITY: usize = 3072;

/// Configuration for the egui pipeline, mirroring egui-wgpu's
/// `RendererOptions`. MSAA and depth-stencil options are intentionally
/// absent.
#[derive(Clone, Copy)]
pub struct EguiOptions {
    /// Dither the output with interleaved gradient noise to reduce banding
    /// (egui-wgpu default: on).
    pub dithering: bool,
    /// Software bilinear filtering in the shader for deterministic snapshot
    /// output (egui-wgpu default: off).
    pub predictable_texture_filtering: bool,
}

impl Default for EguiOptions {
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

/// The egui graphics pipeline and the descriptor set layouts its bind groups
/// are allocated from. `color_format` is the format of the target the
/// pipeline draws into (the swapchain format in the editor); it is baked in
/// via dynamic rendering. `srgb_framebuffer` selects the fragment entry
/// point: an sRGB target needs the shader to convert gamma values to linear
/// and let the hardware re-encode on write; an unorm target takes the
/// shader's gamma values verbatim.
pub struct EguiPipeline {
    pipeline: GraphicsPipeline,
    /// Held to keep the layout alive for the lifetime of the per-slot uniform
    /// bind groups in [`EguiFrameResources`]; read at their construction.
    uniform_layout: BindGroupLayout,
    /// The layout every texture descriptor set in [`EguiTextures`] is
    /// allocated from.
    texture_layout: BindGroupLayout,
    /// Samplers cached by egui sampler options; owned, destroyed on drop.
    samplers: HashMap<TextureOptions, Sampler>,
    options: EguiOptions,
    /// Reserved for future paint callbacks; see [`CallbackResources`].
    pub callback_resources: CallbackResources,
}

impl EguiPipeline {
    /// Compile the egui shaders and build the pipeline for `color_format`.
    pub fn new(
        device: &Device,
        color_format: Format,
        srgb_framebuffer: bool,
        options: EguiOptions,
    ) -> Result<Self, String> {
        let compiler = Compiler::new().map_err(|e| e.to_string())?;
        let vertex_spirv = compiler
            .compile_file_to_spirv(&egui_shader_path(), "vs_main")
            .map_err(|e| e.to_string())?;
        let fragment_entry = if srgb_framebuffer {
            "fs_linear"
        } else {
            "fs_gamma"
        };
        let fragment_spirv = compiler
            .compile_file_to_spirv(&egui_shader_path(), fragment_entry)
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
            &[color_format],
            None,
            &vertex_shader,
            &fragment_shader,
            &vertex_layout,
            &[],
            &PipelineOptions {
                set_layouts: &[&uniform_layout, &texture_layout],
            },
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            pipeline,
            uniform_layout,
            texture_layout,
            samplers: HashMap::new(),
            options,
            callback_resources: CallbackResources { _private: () },
        })
    }

    /// The options the pipeline was built with (the uniform block mirrors
    /// them every frame).
    pub fn options(&self) -> &EguiOptions {
        &self.options
    }

    /// Create or fetch the cached sampler for the given egui options (egui
    /// mip levels are always 1, so the mipmap mode only selects the enum).
    fn sampler(&mut self, device: &Device, options: TextureOptions) -> Result<&Sampler, String> {
        Ok(match self.samplers.entry(options) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry
                .insert(Sampler::new(device, &sampler_desc(options)).map_err(|e| e.to_string())?),
        })
    }

    /// Create-or-fetch the sampler for `options` and bind `view` with it in
    /// the texture layout. The borrow checker cannot mix the mutable sampler
    /// cache access with reads of `texture_layout` at call sites, so both
    /// happen here in one scope.
    fn texture_bind_group(
        &mut self,
        device: &Device,
        view: &TextureView,
        options: TextureOptions,
    ) -> Result<BindGroup, String> {
        self.sampler(device, options)?;
        let sampler = self
            .samplers
            .get(&options)
            .expect("sampler was just ensured");
        BindGroup::new(
            device,
            &self.texture_layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Texture { view, sampler },
            }],
        )
        .map_err(|e| e.to_string())
    }
}

/// Map egui sampler options to the crate's [`SamplerDesc`].
fn sampler_desc(options: TextureOptions) -> SamplerDesc {
    let filter = |f: TextureFilter| match f {
        TextureFilter::Nearest => Filter::Nearest,
        TextureFilter::Linear => Filter::Linear,
    };
    SamplerDesc {
        min_filter: filter(options.minification),
        mag_filter: filter(options.magnification),
        mipmap_filter: options.mipmap_mode.map(filter),
        wrap: match options.wrap_mode {
            TextureWrapMode::ClampToEdge => WrapMode::ClampToEdge,
            TextureWrapMode::Repeat => WrapMode::Repeat,
            TextureWrapMode::MirroredRepeat => WrapMode::MirroredRepeat,
        },
    }
}

/// An egui-managed RGBA texture: one [`Texture`] per `TextureId::Managed`, no
/// mipmaps, no atlas packing (egui-wgpu parity).
struct ManagedTexture {
    texture: Texture,
    set: BindGroup,
    options: TextureOptions,
}

enum TextureEntry {
    Managed(Box<ManagedTexture>),
    /// An external image (e.g. the viewport's offscreen target). Only the
    /// descriptor set is owned; the image belongs to the caller.
    User(Box<BindGroup>),
}

impl TextureEntry {
    fn set(&self) -> &BindGroup {
        match self {
            Self::Managed(texture) => &texture.set,
            Self::User(set) => set,
        }
    }
}

/// The egui texture store: egui-managed textures, registered user textures,
/// the upload command pool, and the deferred-free ring. egui asks for a
/// texture to be freed while in-flight frames may still sample it, so frees
/// are deferred per frame-in-flight slot and applied when the slot comes
/// around again.
pub struct EguiTextures {
    textures: HashMap<TextureId, TextureEntry>,
    next_user_texture_id: u64,
    upload_pool: CommandPool,
    free_ring: [Vec<TextureId>; MAX_FRAMES_IN_FLIGHT],
    /// Held so `Drop` can idle the device before the textures are destroyed:
    /// render-world resources drop in arbitrary order, and the last presented
    /// frame may still be in flight (the `OffscreenTarget` /
    /// `WindowSurfaceData` pattern).
    device: RenderDevice,
}

impl EguiTextures {
    /// Create the store on the shared render device.
    pub fn new(render_device: &RenderDevice) -> Result<Self, String> {
        let device = render_device.device();
        let upload_pool = CommandPool::new(device, device.queue_family_indices().graphics)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            textures: HashMap::new(),
            next_user_texture_id: 0,
            upload_pool,
            free_ring: std::array::from_fn(|_| Vec::new()),
            device: render_device.clone(),
        })
    }

    /// Create or update an egui-managed texture from an [`ImageDelta`]. A
    /// delta with `pos: None` (re)allocates the texture; a delta with a
    /// `pos` updates a sub-region of the existing one. Uploads are blocking
    /// one-shot submissions on the graphics queue.
    pub fn update_texture(
        &mut self,
        device: &Device,
        pipeline: &mut EguiPipeline,
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
            match self.textures.get(&id) {
                Some(TextureEntry::User(_)) => {
                    return Err(format!("partial update of user texture {id:?}"));
                }
                None => {
                    return Err(format!("partial update of unknown texture {id:?}"));
                }
                Some(TextureEntry::Managed(_)) => {}
            }
            let Some(TextureEntry::Managed(entry)) = self.textures.get(&id) else {
                unreachable!("managed texture was just matched");
            };
            entry
                .texture
                .upload(
                    device,
                    &self.upload_pool,
                    &bytes,
                    Some((pos[0] as i32, pos[1] as i32)),
                    (width, height),
                )
                .map_err(|e| e.to_string())?;
            // Sampler options changed: rebuild the descriptor set against the
            // cached-or-new sampler (egui-wgpu parity).
            if entry.options != delta.options {
                let set =
                    pipeline.texture_bind_group(device, &entry.texture.view(), delta.options)?;
                let Some(TextureEntry::Managed(entry)) = self.textures.get_mut(&id) else {
                    unreachable!("managed texture was just matched");
                };
                entry.set = set;
                entry.options = delta.options;
            }
            return Ok(());
        }

        let texture = Texture::new(device, width, height, Format::R8G8B8A8Unorm)
            .map_err(|e| e.to_string())?;
        texture
            .upload(device, &self.upload_pool, &bytes, None, (width, height))
            .map_err(|e| e.to_string())?;
        let set = pipeline.texture_bind_group(device, &texture.view(), delta.options)?;
        self.textures.insert(
            id,
            TextureEntry::Managed(Box::new(ManagedTexture {
                texture,
                set,
                options: delta.options,
            })),
        );
        Ok(())
    }

    /// Free a texture. Managed textures are destroyed; user textures only
    /// lose their descriptor set (the image belongs to the caller). Prefer
    /// [`defer_free`](Self::defer_free) — this is the immediate path the ring
    /// drains into.
    pub fn free_texture(&mut self, id: &TextureId) {
        self.textures.remove(id);
    }

    /// Free a batch of textures; see [`free_texture`](Self::free_texture).
    pub fn free_textures(&mut self, ids: &[TextureId]) {
        for id in ids {
            self.free_texture(id);
        }
    }

    /// Free the textures deferred into `slot` when it last came around: their
    /// frame-in-flight fence has passed, so the GPU no longer samples them.
    pub fn deferred_free_slot(&mut self, slot: usize) {
        let ids = std::mem::take(&mut self.free_ring[slot % MAX_FRAMES_IN_FLIGHT]);
        self.free_textures(&ids);
    }

    /// Defer freeing `ids` until `slot` comes around again
    /// ([`MAX_FRAMES_IN_FLIGHT`] frames later).
    pub fn defer_free(&mut self, slot: usize, ids: Vec<TextureId>) {
        self.free_ring[slot % MAX_FRAMES_IN_FLIGHT].extend(ids);
    }

    /// The descriptor set bound when a mesh references `id`, for inspection
    /// and future paint callbacks (egui-wgpu parity: `Renderer::texture`).
    pub fn texture(&self, id: &TextureId) -> Option<&BindGroup> {
        self.textures.get(id).map(TextureEntry::set)
    }

    /// Register an external image as a `TextureId::User`, sampling it with
    /// the given sampler (egui-wgpu parity: `register_native_texture`). The
    /// caller keeps the image alive until [`free_texture`](Self::free_texture).
    pub fn register_native_texture(
        &mut self,
        device: &Device,
        pipeline: &mut EguiPipeline,
        view: &TextureView,
        sampler: &Sampler,
    ) -> Result<TextureId, String> {
        let set = BindGroup::new(
            device,
            &pipeline.texture_layout,
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
        pipeline: &mut EguiPipeline,
        view: &TextureView,
        options: TextureOptions,
    ) -> Result<TextureId, String> {
        let set = pipeline.texture_bind_group(device, view, options)?;
        let id = TextureId::User(self.next_user_texture_id);
        self.next_user_texture_id += 1;
        self.textures.insert(id, TextureEntry::User(Box::new(set)));
        Ok(id)
    }

    /// Rebind an existing user texture id to a new image (egui-wgpu parity:
    /// `update_egui_texture_from_wgpu_texture`) — the path a resizable
    /// offscreen target uses to keep its `TextureId` stable.
    pub fn update_native_texture(
        &mut self,
        device: &Device,
        pipeline: &mut EguiPipeline,
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
                &pipeline.texture_layout,
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
        pipeline: &mut EguiPipeline,
        id: TextureId,
        view: &TextureView,
        options: TextureOptions,
    ) -> Result<(), String> {
        let Some(entry) = self.textures.get(&id) else {
            return Err(format!("unknown user texture {id:?}"));
        };
        if !matches!(entry, TextureEntry::User(_)) {
            return Err(format!("cannot rebind managed texture {id:?}"));
        }
        let set = pipeline.texture_bind_group(device, view, options)?;
        self.textures.insert(id, TextureEntry::User(Box::new(set)));
        Ok(())
    }
}

impl Drop for EguiTextures {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so no texture or descriptor set is
        // destroyed while the GPU still uses it (render-world resources drop
        // in arbitrary order, and the last presented frame may be in flight).
        unsafe {
            let _ = self.device.device().raw().device_wait_idle();
        }
    }
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

/// The per-slot buffer ring. `frames_in_flight` must match the frame loop
/// (the window surfaces' [`MAX_FRAMES_IN_FLIGHT`]); each slot is only written
/// after its fence has passed (i.e. inside the frame, after acquire).
pub struct EguiFrameResources {
    frames: Vec<FrameResources>,
}

impl EguiFrameResources {
    /// Create one buffer set per frame-in-flight slot; uniform sets bind
    /// against the pipeline's uniform layout.
    pub fn new(
        device: &Device,
        pipeline: &EguiPipeline,
        frames_in_flight: usize,
    ) -> Result<Self, String> {
        let mut frames = Vec::with_capacity(frames_in_flight);
        for _ in 0..frames_in_flight {
            frames.push(FrameResources::new(device, &pipeline.uniform_layout)?);
        }
        Ok(Self { frames })
    }

    /// Upload the uniform block and tessellated mesh data into the given
    /// frame slot's buffers, growing them by doubling when full. Must be
    /// called after the slot's fence has passed and before [`record_egui`]
    /// with the same `primitives`.
    pub fn update(
        &mut self,
        device: &Device,
        frame_slot: usize,
        primitives: &[ClippedPrimitive],
        screen_size_points: [f32; 2],
        options: &EguiOptions,
    ) -> Result<(), String> {
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
}

/// Record the draw commands for the primitives uploaded by
/// [`EguiFrameResources::update`] into the caller's open render pass.
/// `extent` is the render target's pixel extent; clip rects are in points
/// and scaled by `pixels_per_point`.
#[allow(clippy::too_many_arguments)] // one parameter per resource the pass reads
pub fn record_egui(
    command_buffer: &CommandBuffer,
    pipeline: &EguiPipeline,
    textures: &EguiTextures,
    frames: &EguiFrameResources,
    frame_slot: usize,
    extent: (u32, u32),
    pixels_per_point: f32,
    primitives: &[ClippedPrimitive],
) {
    let Some(frame) = frames.frames.get(frame_slot) else {
        return;
    };
    // egui draws with premultiplied-alpha blending, no culling, no depth.
    command_buffer.set_blend_state(BlendMode::PremultipliedAlpha);
    command_buffer.set_cull_state(CullState {
        cull_mode: CullMode::None,
        front_face: FrontFace::Clockwise,
    });
    command_buffer.set_depth_state(DepthState {
        test_enable: false,
        write_enable: false,
        compare_op: CompareOp::GreaterOrEqual,
    });
    command_buffer.bind_graphics_pipeline(&pipeline.pipeline);
    command_buffer.bind_graphics_descriptor_sets(
        pipeline.pipeline.layout(),
        0,
        &[&frame.uniform_set],
    );
    command_buffer.bind_vertex_buffers(0, &[&frame.vertex_buffer], &[0]);
    command_buffer.bind_index_buffer(&frame.index_buffer, 0, IndexFormat::Uint32);

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
        let Some(entry) = textures.textures.get(&mesh.texture_id) else {
            continue;
        };
        command_buffer.set_scissor(scissor);
        command_buffer.bind_graphics_descriptor_sets(pipeline.pipeline.layout(), 1, &[entry.set()]);
        command_buffer.draw_indexed(
            draw.index_count,
            1,
            draw.index_offset,
            draw.vertex_offset,
            0,
        );
    }
}

/// Convert an egui clip rect (points) into a scissor rect (physical pixels),
/// clamped to the target extent. `None` when the clip is empty.
fn clip_rect_to_scissor(
    clip_rect: egui::Rect,
    pixels_per_point: f32,
    extent: (u32, u32),
) -> Option<Rect2d> {
    let (width, height) = (extent.0 as f32, extent.1 as f32);
    let min_x = (clip_rect.min.x * pixels_per_point)
        .round()
        .clamp(0.0, width) as i32;
    let min_y = (clip_rect.min.y * pixels_per_point)
        .round()
        .clamp(0.0, height) as i32;
    let max_x = (clip_rect.max.x * pixels_per_point)
        .round()
        .clamp(min_x as f32, width) as i32;
    let max_y = (clip_rect.max.y * pixels_per_point)
        .round()
        .clamp(min_y as f32, height) as i32;
    let (width, height) = (max_x - min_x, max_y - min_y);
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(Rect2d {
        offset: Offset2d { x: min_x, y: min_y },
        extent: Extent2d {
            width: width as u32,
            height: height as u32,
        },
    })
}

/// The egui shader file (`<repo root>/assets/shaders/egui.slang`), ported
/// from egui-wgpu's `egui.wgsl` (0.36): one Slang module with one vertex
/// entry and two fragment entries (gamma vs. sRGB target).
///
/// `CARGO_MANIFEST_DIR` is a compile-time absolute path, so the file resolves
/// whichever directory the process runs from.
fn egui_shader_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/shaders")
        .join("egui.slang")
        .to_string_lossy()
        .into_owned()
}
