//! The opaque 3D phase: mesh draw items, their draw function, and queueing.
//!
//! [`Opaque3d`] items are pure data: queueing computes the camera-space depth
//! and the final view-projection × model matrix, and [`DrawMesh`] — registered
//! in the phase's [`DrawFunctions`] registry — records each item. The core 3D
//! pass only dispatches items to their registered draw functions; it never
//! names mesh types.

use std::sync::Mutex;

use moonfield_app::prelude::World;
use moonfield_asset::AssetId;
use moonfield_camera::view_matrix;
use moonfield_math::{GlobalTransform, Mat4, Vec3A};
use moonfield_render_core::{DrawFunction, DrawFunctionId, MainEntity, OrderedFloat, PhaseItem};
use moonfield_rhi::{BumpAlloc, CommandBuffer, GpuBumpAllocator, IndexFormat};

use crate::core_3d::Core3dFrame;
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
    /// Registered draw function that records this item.
    pub draw_function: DrawFunctionId,
}

impl PhaseItem for Opaque3d {
    type SortKey = OrderedFloat;

    fn sort_key(&self) -> Self::SortKey {
        OrderedFloat(self.distance)
    }

    fn draw_function(&self) -> DrawFunctionId {
        self.draw_function
    }
}

/// Per-draw push data: object-to-world matrix + flat color. The
/// view-projection lives in the pass's [`ViewUniforms`] record.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawData {
    model: [f32; 16],
    color: [f32; 4],
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

/// The opaque phase's registered draw function. A marker type with no state.
pub struct DrawMesh;

impl DrawFunction<Opaque3d> for DrawMesh {
    fn draw(&self, world: &World, item: &Opaque3d, command_buffer: &CommandBuffer) {
        let Some(extracted_meshes) = world.get_resource::<ExtractedMeshes>() else {
            return;
        };
        let Some(prepared_meshes) = world.get_resource::<PreparedGpuMeshes>() else {
            return;
        };
        let Some(pipeline) = world.get_resource::<Core3dPipeline>() else {
            return;
        };
        let Some(revision) = extracted_meshes.get(item.mesh).map(|mesh| mesh.revision) else {
            return;
        };
        let Some(gpu) = prepared_meshes.get_for_revision(item.mesh, revision) else {
            return;
        };
        let Some(draw_arena) = world.get_resource::<FrameDrawArena>() else {
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
            };
        }

        let graphics_pipeline = pipeline.pipeline();
        command_buffer.bind_graphics_pipeline(graphics_pipeline);
        command_buffer.bind_vertex_buffers(0, &[gpu.vertex()], &[0]);
        command_buffer.bind_index_buffer(gpu.index(), 0, IndexFormat::Uint32);

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
        command_buffer.push_data(pipeline.root().offset as u32, &root_bytes);
        command_buffer.draw_indexed(gpu.index_count(), 1, 0, 0, 0);
    }
}

/// The opaque phase's registered draw-function id, threaded from plugin build
/// to the queue system.
#[derive(Debug, Clone, Copy)]
pub struct Opaque3dDrawFunction(pub DrawFunctionId);

/// `RenderQueue` system: fill every view's opaque [`RenderPhase`] from the
/// extracted mesh entities. Runs after `prepare_core_3d_frame` so the per-view
/// phases exist first.
pub fn queue_opaque_3d(world: &mut World) {
    let Some(meshes) = world.get_resource::<ExtractedMeshes>() else {
        return;
    };
    let Some(opaque) = world.get_resource::<Opaque3dDrawFunction>() else {
        return;
    };

    let drawables: Vec<(MainEntity, AssetId, Vec3A, Mat4, [f32; 4])> = world
        .query::<(&MeshRenderer, &GlobalTransform)>()
        .filter_map(|(entity, (renderer, global))| {
            let main_entity = world.get_component::<MainEntity>(entity).copied()?;
            let mesh = renderer.mesh.0.id();
            if meshes
                .get(mesh)
                .is_none_or(|extracted| extracted.mesh.indices().is_empty())
            {
                return None;
            }
            let affine = global.affine();
            Some((
                main_entity,
                mesh,
                affine.translation,
                Mat4::from(affine),
                renderer.color,
            ))
        })
        .collect();
    if drawables.is_empty() {
        return;
    }

    let Some(mut frame) = world.get_resource_mut::<Core3dFrame>() else {
        return;
    };
    for view in frame.views_mut() {
        let view_from_world = view_matrix(&view.view.world_from_view);
        for (main_entity, mesh, world_position, model, color) in &drawables {
            let distance = -view_from_world.transform_point3((*world_position).into()).z;
            view.opaque.add(Opaque3d {
                main_entity: *main_entity,
                mesh: *mesh,
                model: *model,
                color: *color,
                distance,
                draw_function: opaque.0,
            });
        }
        view.opaque.sort();
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
    use moonfield_render_core::extract_cameras;

    #[test]
    fn test_queue_opaque_3d_skips_missing_meshes_and_sorts_front_to_back() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        app.add_extract_system(extract_cameras);
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
        let frame = app
            .render_world()
            .get_resource::<Core3dFrame>()
            .expect("Core3dFrame");
        let view = frame
            .views()
            .iter()
            .find(|view| view.is_primary)
            .expect("primary");
        let phase = &view.opaque;

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
