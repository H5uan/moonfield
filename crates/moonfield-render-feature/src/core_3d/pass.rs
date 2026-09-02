//! The core 3D opaque pass: a plain system, not a renderer object.
//!
//! Bevy-style, the pass owns nothing. Persistent GPU state lives in two
//! render-world resources — [`Core3dPipeline`] (the flat-lit mesh pipeline)
//! and [`ViewTargets`] (one offscreen target per logical
//! [`RenderTarget`]) — and the frame's command buffer comes from
//! [`WindowSurfaces`](moonfield_render_core::WindowSurfaces) between
//! `acquire_window_frames` and `submit_window_frames`.
//! [`main_opaque_pass_3d`] reads the [`Core3dFrame`] built in `RenderQueue`
//! and records one render pass per view whose target has an attachment.
//!
//! With no primary camera targeting a view target, the target is cleared to
//! a dim background color.

use moonfield_app::prelude::World;
use moonfield_camera::RenderTarget;
use moonfield_log::{error, info};
use moonfield_render_core::{ViewTargets, WindowSurfaces};
use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBuffer, CompareOp, Compiler, CullMode, CullState,
    DepthState, Format, FrontFace, GraphicsPipeline, LoadOp, OffscreenTarget, Rect2d,
    RenderAttachment, RenderDevice, RenderPassDesc, Result, RootBinder, ShaderModule, StoreOp,
    VertexBufferLayout, Viewport,
};
use std::collections::HashMap;
use std::path::PathBuf;

use moonfield_render_core::{DrawFunctions, PhaseItem};

use super::{Core3dFrame, Core3dView};
use crate::render_phase::{FrameDrawArena, Opaque3d};

/// Initial offscreen target size; consumers (e.g. the editor's viewport
/// panel) report real sizes through [`RenderTargetSizes`]. Queue systems use
/// the same fallback when computing view aspect ratios.
pub(crate) const INITIAL_WIDTH: u32 = 1280;
pub(crate) const INITIAL_HEIGHT: u32 = 720;

/// Resolve a repository shader file under `<repo root>/assets/shaders/`.
///
/// `CARGO_MANIFEST_DIR` is a compile-time absolute path, so the file resolves
/// whichever directory the process runs from (`cargo run` from the workspace
/// root, `cargo test` from a crate directory).
fn shader_path(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/shaders")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// Compile both stages of `core_3d.slang` and derive the vertex layout and
/// root blob from the reflected entry points — the shader is the single
/// source of truth for both.
fn compile_core_3d(
    compiler: &Compiler,
    device: &moonfield_rhi::Device,
) -> Result<(ShaderModule, ShaderModule, VertexBufferLayout, RootBinder)> {
    let reflection =
        compiler.compile_file_to_reflection(&shader_path("core_3d.slang"), "vs_main")?;
    let vertex_layout = reflection.vertex_layout("vs_main")?;
    let root = RootBinder::new(&reflection, "vs_main")?;
    drop(reflection);

    let vertex_shader = ShaderModule::from_compiled(
        device,
        &compiler.compile_file_to_spirv(&shader_path("core_3d.slang"), "vs_main")?,
    )?;
    let fragment_shader = ShaderModule::from_compiled(
        device,
        &compiler.compile_file_to_spirv(&shader_path("core_3d.slang"), "fs_main")?,
    )?;
    Ok((vertex_shader, fragment_shader, vertex_layout, root))
}

/// The flat-lit mesh pipeline of the core 3D pass, as a render-world
/// resource (lazily created by [`main_opaque_pass_3d`] from the
/// [`RenderDevice`], the plain-data counterpart of Bevy's
/// `init_gpu_resource`).
pub struct Core3dPipeline {
    pipeline: GraphicsPipeline,
    /// Reflection-built root blob template (`Ptr<DrawData>` → one GPU
    /// address); draws clone it, set the pointer, and push it.
    root: RootBinder,
}

impl Core3dPipeline {
    /// Compile the shaders and build the pipeline for the view-target format.
    pub fn new(render_device: &RenderDevice) -> Result<Self> {
        let device = render_device.device();
        let compiler = Compiler::new()?;
        let (vertex_shader, fragment_shader, vertex_layout, root) =
            compile_core_3d(&compiler, device)?;
        // Descriptor-heap pipeline: per-draw root pointers go through `push_data`.
        let pipeline = GraphicsPipeline::new_with_options(
            device,
            &[VIEW_TARGET_FORMAT],
            Some(Format::D32Sfloat),
            &vertex_shader,
            &fragment_shader,
            &vertex_layout,
        )?;
        Ok(Self { pipeline, root })
    }

    /// The graphics pipeline for per-draw binding.
    pub fn pipeline(&self) -> &GraphicsPipeline {
        &self.pipeline
    }

    /// A cloneable root-blob template; draws fill the `root` pointer and push
    /// the blob before each draw.
    pub fn root(&self) -> &RootBinder {
        &self.root
    }
}

/// The color format of offscreen view targets.
pub const VIEW_TARGET_FORMAT: Format = Format::B8G8R8A8Unorm;

/// Physical sizes requested for logical render targets, written by consumers
/// (the editor writes the `Viewport` entry from its panel size each frame).
#[derive(Default)]
pub struct RenderTargetSizes(pub HashMap<RenderTarget, (u32, u32)>);

/// `Render` system: ensure every view's target has an offscreen attachment
/// of the requested size. Runs before [`main_opaque_pass_3d`].
pub fn prepare_view_targets(world: &mut World) {
    let requested: Vec<RenderTarget> = {
        let Some(frame) = world.get_resource::<Core3dFrame>() else {
            return;
        };
        frame
            .views()
            .iter()
            .map(|view| view.target.0)
            // Only offscreen targets are attachments here; window-targeted
            // views resolve against the swapchain, which is not drawn into yet.
            .filter(|target| matches!(target, RenderTarget::Viewport))
            .collect()
    };
    if requested.is_empty() {
        return;
    }
    let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone()) else {
        return;
    };
    let sizes = world
        .get_resource::<RenderTargetSizes>()
        .map(|sizes| sizes.0.clone())
        .unwrap_or_default();
    if !world.contains_resource::<ViewTargets>() {
        world.insert_resource(ViewTargets::default());
    }
    let mut targets = world
        .get_resource_mut::<ViewTargets>()
        .expect("ViewTargets was just ensured");
    for target in requested {
        let (width, height) = sizes
            .get(&target)
            .copied()
            .unwrap_or((INITIAL_WIDTH, INITIAL_HEIGHT));
        targets.ensure(target, width, height, VIEW_TARGET_FORMAT, &render_device);
    }
}

/// Record one view's opaque pass into `command_buffer`: clear color and
/// depth, then dispatch every queued [`Opaque3d`] item to its registered
/// draw function. `view: None` clears the target to a dim background color
/// (no camera case).
pub fn record_view_pass(
    world: &World,
    view: Option<&Core3dView>,
    target: &OffscreenTarget,
    draw_functions: &DrawFunctions<Opaque3d>,
    command_buffer: &CommandBuffer,
) {
    let phase = view.map(|view| &view.opaque);
    let (width, height) = target.extent();
    let clear_color = match view {
        Some(view) => view.view.camera.clear_color,
        None => [0.05, 0.0, 0.08, 1.0],
    };

    // Debug seam: MOONFIELD_DEBUG_SCENE=1 logs the scene contents once.
    if std::env::var_os("MOONFIELD_DEBUG_SCENE").is_some() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let camera_pos = view.map(|view| {
                let t = view.view.world_from_view.affine().translation;
                (t.x, t.y, t.z)
            });
            info!(
                "scene: camera={camera_pos:?} items={} extent=({width}, {height})",
                phase.map_or(0, |phase| phase.items().len()),
            );
            if let Some(phase) = phase {
                for item in phase.items() {
                    info!("  item mvp: {:?}", item.mvp.to_cols_array());
                }
            }
        });
    }

    let color_attachment = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color(clear_color),
    };
    // Reverse-Z: the depth clear value is 0.0 (near → 1).
    let depth_attachment = target.depth_view().map(|view| RenderAttachment {
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
    if let Some(phase) = phase {
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
        for item in phase.items() {
            let Some(draw) = draw_functions.get(item.draw_function()) else {
                continue;
            };
            draw.draw(world, item, command_buffer);
        }
    }
    command_buffer.end_rendering();
}

/// `Render` system: record the opaque pass of every view whose target has an
/// offscreen attachment, into the window frame's command buffer. No-ops when
/// no window frame is in progress (headless runs, minimized windows).
///
/// Ordering: registered `.after(acquire_window_frames)` and
/// `.before(submit_window_frames)` by [`RenderFeaturePlugin`].
pub fn main_opaque_pass_3d(world: &mut World) {
    if !world.contains_resource::<FrameDrawArena>()
        && let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone())
    {
        match FrameDrawArena::new(render_device.device()) {
            Ok(arena) => world.insert_resource(arena),
            Err(e) => error!("failed to create frame draw arena: {e}"),
        }
    }
    if !world.contains_resource::<Core3dPipeline>() {
        let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone()) else {
            return;
        };
        match Core3dPipeline::new(&render_device) {
            Ok(pipeline) => world.insert_resource(pipeline),
            Err(e) => {
                error!("failed to create core 3d pipeline: {e}");
                return;
            }
        }
    }

    let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
        return;
    };
    // The frame slot must be captured before the command-buffer borrow below:
    // `CommandBuffer` keeps `surfaces` mutably borrowed until recording ends.
    let frame_slot = surfaces.values_mut().next().map(|data| data.frame_index());
    let Some(command_buffer) = surfaces
        .values_mut()
        .find_map(|data| data.current_command_buffer())
    else {
        return;
    };
    let command_buffer: &CommandBuffer = command_buffer;

    let Some(frame) = world.get_resource::<Core3dFrame>().map(|f| (*f).clone()) else {
        return;
    };
    let Some(draw_functions) = world.get_resource::<DrawFunctions<Opaque3d>>() else {
        return;
    };
    let targets = world.get_resource::<ViewTargets>();
    let Some(targets) = targets.as_deref() else {
        return;
    };

    if let Some(slot) = frame_slot
        && let Some(arena) = world.get_resource::<FrameDrawArena>()
    {
        arena.begin_frame(slot);
    }

    // Record every offscreen target: views draw into their target; targets
    // with no view this frame are still cleared (background color).
    for (target_key, target) in targets.iter() {
        let view = frame
            .views()
            .iter()
            .find(|view| view.is_primary && view.target.0 == *target_key);
        record_view_pass(&*world, view, target, &draw_functions, command_buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Mesh, MeshHandle, MeshRenderer};
    use moonfield_app::App;
    use moonfield_asset::Assets;
    use moonfield_camera::{Camera, PrimaryCamera};
    use moonfield_math::{GlobalTransform, Transform, Vec3};
    use moonfield_render_core::ViewTarget;
    use moonfield_rhi::{CommandBufferUsage, CommandPool};

    const TEST_QUAD_VERTICES: &[[f32; 3]] = &[
        [-0.5, -0.5, 0.0],
        [0.5, -0.5, 0.0],
        [0.5, 0.5, 0.0],
        [-0.5, 0.5, 0.0],
    ];
    const TEST_QUAD_INDICES: &[u32] = &[0, 3, 2, 2, 1, 0];

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
        app.add_plugin(crate::RenderFeaturePlugin);
        app.render_world_mut()
            .insert_resource(render_device.clone());
        // The draw function reads the pipeline from the render world; insert
        // after `RenderDevice` so LIFO teardown destroys it first.
        app.render_world_mut()
            .insert_resource(Core3dPipeline::new(render_device).expect("pipeline"));
        // The draw function allocates per-draw root data from this arena; the
        // test drives slot 0 manually (no window frame loop in headless mode).
        app.render_world_mut()
            .insert_resource(FrameDrawArena::new(render_device.device()).expect("arena"));
        app.add_extract_system(moonfield_render_core::extract_cameras);
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
        let target = OffscreenTarget::new_with_depth(
            device,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            VIEW_TARGET_FORMAT,
        )
        .expect("target");

        let world = app.render_world();
        let frame = world.get_resource::<Core3dFrame>().expect("Core3dFrame");
        let view = frame.primary_view(ViewTarget(RenderTarget::Viewport));
        let draw_functions = world
            .get_resource::<DrawFunctions<Opaque3d>>()
            .expect("DrawFunctions<Opaque3d>");

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
        record_view_pass(world, view, &target, &draw_functions, &command_buffer);

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
