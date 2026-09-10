//! The opaque 3D phase: mesh draw items, their draw function, and queueing.
//!
//! [`Opaque3d`] items are pure data: queueing computes the camera-space depth
//! and the final view-projection × model matrix, and [`DrawMesh`] — registered
//! in the phase's [`DrawFunctions`] registry — records each item. The core 3D
//! pass only dispatches items to their registered draw functions; it never
//! names mesh types.

use moonfield_app::prelude::{Query, Res, World};
use moonfield_asset::AssetId;
use moonfield_camera::{RenderTarget, view_matrix};
use moonfield_math::{GlobalTransform, Mat4, Vec3A};
use moonfield_render_core::{
    DrawFunctionId, DrawFunctions, ExtractedView, FrameDrawArena, MainEntity, OrderedFloat,
    PhaseItem, RenderCommand, RenderPhase, TrackedRenderPass, WindowSurfaces,
};
use moonfield_rhi::Format;

use crate::core_3d::pass::{Core3dPipelines, VIEW_TARGET_FORMAT};
use crate::mesh::{ExtractedMeshes, MeshRenderer, PreparedGpuMeshes};

/// One opaque mesh draw queued for a view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opaque3d {
    /// Source entity in the main world.
    pub main_entity: MainEntity,
    /// Prepared mesh lookup key.
    pub mesh: AssetId,
    /// Object-to-world, stored at queue time; the view-projection comes
    /// from the pass's [`ViewUniforms`] record.
    pub model: Mat4,
    /// Flat linear RGBA color used by the current mesh pipeline.
    pub color: [f32; 4],
    /// Positive camera-space depth used for front-to-back sorting.
    pub distance: f32,
    /// The color format the item's pipeline variant targets, stamped at
    /// queue time from the view's target ([`DrawMesh`] resolves it against
    /// the format-keyed `Core3dPipelines`).
    pub pipeline: Format,
    /// Registered draw command that records this item.
    pub draw_function: DrawFunctionId<Opaque3d>,
}

impl PhaseItem for Opaque3d {
    type SortKey = OrderedFloat;

    fn sort_key(&self) -> Self::SortKey {
        OrderedFloat(self.distance)
    }

    fn draw_function(&self) -> DrawFunctionId<Opaque3d> {
        self.draw_function
    }
}

/// Per-draw data: the object's transform and color plus its geometry
/// pointers. Layout must match `DrawData` in `core_3d.slang` — the natural
/// (C-like) layout Slang uses behind `Ptr`, verified by
/// `pulling_vertex_shape`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawData {
    model: [f32; 16],
    color: [f32; 4],
    positions: u64,
    indices: u64,
    index_count: u32,
    _pad0: u32,
}

/// Per-view constants, one arena record per pass. Layout must match
/// `ViewUniforms` in `core_3d.slang` — the natural (C-like) layout Slang
/// uses behind `Ptr`, verified by `two_pointer_roots_and_ptr_struct_layout`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniforms {
    pub(crate) view_proj: [f32; 16],
    pub(crate) view_pos: [f32; 3],
    pub(crate) _pad0: f32,
}

/// The opaque phase's registered draw command. A marker type with no state.
pub struct DrawMesh;

/// The command's inputs, fetched once per item instead of re-read per
/// statement: the extracted and prepared meshes, the format-keyed pipelines,
/// and the frame draw arena.
type DrawMeshParam<'w, 's> = (
    Option<Res<'w, ExtractedMeshes>>,
    Option<Res<'w, PreparedGpuMeshes>>,
    Option<Res<'w, Core3dPipelines>>,
    Option<Res<'w, FrameDrawArena>>,
);

impl RenderCommand<Opaque3d> for DrawMesh {
    type Param = (
        Option<Res<'static, ExtractedMeshes>>,
        Option<Res<'static, PreparedGpuMeshes>>,
        Option<Res<'static, Core3dPipelines>>,
        Option<Res<'static, FrameDrawArena>>,
    );

    fn render(_world: &World, item: &Opaque3d, pass: &mut TrackedRenderPass, param: DrawMeshParam) {
        let (extracted_meshes, prepared_meshes, pipelines, draw_arena) = match param {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => return,
        };
        let Some(revision) = extracted_meshes.get(item.mesh).map(|mesh| mesh.revision) else {
            return;
        };
        let Some(gpu) = prepared_meshes.get_for_revision(item.mesh, revision) else {
            return;
        };
        let Some(pipeline) = pipelines.get(item.pipeline) else {
            return;
        };
        let root = match draw_arena.alloc::<DrawData>() {
            Ok(allocation) => allocation,
            Err(e) => {
                moonfield_log::error!("draw arena allocation failed: {e}");
                return;
            }
        };
        unsafe {
            *root.cpu.typed::<DrawData>() = DrawData {
                model: item.model.to_cols_array(),
                color: item.color,
                positions: gpu.positions().as_raw(),
                indices: gpu.indices().as_raw(),
                index_count: gpu.index_count(),
                _pad0: 0,
            };
        }

        // The whole draw state is one arena record behind one pointer: bind
        // the pipeline (deduplicated by the tracked pass), push the record's
        // address, and issue a non-indexed draw whose vertex count is the
        // index count — the vertex shader pulls both arrays through the
        // record's pointers.
        pass.set_graphics_pipeline(pipeline.pipeline());

        // The root is the reflected `Ptr<DrawData>` placement: encode the
        // arena address on the stack and push it at the place's offset —
        // offsets and sizes come from the shader, not a hand-synced struct.
        let root_bytes = match pipeline.root().pointer_bytes(root.gpu.as_raw()) {
            Ok(bytes) => bytes,
            Err(e) => {
                moonfield_log::error!("root encode failed: {e}");
                return;
            }
        };
        pass.push_data(pipeline.root().offset as u32, &root_bytes);
        pass.draw(gpu.index_count(), 1, 0, 0);
    }
}

/// `Queue` system: fill every view's opaque [`RenderPhase`] component from
/// the extracted mesh entities. Runs after `prepare_view_phases` so the
/// per-view phases exist first; sorting is the `PhaseSort` set's.
pub fn queue_opaque_3d(
    meshes: Option<Res<ExtractedMeshes>>,
    draw_functions: Option<Res<DrawFunctions<Opaque3d>>>,
    surfaces: Option<Res<WindowSurfaces>>,
    drawables: Query<(&MeshRenderer, &GlobalTransform, &MainEntity)>,
    mut views: Query<(&ExtractedView, &mut RenderPhase<Opaque3d>)>,
) {
    let (Some(meshes), Some(draw_functions)) = (meshes.as_deref(), draw_functions.as_deref())
    else {
        return;
    };
    let Some(draw_function) = draw_functions.id::<DrawMesh>() else {
        return;
    };

    let drawable_data: Vec<(MainEntity, AssetId, Vec3A, Mat4, [f32; 4])> = drawables
        .iter()
        .filter_map(|(_, (renderer, global, main_entity))| {
            let mesh = renderer.mesh.0.id();
            if meshes
                .get(mesh)
                .is_none_or(|extracted| extracted.mesh.indices().is_empty())
            {
                return None;
            }
            let affine = global.affine();
            Some((
                *main_entity,
                mesh,
                affine.translation,
                Mat4::from(affine),
                renderer.color,
            ))
        })
        .collect();
    if drawable_data.is_empty() {
        return;
    }

    for (_, (view, mut phase)) in views.iter_mut() {
        // The pipeline variant the view's target format needs: the offscreen
        // format for viewport views, the primary surface's format for window
        // views (unresolvable — no window, unreadable format — means the
        // pass will skip the view anyway, so nothing is queued).
        let pipeline = match view.target.0 {
            RenderTarget::Viewport => VIEW_TARGET_FORMAT,
            RenderTarget::PrimaryWindow => {
                let Some(format) = surfaces
                    .as_deref()
                    .and_then(|surfaces| surfaces.primary())
                    .and_then(|data| data.format().ok().map(|(format, _)| format))
                else {
                    continue;
                };
                format
            }
        };
        let view_from_world = view_matrix(&view.world_from_view);
        for (main_entity, mesh, world_position, model, color) in &drawable_data {
            let distance = -view_from_world.transform_point3((*world_position).into()).z;
            phase.add(Opaque3d {
                main_entity: *main_entity,
                mesh: *mesh,
                model: *model,
                color: *color,
                distance,
                pipeline,
                draw_function,
            });
        }
    }
}

/// `Queue` system (debug): with `MOONFIELD_DEBUG_SCENE=1`, log the scene
/// contents once per process — each view's camera position and every queued
/// item's model matrix. Replaces the seam that used to live inside the pass
/// recording body.
pub fn debug_scene_log(views: Query<(&ExtractedView, &RenderPhase<Opaque3d>)>) {
    if std::env::var_os("MOONFIELD_DEBUG_SCENE").is_none() {
        return;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        for (_, (view, phase)) in views.iter() {
            let camera_pos = view.world_from_view.affine().translation;
            moonfield_log::info!(
                "scene: camera=({:.1}, {:.1}, {:.1}) items={}",
                camera_pos.x,
                camera_pos.y,
                camera_pos.z,
                phase.items().len(),
            );
            for item in phase.items() {
                moonfield_log::info!("  item model: {:?}", item.model.to_cols_array());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderFeaturePlugin, mesh::Mesh};
    use moonfield_app::App;
    use moonfield_asset::Assets;
    use moonfield_camera::{Camera, PrimaryCamera};
    use moonfield_math::Transform;

    #[test]
    fn test_queue_opaque_3d_skips_missing_meshes_and_sorts_front_to_back() {
        let mut app = App::new();
        // The real composition: RenderPlugin registers the set chain that
        // orders Queue before PhaseSort and provides extract_cameras
        // (RenderFeaturePlugin alone leaves the set anchors unresolved, so
        // ordering falls to registration).
        app.add_plugin(moonfield_render_core::RenderPlugin);
        app.add_plugin(RenderFeaturePlugin);
        let (near_mesh, far_mesh, removed_mesh) = {
            let mut meshes = app.world().get_resource_mut::<Assets<Mesh>>().unwrap();
            let near = meshes.add(Mesh::new(vec![[0.0; 3]], vec![0], None));
            let far = meshes.add(Mesh::new(vec![[0.0; 3]], vec![0], None));
            let removed = meshes.add(Mesh::new(vec![[0.0; 3]], vec![0], None));
            meshes.remove(&removed);
            (near, far, removed)
        };
        app.world_mut()
            .spawn((Camera::default(), PrimaryCamera, GlobalTransform::IDENTITY));
        for (mesh, z) in [(far_mesh, -8.0), (near_mesh, -2.0), (removed_mesh, -1.0)] {
            app.world_mut().spawn((
                MeshRenderer::new(crate::mesh::MeshHandle(mesh), [1.0; 4]),
                GlobalTransform::from(Transform::from_xyz(0.0, 0.0, z)),
            ));
        }

        app.render();
        let views: Vec<_> = app
            .render_world()
            .query::<(&ExtractedView, &RenderPhase<Opaque3d>)>()
            .collect();
        assert_eq!(views.len(), 1);
        let (_, (_, phase)) = views[0];

        assert_eq!(phase.items().len(), 2);
        assert_eq!(phase.items()[0].mesh, near_mesh.id());
        assert_eq!(phase.items()[1].mesh, far_mesh.id());
        assert!(
            !phase
                .items()
                .iter()
                .any(|item| item.mesh == removed_mesh.id())
        );
    }
}
