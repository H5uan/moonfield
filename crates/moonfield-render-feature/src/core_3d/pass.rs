//! The core 3D opaque pass: a per-view system, not a renderer object.
//!
//! Bevy-style, the pass owns nothing. Persistent GPU state lives in
//! render-world resources — [`Core3dPipeline`] (the flat-lit mesh pipeline)
//! and [`ViewTargets`] (one offscreen target per logical [`RenderTarget`])
//! — and the frame's command buffer comes from
//! [`FrameContext`](moonfield_render_core::FrameContext) between
//! `acquire_window_frames` and `submit_window_frames`.
//!
//! `prepare_view_targets`, `prepare_core_3d_pipeline`, and
//! `begin_frame_draw_arena` run in the `PrepareViews` set; the camera driver
//! then runs [`opaque_pass_3d`] once per extracted view (the `Core3d`
//! schedule). Offscreen views (the editor viewport) draw into their target
//! ending in `ShaderRead`; a window view draws straight into each
//! in-progress surface's swapchain image (ending in `Present`),
//! depth-tested against the surface's own depth buffer. Offscreen targets
//! no view claims are cleared to a dim background by
//! [`clear_orphan_view_targets`].

use moonfield_app::prelude::{Query, Res, World};
use moonfield_asset::{AssetRevision, Handle};
use moonfield_camera::RenderTarget;
use moonfield_log::{error, error_once, info};
use moonfield_render_core::{
    CurrentView, DrawFunctions, ExtractedView, FrameContext, PhaseItem, RenderPhase, ViewTargets,
    WindowSurfaces,
};
use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBuffer, CompareOp, CullMode, CullState, DepthState,
    Format, FrontFace, GraphicsPipeline, LoadOp, Rect2d, RenderAttachment, RenderDevice,
    RenderPassDesc, Result, RootBinder, RootParamPlace, ShaderModule, StoreOp, TextureView,
    Viewport,
};
use moonfield_shader::Shader;
use std::collections::{HashMap, HashSet};

use crate::render_phase::{FrameDrawArena, Opaque3d, ViewUniforms};
use crate::shader::{
    PipelineShader, PipelineShaders, PreparedShader, PreparedShaders, ShaderEntry,
};

/// Initial offscreen target size; consumers (e.g. the editor's viewport
/// panel) report real sizes through [`RenderTargetSizes`].
pub(crate) const INITIAL_WIDTH: u32 = 1280;
pub(crate) const INITIAL_HEIGHT: u32 = 720;

/// The name keying the core 3D pipeline's shader in [`PipelineShaders`] and
/// [`PreparedShaders`].
pub const CORE_3D_SHADER: &str = "core_3d";

/// The core 3D pipeline's shader request: `core_3d.slang`, a vertex and a
/// fragment entry, root binding reflected from `vs_main`. The caller (the
/// editor, at startup) supplies the loaded asset handle.
pub fn core_3d_shader(shader: Handle<Shader>) -> PipelineShader {
    PipelineShader {
        pipeline: CORE_3D_SHADER,
        shader,
        reflect_entry: "vs_main",
        entries: &[
            ShaderEntry {
                name: "vs_main",
                capabilities: &[],
            },
            ShaderEntry {
                name: "fs_main",
                capabilities: &[],
            },
        ],
    }
}

/// The flat-lit mesh pipeline of the core 3D pass, as a render-world
/// resource. [`prepare_core_3d_pipeline`] (re)builds it from
/// [`PreparedShaders`] whenever the prepared shader's revision advances
/// past the one the pipeline was built from.
pub struct Core3dPipeline {
    pipeline: GraphicsPipeline,
    /// The `Ptr<DrawData>` root's reflected placement. A draw encodes the
    /// arena address on the stack and pushes it at the place's offset —
    /// no per-draw allocation, no name lookup.
    root: RootParamPlace,
    /// The `Ptr<ViewUniforms>` root's reflected placement; the pass pushes
    /// its address once per pass.
    view: RootParamPlace,
    /// The shader asset the pipeline was built from.
    shader: Handle<Shader>,
    /// The prepared-shader revision the pipeline was built from.
    shader_revision: AssetRevision,
}

impl Core3dPipeline {
    /// Build the pipeline for the view-target format from a prepared shader
    /// (compiled from the extracted asset by `prepare_shaders`).
    pub fn new(
        render_device: &RenderDevice,
        shader: Handle<Shader>,
        prepared: &PreparedShader,
    ) -> Result<Self> {
        let device = render_device.device();
        let binder = RootBinder::new(prepared.reflection(), "vs_main")?;
        let root = binder.pointer_param("root")?;
        let view = binder.pointer_param("view")?;
        let entry = |name: &str| {
            prepared.entry(name).ok_or_else(|| {
                moonfield_rhi::Error::Backend(format!(
                    "prepared core 3d shader is missing '{name}'"
                ))
            })
        };
        let vertex_shader = ShaderModule::from_compiled(device, entry("vs_main")?)?;
        let fragment_shader = ShaderModule::from_compiled(device, entry("fs_main")?)?;
        // Descriptor-heap pipeline: per-draw root pointers go through `push_data`.
        let pipeline = GraphicsPipeline::new_with_options(
            device,
            &[VIEW_TARGET_FORMAT],
            Some(Format::D32Sfloat),
            &vertex_shader,
            &fragment_shader,
        )?;
        Ok(Self {
            pipeline,
            root,
            view,
            shader,
            shader_revision: prepared.revision(),
        })
    }

    /// The graphics pipeline for per-draw binding.
    pub fn pipeline(&self) -> &GraphicsPipeline {
        &self.pipeline
    }

    /// The `Ptr<DrawData>` root placement for per-draw encoding.
    pub fn root(&self) -> RootParamPlace {
        self.root
    }

    /// The `Ptr<ViewUniforms>` root placement for the per-pass push.
    pub fn view(&self) -> RootParamPlace {
        self.view
    }
}

/// The color format of offscreen view targets.
pub const VIEW_TARGET_FORMAT: Format = Format::B8G8R8A8Unorm;

/// Physical sizes requested for logical render targets, written by consumers
/// (the editor writes the `Viewport` entry from its panel size each frame).
#[derive(Default)]
pub struct RenderTargetSizes(pub HashMap<RenderTarget, (u32, u32)>);

/// `PrepareViews` system: ensure every offscreen view target has an
/// attachment of the requested size.
pub fn prepare_view_targets(world: &mut World) {
    let requested: Vec<RenderTarget> = world
        .query::<&ExtractedView>()
        .map(|(_, view)| view.target.0)
        // Only offscreen targets need attachments here; window-targeted
        // views resolve against the surface's swapchain image and depth
        // buffer at record time.
        .filter(|target| matches!(target, RenderTarget::Viewport))
        .collect();
    if requested.is_empty() {
        return;
    }
    let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone()) else {
        return;
    };
    if !world.contains_resource::<ViewTargets>() {
        world.insert_resource(ViewTargets::default());
    }
    let sizes = world.get_resource::<RenderTargetSizes>();
    let mut targets = world
        .get_resource_mut::<ViewTargets>()
        .expect("ViewTargets was just ensured");
    for target in requested {
        let (width, height) = sizes
            .as_deref()
            .and_then(|sizes| sizes.0.get(&target))
            .copied()
            .unwrap_or((INITIAL_WIDTH, INITIAL_HEIGHT));
        targets.ensure(target, width, height, VIEW_TARGET_FORMAT, &render_device);
    }
}

/// `PrepareViews` system: (re)build the core 3D pipeline when its prepared
/// shader advanced past the one the pipeline was built from. While the
/// shader is not ready (never compiled, or the latest compile failed) the
/// pass keeps the pipeline it has, or skips with a one-shot log.
pub fn prepare_core_3d_pipeline(world: &mut World) {
    let request = world
        .get_resource::<PipelineShaders>()
        .and_then(|requests| requests.get(CORE_3D_SHADER).copied());
    let Some(request) = request else {
        error_once!("no '{CORE_3D_SHADER}' shader registered; skipping the core 3d pass");
        return;
    };
    let built = {
        let prepared = world
            .get_resource::<PreparedShaders>()
            .expect("PreparedShaders registered by RenderFeaturePlugin");
        match prepared.get(CORE_3D_SHADER) {
            Some(shader) => {
                let stale = world
                    .get_resource::<Core3dPipeline>()
                    .is_none_or(|pipeline| {
                        pipeline.shader != request.shader
                            || pipeline.shader_revision != shader.revision()
                    });
                if !stale {
                    None
                } else {
                    let render_device = world.get_resource::<RenderDevice>().map(|d| (*d).clone());
                    render_device.map(|render_device| {
                        Core3dPipeline::new(&render_device, request.shader, shader)
                    })
                }
            }
            None => {
                if !world.contains_resource::<Core3dPipeline>() {
                    error_once!("the core 3d shader is not ready; skipping the core 3d pass");
                }
                None
            }
        }
    };
    match built {
        Some(Ok(pipeline)) => {
            world.insert_resource(pipeline);
        }
        Some(Err(e)) => {
            error!("failed to build core 3d pipeline: {e}");
        }
        None => {}
    }
}

/// `PrepareViews` system: create the frame draw arena on first use and
/// begin its frame slot for this frame's allocations.
pub fn begin_frame_draw_arena(world: &mut World) {
    if !world.contains_resource::<FrameDrawArena>()
        && let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone())
    {
        match FrameDrawArena::new(render_device.device()) {
            Ok(arena) => world.insert_resource(arena),
            Err(e) => error!("failed to create frame draw arena: {e}"),
        }
    }
    let Some(frame_context) = world.get_resource::<FrameContext>() else {
        return;
    };
    if !frame_context.frame_in_progress() {
        return;
    }
    let slot = frame_context.current_slot();
    if let Some(arena) = world.get_resource::<FrameDrawArena>() {
        arena.begin_frame(slot);
    }
}

/// A resolved draw target for one pass: the color/depth attachment views, the
/// extent, and the layout the color attachment is left in. Offscreen targets
/// end in `ShaderRead` (sampled by the UI); window surfaces end in `Present`.
pub struct PassTarget {
    /// The color attachment view (offscreen target or swapchain image).
    pub color: TextureView,
    /// The depth attachment view, when the pass is depth-tested.
    pub depth: Option<TextureView>,
    /// The target's `(width, height)`.
    pub extent: (u32, u32),
    /// The layout the color attachment is transitioned to.
    pub final_color_layout: AttachmentLayout,
}

/// Record one view's opaque pass into `command_buffer`: clear color and
/// depth, then dispatch every queued [`Opaque3d`] item to its registered
/// draw function. `world` reaches the arena, the pipeline, and the draw
/// functions' prepared data.
pub fn record_view_pass(
    world: &World,
    view: &ExtractedView,
    phase: &RenderPhase<Opaque3d>,
    target: PassTarget,
    draw_functions: &DrawFunctions<Opaque3d>,
    command_buffer: &CommandBuffer,
) {
    let (width, height) = target.extent;
    let clear_color = view.camera.clear_color;

    // Debug seam: MOONFIELD_DEBUG_SCENE=1 logs the scene contents once.
    if std::env::var_os("MOONFIELD_DEBUG_SCENE").is_some() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let camera_pos = view.world_from_view.affine().translation;
            info!(
                "scene: camera=({:.1}, {:.1}, {:.1}) items={} extent=({width}, {height})",
                camera_pos.x,
                camera_pos.y,
                camera_pos.z,
                phase.items().len(),
            );
            for item in phase.items() {
                info!("  item model: {:?}", item.model.to_cols_array());
            }
        });
    }

    let color_attachment = RenderAttachment {
        view: target.color,
        layout: target.final_color_layout,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color(clear_color),
    };
    // Reverse-Z: the depth clear value is 0.0 (near → 1).
    let depth_attachment = target.depth.map(|view| RenderAttachment {
        view,
        layout: AttachmentLayout::DepthStencil,
        load: LoadOp::Clear,
        store: StoreOp::Discard,
        clear: ClearValue::DepthStencil {
            depth: 0.0,
            stencil: 0,
        },
    });
    let begin_info = RenderPassDesc {
        render_area: Rect2d::full(width, height),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment,
    };

    command_buffer.begin_rendering(&begin_info);
    // The engine's projection is Y-up NDC; Vulkan framebuffers are
    // top-left origin. The negative-height viewport performs the flip
    // at the Vulkan boundary (see AGENTS.md clip-space note).
    command_buffer.set_viewport(Viewport::y_flipped(width, height));
    // Reverse-Z depth state + back-face culling with the flipped
    // viewport (front face = clockwise after the Y flip).
    command_buffer.set_depth_state(DepthState {
        test_enable: true,
        write_enable: true,
        compare_op: CompareOp::GreaterOrEqual,
    });
    command_buffer.set_cull_state(CullState {
        cull_mode: CullMode::None,
        front_face: FrontFace::Clockwise,
    });
    // Per-pass view uniforms: one arena record, its address pushed
    // once. The aspect comes from the target's real extent.
    let translation = view.world_from_view.affine().translation;
    let uniforms = ViewUniforms {
        view_proj: view
            .clip_from_world(width as f32 / height.max(1) as f32)
            .to_cols_array(),
        view_pos: [translation.x, translation.y, translation.z],
        _pad0: 0.0,
    };
    let recorded = (|| -> Option<()> {
        let arena = world.get_resource::<FrameDrawArena>()?;
        let pipeline = world.get_resource::<Core3dPipeline>()?;
        let record = arena.alloc_view_uniforms().ok()?;
        unsafe {
            *record.cpu.typed::<ViewUniforms>() = uniforms;
        }
        let place = pipeline.view();
        let bytes = place.pointer_bytes(record.gpu.as_raw()).ok()?;
        command_buffer.push_data(place.offset as u32, &bytes);
        Some(())
    })();
    if recorded.is_none() {
        error!("failed to record view uniforms; skipping view items");
        command_buffer.end_rendering();
        return;
    }
    for item in phase.items() {
        let Some(draw) = draw_functions.get(item.draw_function()) else {
            continue;
        };
        draw.draw(world, item, command_buffer);
    }
    command_buffer.end_rendering();
}

/// Record a clear-only pass into `target` — the dim background shown when
/// no view claims the target this frame.
pub fn record_clear_pass(target: PassTarget, command_buffer: &CommandBuffer) {
    let color_attachment = RenderAttachment {
        view: target.color,
        layout: target.final_color_layout,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.05, 0.0, 0.08, 1.0]),
    };
    let depth_attachment = target.depth.map(|view| RenderAttachment {
        view,
        layout: AttachmentLayout::DepthStencil,
        load: LoadOp::Clear,
        store: StoreOp::Discard,
        clear: ClearValue::DepthStencil {
            depth: 0.0,
            stencil: 0,
        },
    });
    command_buffer.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(target.extent.0, target.extent.1),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment,
    });
    command_buffer.end_rendering();
}

/// `Core3d` system: record the current view's opaque pass into the frame's
/// command buffer. Exclusive — the draw functions read prepared data from
/// the world. No-ops when no frame is in progress (no device, or the begin
/// failed) or the view's phase is missing.
pub fn opaque_pass_3d(world: &mut World) {
    let Some(view_entity) = world.get_resource::<CurrentView>().map(|view| view.0) else {
        return;
    };
    let Some(view) = world.get_component::<ExtractedView>(view_entity).copied() else {
        return;
    };
    let Some(phase) = world.get_component::<RenderPhase<Opaque3d>>(view_entity) else {
        return;
    };
    let Some(frame_context) = world.get_resource::<FrameContext>() else {
        return;
    };
    if !frame_context.frame_in_progress() {
        return;
    }
    let Some(command_buffer) = frame_context.current_command_buffer() else {
        return;
    };
    let Some(draw_functions) = world.get_resource::<DrawFunctions<Opaque3d>>() else {
        return;
    };

    match view.target.0 {
        RenderTarget::Viewport => {
            let Some(targets) = world.get_resource::<ViewTargets>() else {
                return;
            };
            let Some(target) = targets.get(RenderTarget::Viewport) else {
                return;
            };
            let pass_target = PassTarget {
                color: target.view(),
                depth: target.depth_view(),
                extent: target.extent(),
                final_color_layout: AttachmentLayout::ShaderRead,
            };
            record_view_pass(
                world,
                &view,
                phase,
                pass_target,
                &draw_functions,
                command_buffer,
            );
        }
        RenderTarget::PrimaryWindow => {
            let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
                return;
            };
            for data in surfaces.values_mut() {
                if !data.frame_in_progress() {
                    continue;
                }
                // The pipeline is baked for the view-target format; a swapchain
                // with a different (e.g. sRGB) format cannot be drawn into yet.
                match data.format() {
                    Ok((VIEW_TARGET_FORMAT, _)) => {}
                    Ok((other, _)) => {
                        error_once!(
                            "window swapchain format {other:?} is not {VIEW_TARGET_FORMAT:?}; \
                             skipping the window pass"
                        );
                        continue;
                    }
                    Err(e) => {
                        error_once!("failed to read window surface format: {e}");
                        continue;
                    }
                }
                let (Some(color), Some(depth)) = (data.current_image_view(), data.depth_view())
                else {
                    continue;
                };
                let extent = data.extent();
                let pass_target = PassTarget {
                    color,
                    depth: Some(depth),
                    extent: (extent.width, extent.height),
                    final_color_layout: AttachmentLayout::Present,
                };
                record_view_pass(
                    world,
                    &view,
                    phase,
                    pass_target,
                    &draw_functions,
                    command_buffer,
                );
            }
        }
    }
}

/// `Render` system (after the camera driver): clear every offscreen target
/// no view claimed this frame, so stale frames do not linger.
pub fn clear_orphan_view_targets(
    views: Query<&ExtractedView>,
    frame: Option<Res<FrameContext>>,
    targets: Option<Res<ViewTargets>>,
) {
    let (Some(frame), Some(targets)) = (frame, targets) else {
        return;
    };
    if !frame.frame_in_progress() {
        return;
    }
    let Some(command_buffer) = frame.current_command_buffer() else {
        return;
    };
    let claimed: HashSet<RenderTarget> = views.iter().map(|(_, view)| view.target.0).collect();
    for (target_key, target) in targets.iter() {
        if claimed.contains(target_key) {
            continue;
        }
        record_clear_pass(
            PassTarget {
                color: target.view(),
                depth: target.depth_view(),
                extent: target.extent(),
                final_color_layout: AttachmentLayout::ShaderRead,
            },
            command_buffer,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Mesh, MeshHandle, MeshRenderer};
    use crate::shader::PipelineShaders;
    use moonfield_app::{App, ExtractSchedule};
    use moonfield_asset::Assets;
    use moonfield_camera::{Camera, PrimaryCamera};
    use moonfield_math::{GlobalTransform, Transform, Vec3};
    use moonfield_rhi::{CommandBufferUsage, CommandPool, OffscreenTarget};

    const TEST_QUAD_VERTICES: &[[f32; 3]] = &[
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ];
    const TEST_QUAD_INDICES: &[u32] = &[0, 3, 2, 2, 1, 0];

    /// The repository's `core_3d.slang` source — the same file the editor
    /// loads through the asset server at startup.
    fn core_3d_source() -> String {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/shaders/core_3d.slang");
        std::fs::read_to_string(path).expect("core_3d.slang")
    }

    /// A headless Vulkan device, or `None` (test skips) when no driver is
    /// available. GPU tests hold `GPU_LOCK` for their whole body.
    fn headless_device() -> Option<RenderDevice> {
        match RenderDevice::new() {
            Ok(device) => Some(device),
            Err(err) => {
                eprintln!("skipping: no Vulkan device available ({err})");
                None
            }
        }
    }

    /// A camera plus one test quad per `(color, transform)`, drawn in slice
    /// order.
    fn mesh_world(render_device: &RenderDevice, meshes_to_spawn: &[([f32; 4], Transform)]) -> App {
        let mut app = App::new();
        // Device first (the real plugin order), so resources LIFO-drop
        // before it and the device's teardown guard stays on the clean path.
        app.render_world_mut()
            .insert_resource(render_device.clone());
        app.add_plugin(crate::RenderFeaturePlugin);
        // The pass builds the pipeline during `render()` from the prepared
        // shader: register the shader asset and the pipeline's request in
        // the main world, like the editor's startup load does.
        let shader = app
            .world()
            .get_resource_mut::<Assets<Shader>>()
            .expect("Assets<Shader> registered by RenderFeaturePlugin")
            .add(Shader::new(core_3d_source(), "core_3d.slang".into()));
        app.world()
            .get_resource_mut::<PipelineShaders>()
            .expect("PipelineShaders registered by RenderFeaturePlugin")
            .push(core_3d_shader(shader));
        // The draw function allocates per-draw root data from this arena; the
        // test drives slot 0 manually (no window frame loop in headless mode).
        app.render_world_mut()
            .insert_resource(FrameDrawArena::new(render_device.device()).expect("arena"));
        app.add_render_systems(ExtractSchedule, moonfield_render_core::extract_cameras);
        app.world_mut().spawn((
            Camera::default(),
            PrimaryCamera,
            GlobalTransform::from(
                Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
            ),
        ));
        let mut meshes = Assets::<Mesh>::default();
        let mesh = MeshHandle(meshes.add(Mesh::new(
            TEST_QUAD_VERTICES.to_vec(),
            TEST_QUAD_INDICES.to_vec(),
            None,
        )));
        app.world_mut().insert_resource(meshes);
        for &(color, transform) in meshes_to_spawn {
            app.world_mut().spawn((
                MeshRenderer::new(mesh, color),
                GlobalTransform::from(transform),
            ));
        }
        app.render();
        app
    }

    /// Record the opaque pass into a fresh target and read its BGRA pixels
    /// back. A second, empty render pass follows in the same command buffer —
    /// the editor's UI-pass pattern — to cover pass-to-pass state resets.
    fn record_and_readback(app: &App, render_device: &RenderDevice) -> ((u32, u32), Vec<u8>) {
        let device = render_device.device();
        // The mesh uploads recorded during preparation ride the shared
        // uploader; flush it ahead of this command buffer (the frame loop
        // does the same at submit).
        device
            .uploader()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .end_frame()
            .expect("flush uploads");
        let target = OffscreenTarget::new_with_depth(
            device,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            VIEW_TARGET_FORMAT,
        )
        .expect("target");

        let world = app.render_world();
        let view_entity = world
            .query::<&ExtractedView>()
            .next()
            .map(|(entity, _)| entity)
            .expect("extracted view");
        let view = world
            .get_component::<ExtractedView>(view_entity)
            .copied()
            .expect("ExtractedView");
        let phase = world
            .get_component::<RenderPhase<Opaque3d>>(view_entity)
            .expect("opaque phase");
        let draw_functions = world
            .get_resource::<DrawFunctions<Opaque3d>>()
            .expect("DrawFunctions<Opaque3d>");
        assert!(
            world.get_resource::<Core3dPipeline>().is_some(),
            "pipeline built during render"
        );

        // Headless tests drive the arena's slot 0 directly (the window frame
        // loop that would reset/advance it does not run here); single slot,
        // submitted and waited below, needs no ring.
        world
            .get_resource::<FrameDrawArena>()
            .expect("FrameDrawArena inserted by mesh_world")
            .begin_frame(0);

        let command_pool =
            CommandPool::new(device, device.queue_family_indices().graphics).expect("pool");
        let mut command_buffer = command_pool.allocate_command_buffer().expect("cmd");
        command_buffer
            .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
            .expect("begin");
        let pass_target = PassTarget {
            color: target.view(),
            depth: target.depth_view(),
            extent: target.extent(),
            final_color_layout: AttachmentLayout::ShaderRead,
        };
        record_view_pass(
            world,
            &view,
            phase,
            pass_target,
            &draw_functions,
            &command_buffer,
        );

        // The trailing UI-pass pattern: another pass in the same buffer.
        let ui_target =
            OffscreenTarget::new(device, 64, 64, Format::B8G8R8A8Unorm).expect("ui target");
        let ui_color = RenderAttachment {
            view: ui_target.view(),
            layout: AttachmentLayout::ShaderRead,
            load: LoadOp::Clear,
            store: StoreOp::Store,
            clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
        };
        command_buffer.begin_rendering(&RenderPassDesc {
            render_area: Rect2d::full(64, 64),
            layer_count: 1,
            color_attachments: std::slice::from_ref(&ui_color),
            depth_attachment: None,
        });
        command_buffer.end_rendering();
        command_buffer.end().expect("end");

        device
            .submit_and_wait(&[&command_buffer])
            .expect("submit and wait");

        (
            target.extent(),
            target.read_pixels(device).expect("readback"),
        )
    }

    /// The opaque pass must rasterize a mesh in front of the primary camera.
    #[test]
    fn test_opaque_pass_draws_mesh() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let Some(render_device) = headless_device() else {
            return;
        };
        let app = mesh_world(
            &render_device,
            &[([1.0, 0.0, 0.0, 1.0], Transform::from_xyz(-0.75, 0.0, 0.0))],
        );
        let ((width, height), pixels) = record_and_readback(&app, &render_device);

        if std::env::var_os("MOONFIELD_DEBUG_SCENE").is_some() {
            std::fs::create_dir_all("../../target/tmp").unwrap();
            std::fs::write(
                format!("../../target/tmp/scene_test_{width}x{height}.raw"),
                &pixels,
            )
            .unwrap();
        }
        // Compare against the clear color with rounding tolerance (the GPU
        // rounds unorm values, `(x * 255.0) as u8` truncates).
        let clear = Camera::default().clear_color;
        let is_clear = |px: &[u8]| {
            let (b, g, r, a) = (px[0] as i32, px[1] as i32, px[2] as i32, px[3]);
            (b - (clear[2] * 255.0).round() as i32).abs() <= 1
                && (g - (clear[1] * 255.0).round() as i32).abs() <= 1
                && (r - (clear[0] * 255.0).round() as i32).abs() <= 1
                && a == 255
        };
        let non_clear = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| !is_clear(px.as_slice()))
            .count();
        assert!(
            non_clear > 1000,
            "mesh did not rasterize: only {non_clear} non-clear pixels"
        );
    }

    /// Depth direction: near red and far blue quads overlap at screen center;
    /// the center pixel must be red even though the blue mesh is drawn second.
    #[test]
    fn test_opaque_pass_depth_occludes() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let Some(render_device) = headless_device() else {
            return;
        };
        let app = mesh_world(
            &render_device,
            &[
                (
                    [1.0, 0.0, 0.0, 1.0],
                    Transform {
                        translation: Vec3::new(0.0, 0.5, 2.0),
                        scale: Vec3::splat(4.0),
                        ..Transform::IDENTITY
                    },
                ),
                (
                    [0.0, 0.0, 1.0, 1.0],
                    Transform {
                        translation: Vec3::new(0.0, 0.5, -3.0),
                        scale: Vec3::splat(4.0),
                        ..Transform::IDENTITY
                    },
                ),
            ],
        );
        let ((width, height), pixels) = record_and_readback(&app, &render_device);

        let center = ((height / 2) * width + width / 2) as usize;
        let px = &pixels[center * 4..center * 4 + 4];
        // BGRA: the near red mesh wins; the flat shading dims but never
        // swaps channels (red shade ≥ 0.35 → r ≥ 89).
        assert!(
            px[2] > 80 && px[0] < 40 && px[1] < 40,
            "center pixel must be red (near mesh occludes), got BGRA {px:?}"
        );
    }
}
