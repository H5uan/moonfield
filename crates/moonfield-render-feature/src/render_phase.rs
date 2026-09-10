//! The opaque 3D phase: mesh draw items, their draw function, and queueing.
//!
//! [`Opaque3d`] items are pure data: queueing computes the camera-space depth
//! and the final view-projection × model matrix, and [`DrawMesh`] — registered
//! in the phase's [`DrawFunctions`] registry — records each item. The core 3D
//! pass only dispatches items to their registered draw functions; it never
//! names mesh types.

use std::sync::Mutex;

use moonfield_app::prelude::{Query, Res, World};
use moonfield_asset::AssetId;
use moonfield_camera::view_matrix;
use moonfield_math::{GlobalTransform, Mat4, Vec3A};
use moonfield_render_core::{
    DrawFunctionId, DrawFunctions, ExtractedView, MainEntity, OrderedFloat, PhaseItem,
    RenderCommand, RenderPhase, TrackedRenderPass,
};
use moonfield_rhi::{BumpAlloc, GpuBumpAllocator};

use crate::core_3d::pass::Core3dPipeline;
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

/// The mesh pipeline's root pointer: one `GpuPtr` per draw, pushed as a
/// single 8-byte value via `push_data`. The `DrawData` payload itself lives
/// in the frame draw arena.
pub(crate) const DRAW_ARENA_BLOCK: u64 = 1024 * 1024;

pub struct FrameDrawArena {
    inner: std::sync::Mutex<ArenaInner>,
}

struct ArenaInner {
    arenas: Vec<GpuBumpAllocator>, // RING = MAX_FRAMES_IN_FLIGHT(2)
    current: usize,
}

impl FrameDrawArena {
    pub fn new(device: &moonfield_rhi::Device) -> moonfield_rhi::Result<Self> {
        let mut arenas = Vec::with_capacity(moonfield_render_core::MAX_FRAMES_IN_FLIGHT);
        for _ in 0..moonfield_render_core::MAX_FRAMES_IN_FLIGHT {
            arenas.push(GpuBumpAllocator::new(device, DRAW_ARENA_BLOCK)?);
        }
        Ok(Self {
            inner: Mutex::new(ArenaInner { arenas, current: 0 }),
        })
    }

    pub fn begin_frame(&self, slot: usize) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.arenas[slot].free_all();
        g.current = slot;
    }

    /// Allocate the pass's view-uniform record.
    pub fn alloc_view_uniforms(&self) -> moonfield_rhi::Result<BumpAlloc> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = g.current;
        g.arenas[slot].alloc_typed::<ViewUniforms>(1)
    }

    pub fn alloc_draw_data(&self) -> moonfield_rhi::Result<BumpAlloc> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = g.current;
        g.arenas[slot].alloc_typed::<DrawData>(1)
    }
}

/// The opaque phase's registered draw command. A marker type with no state.
pub struct DrawMesh;

/// The command's inputs, fetched once per item instead of re-read per
/// statement: the extracted and prepared meshes, the pipeline, and the
/// frame draw arena.
type DrawMeshParam<'w, 's> = (
    Option<Res<'w, ExtractedMeshes>>,
    Option<Res<'w, PreparedGpuMeshes>>,
    Option<Res<'w, Core3dPipeline>>,
    Option<Res<'w, FrameDrawArena>>,
);

impl RenderCommand<Opaque3d> for DrawMesh {
    type Param = (
        Option<Res<'static, ExtractedMeshes>>,
        Option<Res<'static, PreparedGpuMeshes>>,
        Option<Res<'static, Core3dPipeline>>,
        Option<Res<'static, FrameDrawArena>>,
    );

    fn render(_world: &World, item: &Opaque3d, pass: &mut TrackedRenderPass, param: DrawMeshParam) {
        let (extracted_meshes, prepared_meshes, pipeline, draw_arena) = match param {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => return,
        };
        let Some(revision) = extracted_meshes.get(item.mesh).map(|mesh| mesh.revision) else {
            return;
        };
        let Some(gpu) = prepared_meshes.get_for_revision(item.mesh, revision) else {
            return;
        };
        let root = match draw_arena.alloc_draw_data() {
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
        let view_from_world = view_matrix(&view.world_from_view);
        for (main_entity, mesh, world_position, model, color) in &drawable_data {
            let distance = -view_from_world.transform_point3((*world_position).into()).z;
            phase.add(Opaque3d {
                main_entity: *main_entity,
                mesh: *mesh,
                model: *model,
                color: *color,
                distance,
                draw_function,
            });
        }
    }
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
