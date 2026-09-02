//! egui → Vulkan drawing, as plain data resources plus a recording function.
//!
//! There is no "renderer" object. Persistent GPU state lives in three
//! resources the render world owns — [`EguiPipeline`] (shader modules,
//! graphics pipeline, the descriptor heap, cached sampler slots),
//! [`EguiTextures`] (egui-managed textures, user-texture registrations, the
//! deferred-free ring), and [`EguiFrameResources`] (per-frame-in-flight
//! vertex/index buffers) — while [`record_egui`] records the draw commands
//! into a render pass the caller has open. The editor's `prepare_egui_frame`
//! / `egui_pass` systems drive them; tests drive them directly.
//!
//! The feature spec is egui-wgpu 0.36 (reference source cloned at
//! `target/egui-src/crates/egui-wgpu/`), ported to the RHI's bindless model:
//! all shader resources come from the `VK_EXT_descriptor_heap` heaps (texture
//! and sampler slots indexed in the shader), and per-draw root data (screen
//! size, option flags, texture/sampler handles) is pushed with
//! [`CommandBuffer::push_data`]. There are no descriptor set layouts, no
//! descriptor sets, and no push constant ranges anywhere in the pipeline.
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
    BlendMode, Buffer, BufferUsage, CommandBuffer, CompareOp, Compiler, CullMode, CullState,
    DepthState, DescriptorHeap, Device, Extent2d, Filter, Format, FrameUploader, FrontFace,
    GraphicsPipeline, IndexFormat, Offset2d, Rect2d, RenderDevice, RootBinder, SamplerDesc,
    SamplerHandle, ShaderModule, Texture, TextureHandle, UPLOAD_ARENA_SIZE,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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

/// Per-draw root data, pushed via [`CommandBuffer::push_data`] and read by
/// the shader through the PushConstant storage class. Layout must match
/// `EguiRoot` in `egui.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EguiRoot {
    screen_size_in_points: [f32; 2],
    texture: u32,
    sampler: u32,
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

/// The egui graphics pipeline: a descriptor-heap pipeline (null layout, no
/// set layouts, no push constant ranges) plus the shared heap and the sampler
/// slot cache its textures draw from. `color_format` is the format of the
/// target the pipeline draws into (the swapchain format in the editor); it is
/// baked in via dynamic rendering. `srgb_framebuffer` selects the fragment
/// entry point: an sRGB target needs the shader to convert gamma values to
/// linear and let the hardware re-encode on write; an unorm target takes the
/// shader's gamma values verbatim.
pub struct EguiPipeline {
    pipeline: GraphicsPipeline,
    /// Reflection-built root blob template (`uniform EguiRoot` → the inline
    /// struct); draws clone it, write the fields, and push it.
    root: RootBinder,
    /// The shared descriptor heap every egui texture/sampler slot lives in.
    heap: Arc<DescriptorHeap>,
    /// Sampler heap slots cached by egui sampler options; freed on drop.
    samplers: HashMap<TextureOptions, SamplerHandle>,
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
        let fragment_entry = if srgb_framebuffer {
            "fs_linear"
        } else {
            "fs_gamma"
        };
        let vertex_shader = ShaderModule::from_compiled(
            device,
            &compiler
                .compile_file_to_spirv(&egui_shader_path(), "vs_main")
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let fragment_shader = ShaderModule::from_compiled(
            device,
            &compiler
                .compile_file_to_spirv_with_capabilities(
                    &egui_shader_path(),
                    fragment_entry,
                    &["spvDescriptorHeapEXT"],
                )
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

        // Derive the vertex layout from the vertex shader's reflected inputs
        // (pos f32×2, uv f32×2, packed color u32 — the shader is the single
        // source of truth; `PodVertex` must match, enforced by the upload
        // path's layout assertions).
        let reflection = compiler
            .compile_file_to_reflection(&egui_shader_path(), "vs_main")
            .map_err(|e| e.to_string())?;
        let vertex_layout = reflection
            .vertex_layout("vs_main")
            .map_err(|e| e.to_string())?;
        let root = RootBinder::new(&reflection, "vs_main").map_err(|e| e.to_string())?;
        // Layout alignment guard: the Rust `EguiRoot` struct written into the
        // root blob must be exactly as large as the shader's reflected
        // `uniform EguiRoot`, so the two can never silently drift.
        if root.blob().len() != std::mem::size_of::<EguiRoot>() {
            return Err(format!(
                "EguiRoot layout mismatch: Rust struct is {} bytes, egui.slang \
                 root is {} bytes",
                std::mem::size_of::<EguiRoot>(),
                root.blob().len()
            ));
        }
        drop(reflection);
        // Descriptor-heap pipeline: null layout, no set layouts, no push
        // constant ranges, no bindings. The fragment shader reads the texture
        // and sampler straight from the untyped descriptor heaps at the slot
        // indices carried in each draw's push data.
        let pipeline = GraphicsPipeline::new_with_options(
            device,
            &[color_format],
            None,
            &vertex_shader,
            &fragment_shader,
            &vertex_layout,
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            pipeline,
            root,
            heap: device.descriptor_heap(),
            samplers: HashMap::new(),
            options,
            callback_resources: CallbackResources { _private: () },
        })
    }

    /// The options the pipeline was built with (the per-draw root data
    /// mirrors them every draw).
    pub fn options(&self) -> &EguiOptions {
        &self.options
    }

    /// Create-or-fetch the descriptor-heap slot of the sampler for the given
    /// egui options (egui mip levels are always 1, so the mipmap mode only
    /// selects the enum).
    fn sampler_slot(&mut self, options: TextureOptions) -> Result<SamplerHandle, String> {
        Ok(match self.samplers.entry(options) {
            std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let handle = self.heap.alloc_sampler_slot().map_err(|e| e.to_string())?;
                self.heap
                    .write_samplers(&[(handle, sampler_desc(options))])
                    .map_err(|e| e.to_string())?;
                *entry.insert(handle)
            }
        })
    }
}

impl Drop for EguiPipeline {
    fn drop(&mut self) {
        for (_, handle) in self.samplers.drain() {
            if let Err(e) = self.heap.free_sampler_slot(handle) {
                moonfield_log::error!("failed to free egui sampler slot: {e}");
            }
        }
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

/// An egui-managed RGBA texture: one bindless [`Texture`] per
/// `TextureId::Managed`, no mipmaps, no atlas packing (egui-wgpu parity).
struct ManagedTexture {
    texture: Texture,
    texture_handle: TextureHandle,
    sampler: SamplerHandle,
    options: TextureOptions,
}

/// An external image (e.g. the viewport's offscreen target): just the heap
/// handles; the image and its slots belong to the caller.
#[derive(Clone, Copy)]
struct UserTexture {
    texture: TextureHandle,
    sampler: SamplerHandle,
}

enum TextureEntry {
    Managed(Box<ManagedTexture>),
    User(UserTexture),
}

impl TextureEntry {
    /// The handles the per-draw root data carries for this entry.
    fn handles(&self) -> (TextureHandle, SamplerHandle) {
        match self {
            Self::Managed(texture) => (texture.texture_handle, texture.sampler),
            Self::User(user) => (user.texture, user.sampler),
        }
    }
}

/// A deferred free. egui asks for textures to be freed — and full updates
/// replace textures — while in-flight frames may still sample their heap
/// slots, so destruction is deferred per frame-in-flight slot and applied
/// when the slot comes around again.
enum DeferredFree {
    /// egui freed this id; the map entry is removed (and dropped) at drain.
    Id(TextureId),
    /// An entry already replaced in the map by a full update; dropped at
    /// drain (returning its heap slot) once no frame can reference it.
    Replaced(TextureEntry),
}

/// The egui texture store: egui-managed textures, registered user textures,
/// the upload command pool, and the deferred-free ring.
pub struct EguiTextures {
    textures: HashMap<TextureId, TextureEntry>,
    next_user_texture_id: u64,
    /// Frame-scoped uploader staging this frame's texture deltas; flushed
    /// once per frame by the caller (one submit).
    uploader: FrameUploader,
    free_ring: [Vec<DeferredFree>; MAX_FRAMES_IN_FLIGHT],
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
        let uploader = FrameUploader::new(device, UPLOAD_ARENA_SIZE).map_err(|e| e.to_string())?;
        Ok(Self {
            textures: HashMap::new(),
            next_user_texture_id: 0,
            uploader,
            free_ring: std::array::from_fn(|_| Vec::new()),
            device: render_device.clone(),
        })
    }

    /// Submit all texture uploads recorded this frame — one queue submit
    /// per frame instead of one per texture delta. Idempotent: a frame with
    /// no uploads submits nothing.
    pub fn flush_uploads(&mut self) -> Result<(), String> {
        self.uploader.end_frame().map_err(|e| e.to_string())
    }

    /// Create or update an egui-managed texture from an [`ImageDelta`]. A
    /// delta with `pos: None` (re)allocates the texture; a delta with a
    /// `pos` updates a sub-region of the existing one. A replaced texture
    /// goes to `frame_slot`'s deferred-free ring: in-flight frames may still
    /// sample its heap slot.
    pub fn update_texture(
        &mut self,
        device: &Device,
        pipeline: &mut EguiPipeline,
        id: TextureId,
        delta: &ImageDelta,
        frame_slot: usize,
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
            // Partial update of an existing managed texture: the heap slot
            // and its descriptor are untouched, only the pixels move.
            match self.textures.get(&id) {
                Some(TextureEntry::User(_)) => {
                    return Err(format!("partial update of user texture {id:?}"));
                }
                None => {
                    return Err(format!("partial update of unknown texture {id:?}"));
                }
                Some(TextureEntry::Managed(_)) => {}
            }
            let Some(TextureEntry::Managed(entry)) = self.textures.get_mut(&id) else {
                unreachable!("managed texture was just matched");
            };
            entry
                .texture
                .upload(
                    &mut self.uploader,
                    &bytes,
                    Some((pos[0] as i32, pos[1] as i32)),
                    (width, height),
                )
                .map_err(|e| e.to_string())?;
            // Sampler options changed: just swap the sampler slot the draw's
            // root data carries (egui-wgpu parity, minus the rebind).
            if entry.options != delta.options {
                entry.sampler = pipeline.sampler_slot(delta.options)?;
                entry.options = delta.options;
            }
            return Ok(());
        }

        let sampler = pipeline.sampler_slot(delta.options)?;
        let texture = Texture::bindless(
            device,
            &mut self.uploader,
            width,
            height,
            Format::R8G8B8A8Unorm,
            &bytes,
        )
        .map_err(|e| e.to_string())?;
        let texture_handle = texture
            .handle()
            .expect("Texture::bindless always has a slot");
        let entry = TextureEntry::Managed(Box::new(ManagedTexture {
            texture,
            texture_handle,
            sampler,
            options: delta.options,
        }));
        if let Some(replaced) = self.textures.insert(id, entry) {
            self.free_ring[frame_slot % MAX_FRAMES_IN_FLIGHT]
                .push(DeferredFree::Replaced(replaced));
        }
        Ok(())
    }

    /// Free a texture. Managed textures are destroyed; user textures only
    /// lose their registration (the image and its heap slots belong to the
    /// caller). Prefer [`defer_free`](Self::defer_free) — this is the
    /// immediate path the ring drains into.
    pub fn free_texture(&mut self, id: &TextureId) {
        self.textures.remove(id);
    }

    /// Free a batch of textures; see [`free_texture`](Self::free_texture).
    pub fn free_textures(&mut self, ids: &[TextureId]) {
        for id in ids {
            self.free_texture(id);
        }
    }

    /// Apply the frees deferred into `slot` when it last came around: their
    /// frame-in-flight fence has passed, so the GPU no longer samples them.
    pub fn deferred_free_slot(&mut self, slot: usize) {
        for free in std::mem::take(&mut self.free_ring[slot % MAX_FRAMES_IN_FLIGHT]) {
            match free {
                DeferredFree::Id(id) => self.free_texture(&id),
                DeferredFree::Replaced(entry) => drop(entry),
            }
        }
    }

    /// Defer freeing `ids` until `slot` comes around again
    /// ([`MAX_FRAMES_IN_FLIGHT`] frames later).
    pub fn defer_free(&mut self, slot: usize, ids: Vec<TextureId>) {
        self.free_ring[slot % MAX_FRAMES_IN_FLIGHT].extend(ids.into_iter().map(DeferredFree::Id));
    }

    /// The heap handles bound when a mesh references `id`, for inspection
    /// and future paint callbacks (egui-wgpu parity: `Renderer::texture`).
    pub fn texture(&self, id: &TextureId) -> Option<(TextureHandle, SamplerHandle)> {
        self.textures.get(id).map(TextureEntry::handles)
    }

    /// Register an external image's heap slots as a `TextureId::User`
    /// (egui-wgpu parity: `register_native_texture`). The caller keeps the
    /// image alive and its slots valid until [`free_texture`](Self::free_texture).
    pub fn register_native_texture(
        &mut self,
        texture: TextureHandle,
        sampler: SamplerHandle,
    ) -> TextureId {
        let id = TextureId::User(self.next_user_texture_id);
        self.next_user_texture_id += 1;
        self.textures
            .insert(id, TextureEntry::User(UserTexture { texture, sampler }));
        id
    }

    /// Register an external image's texture slot with a sampler slot created
    /// from egui sampler options (egui-wgpu parity:
    /// `register_native_texture_with_sampler_options`).
    pub fn register_native_texture_with_options(
        &mut self,
        pipeline: &mut EguiPipeline,
        texture: TextureHandle,
        options: TextureOptions,
    ) -> Result<TextureId, String> {
        let sampler = pipeline.sampler_slot(options)?;
        Ok(self.register_native_texture(texture, sampler))
    }
}

impl Drop for EguiTextures {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so no texture or heap slot is destroyed
        // while the GPU still uses it (render-world resources drop in
        // arbitrary order, and the last presented frame may be in flight).
        unsafe {
            let _ = self.device.device().raw().device_wait_idle();
        }
    }
}

/// Per-frame-in-flight GPU resources: vertex/index buffers. Buffers grow by
/// doubling and are never shrunk.
struct FrameResources {
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    vertex_capacity: usize,
    index_capacity: usize,
    mesh_draws: Vec<MeshDraw>,
}

impl FrameResources {
    fn new(device: &Device) -> Result<Self, String> {
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
        Ok(Self {
            vertex_buffer,
            index_buffer,
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
    /// Create one buffer set per frame-in-flight slot.
    pub fn new(device: &Device, frames_in_flight: usize) -> Result<Self, String> {
        let mut frames = Vec::with_capacity(frames_in_flight);
        for _ in 0..frames_in_flight {
            frames.push(FrameResources::new(device)?);
        }
        Ok(Self { frames })
    }

    /// Upload the tessellated mesh data into the given frame slot's buffers,
    /// growing them by doubling when full. Must be called after the slot's
    /// fence has passed and before [`record_egui`] with the same
    /// `primitives`.
    pub fn update(
        &mut self,
        device: &Device,
        frame_slot: usize,
        primitives: &[ClippedPrimitive],
    ) -> Result<(), String> {
        let frame = self
            .frames
            .get_mut(frame_slot)
            .ok_or_else(|| format!("frame slot {frame_slot} out of range"))?;

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
/// and scaled by `pixels_per_point`. Each mesh's root data (screen size,
/// option flags, texture/sampler heap handles) is pushed with
/// [`CommandBuffer::push_data`] right before its draw.
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
    // Heap binding is command-buffer scoped; binding here keeps the pass
    // self-contained for the editor and for tests alike.
    if let Err(e) = pipeline.heap.cmd_bind(command_buffer) {
        moonfield_log::error!("failed to bind descriptor heaps: {e}");
        return;
    }
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
    command_buffer.bind_vertex_buffers(0, &[&frame.vertex_buffer], &[0]);
    command_buffer.bind_index_buffer(&frame.index_buffer, 0, IndexFormat::Uint32);

    let options = pipeline.options();
    let screen_size_in_points = [
        extent.0 as f32 / pixels_per_point,
        extent.1 as f32 / pixels_per_point,
    ];
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
        let (texture, sampler) = entry.handles();
        let root = EguiRoot {
            screen_size_in_points,
            texture: texture.0,
            sampler: sampler.0,
            dithering: options.dithering as u32,
            predictable_filtering: options.predictable_texture_filtering as u32,
        };
        // The root blob is built from reflection: clone the pipeline's
        // template and write the whole `uniform EguiRoot` struct into it.
        let mut root_blob = pipeline.root.clone();
        if let Err(e) = root_blob.set_bytes("root", bytemuck::bytes_of(&root)) {
            moonfield_log::error!("egui root binding failed: {e}");
            continue;
        }
        command_buffer.set_scissor(scissor);
        command_buffer.push_data(0, root_blob.blob());
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
/// entry and two fragment entries (gamma vs. sRGB target), all resources
/// sourced from the descriptor heaps and push data.
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
