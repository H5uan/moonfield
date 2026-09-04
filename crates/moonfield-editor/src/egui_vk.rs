//! egui → Vulkan drawing, as plain data resources plus a recording function.
//!
//! There is no "renderer" object. Persistent GPU state lives in three
//! resources the render world owns — [`EguiPipeline`] (shader modules,
//! graphics pipeline, the descriptor heap), [`EguiTextures`] (egui-managed
//! textures, user-texture registrations, the shared frame uploader), and
//! [`EguiFrameResources`] (per-frame-in-flight
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
use moonfield_rhi::Memory;
use moonfield_rhi::types::WrapMode;
use moonfield_rhi::{
    BlendMode, CommandBuffer, CompareOp, Compiler, CullMode, CullState, DepthState, DescriptorHeap,
    Device, Extent2d, Filter, Format, FrameUploader, FrontFace, GpuAllocation, GraphicsPipeline,
    Offset2d, Rect2d, RenderDevice, RootBinder, SamplerDesc, SamplerHandle, ShaderModule, Texture,
    TextureHandle,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

/// Root data, pushed via [`CommandBuffer::push_data`] and read by the shader
/// through the PushConstant storage class. The static fields (screen size,
/// option flags, vertex/index array pointers) are pushed once per pass; the
/// varying tail (texture/sampler heap slots, the draw's index base) is
/// pushed per draw. Layout must match `EguiRoot` in `egui.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EguiRoot {
    screen_size_in_points: [f32; 2],
    dithering: u32,
    predictable_filtering: u32,
    vertices: u64,
    indices: u64,
    texture: u32,
    sampler: u32,
    index_base: u32,
    _pad0: u32,
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

/// One mesh's draw parameters within a frame slot's shared arrays. The
/// upload rewrites each mesh's local indices to absolute vertex indices, so
/// a draw needs only its index range.
#[derive(Clone, Copy)]
struct MeshDraw {
    index_base: u32,
    index_count: u32,
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
    /// The shared descriptor heap every egui texture/sampler slot lives in
    /// (samplers through its description cache).
    heap: Arc<DescriptorHeap>,
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

        // Reflect the entry point for the root blob and the layout guards.
        let reflection = compiler
            .compile_file_to_reflection(&egui_shader_path(), "vs_main")
            .map_err(|e| e.to_string())?;
        let root = RootBinder::new(&reflection, "vs_main").map_err(|e| e.to_string())?;
        // Layout alignment guard: the Rust `EguiRoot` struct pushed as root
        // data must be exactly as large as the shader's reflected `uniform
        // EguiRoot`, so the two can never silently drift.
        if root.blob().len() != std::mem::size_of::<EguiRoot>() {
            return Err(format!(
                "EguiRoot layout mismatch: Rust struct is {} bytes, egui.slang \
                 root is {} bytes",
                std::mem::size_of::<EguiRoot>(),
                root.blob().len()
            ));
        }
        // The varying fields (texture, sampler, index base) must be the
        // struct's tail: the pass pushes the static prefix once and every
        // draw pushes only the tail.
        if core::mem::offset_of!(EguiRoot, texture) + 16 != std::mem::size_of::<EguiRoot>() {
            return Err(
                "EguiRoot layout mismatch: the varying fields (texture, sampler, \
                 index base) must be the struct's tail"
                    .to_string(),
            );
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
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            pipeline,
            heap: device.descriptor_heap(),
            options,
            callback_resources: CallbackResources { _private: () },
        })
    }

    /// The options the pipeline was built with (the per-draw root data
    /// mirrors them every draw).
    pub fn options(&self) -> &EguiOptions {
        &self.options
    }

    /// The descriptor-heap slot of the sampler for the given egui options
    /// (egui mip levels are always 1, so the mipmap mode only selects the
    /// enum), from the heap's sampler cache.
    fn sampler_slot(&self, options: TextureOptions) -> Result<SamplerHandle, String> {
        self.heap
            .sampler_for(sampler_desc(options))
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

/// The egui texture store: egui-managed textures, registered user textures,
/// and the device's shared frame uploader.
pub struct EguiTextures {
    textures: HashMap<TextureId, TextureEntry>,
    next_user_texture_id: u64,
    /// The device's shared frame uploader, staging this frame's texture
    /// deltas; the window frame loop flushes it once per frame.
    uploader: Arc<Mutex<FrameUploader>>,
}

impl EguiTextures {
    /// Create the store on the shared render device.
    pub fn new(render_device: &RenderDevice) -> Result<Self, String> {
        Ok(Self {
            textures: HashMap::new(),
            next_user_texture_id: 0,
            uploader: render_device.device().uploader(),
        })
    }

    /// Create or update an egui-managed texture from an [`ImageDelta`]. A
    /// delta with `pos: None` (re)allocates the texture; a delta with a
    /// `pos` updates a sub-region of the existing one. A replaced texture
    /// retires through the device's retirement ring: in-flight frames may
    /// still sample its heap slot.
    pub fn update_texture(
        &mut self,
        device: &Device,
        pipeline: &EguiPipeline,
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
            let mut uploader = self.uploader.lock().unwrap_or_else(|e| e.into_inner());
            entry
                .texture
                .upload(
                    &mut uploader,
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
        let texture = {
            let mut uploader = self.uploader.lock().unwrap_or_else(|e| e.into_inner());
            Texture::bindless(
                device,
                &mut uploader,
                width,
                height,
                Format::R8G8B8A8Unorm,
                &bytes,
            )
            .map_err(|e| e.to_string())?
        };
        let texture_handle = texture
            .handle()
            .expect("Texture::bindless always has a slot");
        let entry = TextureEntry::Managed(Box::new(ManagedTexture {
            texture,
            texture_handle,
            sampler,
            options: delta.options,
        }));
        // A replaced entry drops now; its texture and heap slot retire
        // through the device's retirement ring.
        let _ = self.textures.insert(id, entry);
        Ok(())
    }

    /// Free a texture. Managed textures retire through the device's
    /// retirement ring (in-flight frames may still sample them); user
    /// textures only lose their registration (the image and its heap slots
    /// belong to the caller).
    pub fn free_texture(&mut self, id: &TextureId) {
        self.textures.remove(id);
    }

    /// Free a batch of textures; see [`free_texture`](Self::free_texture).
    pub fn free_textures(&mut self, ids: &[TextureId]) {
        for id in ids {
            self.free_texture(id);
        }
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
        pipeline: &EguiPipeline,
        texture: TextureHandle,
        options: TextureOptions,
    ) -> Result<TextureId, String> {
        let sampler = pipeline.sampler_slot(options)?;
        Ok(self.register_native_texture(texture, sampler))
    }
}

/// Per-frame-in-flight GPU resources: the vertex and index arrays. Allocated
/// in host-visible memory (rewritten wholesale every frame) and grown by
/// doubling, never shrunk.
struct FrameResources {
    vertices: GpuAllocation,
    indices: GpuAllocation,
    vertex_capacity: usize,
    index_capacity: usize,
    mesh_draws: Vec<MeshDraw>,
}

impl FrameResources {
    fn new(device: &Device) -> Result<Self, String> {
        let vertices = GpuAllocation::new(
            device,
            (INITIAL_VERTEX_CAPACITY * std::mem::size_of::<PodVertex>()) as u64,
            Memory::Default,
        )
        .map_err(|e| e.to_string())?;
        let indices = GpuAllocation::new(
            device,
            (INITIAL_INDEX_CAPACITY * std::mem::size_of::<u32>()) as u64,
            Memory::Default,
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            vertices,
            indices,
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            index_capacity: INITIAL_INDEX_CAPACITY,
            mesh_draws: Vec::new(),
        })
    }
}

/// The per-slot buffer ring. `frames_in_flight` must match the frame loop
/// (the window surfaces' [`moonfield_render_core::MAX_FRAMES_IN_FLIGHT`]);
/// each slot is only written after its fence has passed (i.e. inside the
/// frame, after acquire).
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

    /// Upload the tessellated mesh data into the given frame slot's arrays,
    /// growing them by doubling when full. Each mesh's local indices are
    /// rewritten to absolute vertex indices, so a draw needs only its index
    /// range. Must be called after the slot's fence has passed and before
    /// [`record_egui`] with the same `primitives`.
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
                index_base: indices.len() as u32,
                index_count: mesh.indices.len() as u32,
            });
            let vertex_offset = vertices.len() as u32;
            vertices.extend(mesh.vertices.iter().map(|v| PodVertex {
                pos: [v.pos.x, v.pos.y],
                uv: [v.uv.x, v.uv.y],
                color: u32::from_le_bytes(v.color.to_array()),
            }));
            indices.extend(mesh.indices.iter().map(|i| i + vertex_offset));
        }

        if vertices.len() > frame.vertex_capacity {
            frame.vertex_capacity = vertices.len().next_power_of_two();
            frame.vertices = GpuAllocation::new(
                device,
                (frame.vertex_capacity * std::mem::size_of::<PodVertex>()) as u64,
                Memory::Default,
            )
            .map_err(|e| e.to_string())?;
        }
        if indices.len() > frame.index_capacity {
            frame.index_capacity = indices.len().next_power_of_two();
            frame.indices = GpuAllocation::new(
                device,
                (frame.index_capacity * std::mem::size_of::<u32>()) as u64,
                Memory::Default,
            )
            .map_err(|e| e.to_string())?;
        }
        // Host-visible allocations: write through the mapped view directly.
        if !vertices.is_empty() {
            let host = frame
                .vertices
                .host()
                .ok_or_else(|| "vertex allocation lost its host view".to_string())?;
            // SAFETY: the allocation spans `vertex_capacity` slots (grown
            // above to fit) and nothing else aliases this frame slot's view.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    vertices.as_ptr(),
                    host.typed::<PodVertex>(),
                    vertices.len(),
                );
            }
        }
        if !indices.is_empty() {
            let host = frame
                .indices
                .host()
                .ok_or_else(|| "index allocation lost its host view".to_string())?;
            // SAFETY: as above, `index_capacity` slots, no aliases.
            unsafe {
                std::ptr::copy_nonoverlapping(indices.as_ptr(), host.typed::<u32>(), indices.len());
            }
        }
        frame.mesh_draws = mesh_draws;
        Ok(())
    }
}

/// Record the draw commands for the primitives uploaded by
/// [`EguiFrameResources::update`] into the caller's open render pass.
/// `extent` is the render target's pixel extent; clip rects are in points
/// and scaled by `pixels_per_point`. The static root fields (screen size,
/// option flags) are pushed once per pass; each mesh pushes only its
/// texture/sampler heap handles right before its draw.
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
    // The command buffer's owner (the frame loop, or the test) has bound
    // the descriptor heaps; this pass only records state and draws.
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

    let options = pipeline.options();
    let screen_size_in_points = [
        extent.0 as f32 / pixels_per_point,
        extent.1 as f32 / pixels_per_point,
    ];
    // The static root prefix (screen size, option flags, and the vertex/
    // index array pointers) is pushed once per pass; every draw pushes only
    // the varying tail (texture + sampler slots, its index base) at its
    // offset — bytes outside a written range keep their values
    // (GPU-verified by `command_push_data`).
    let static_prefix = EguiRoot {
        screen_size_in_points,
        dithering: options.dithering as u32,
        predictable_filtering: options.predictable_texture_filtering as u32,
        vertices: frame.vertices.gpu().as_raw(),
        indices: frame.indices.gpu().as_raw(),
        texture: 0,
        sampler: 0,
        index_base: 0,
        _pad0: 0,
    };
    let static_len = core::mem::offset_of!(EguiRoot, texture);
    command_buffer.push_data(0, &bytemuck::bytes_of(&static_prefix)[..static_len]);
    let varying_offset = core::mem::offset_of!(EguiRoot, texture) as u32;
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
        command_buffer.set_scissor(scissor);
        let varying = [texture.0, sampler.0, draw.index_base, 0u32];
        command_buffer.push_data(varying_offset, bytemuck::bytes_of(&varying));
        // Non-indexed draw: `vid` runs over the mesh's index range and the
        // vertex shader pulls both arrays through the root's pointers.
        command_buffer.draw(draw.index_count, 1, 0, 0);
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
