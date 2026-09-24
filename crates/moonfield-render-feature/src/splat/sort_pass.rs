//! The per-view splat sort pass — the splat chain's first system on the
//! render schedule.
//!
//! The pass-machinery acceptance slice: a per-view system in the `Core3d`
//! schedule, ordered after the opaque pass, records GPU work into the
//! frame command buffer — a new file plus one registration call, with no
//! edits to render-feature core. The sort machinery is
//! [`crate::gpu_util::RadixSort`], built from the prepared
//! `radix_sort.slang` asset like the graphics pipelines build from theirs
//! (whoever wires the app registers the request through
//! [`crate::shader::PipelineShaders`]); the pairs are synthetic until splat
//! extraction produces real ones.

#[cfg(test)]
use std::sync::{Arc, Mutex};

use moonfield_app::prelude::{App, IntoSystemConfigs, Render, World};
use moonfield_asset::{AssetRevision, Handle};
#[cfg(test)]
use moonfield_asset::{AssetServer, Assets};
use moonfield_log::error_once;
use moonfield_render_core::RenderContext;
use moonfield_render_core::schedule as render_sets;
use moonfield_rhi::{GpuAllocation, Memory, RenderDevice};
use moonfield_shader::Shader;
#[cfg(test)]
use moonfield_shader::SlangLoader;

use crate::core_3d::Core3d;
use crate::core_3d::pass::opaque_pass_3d;
use crate::gpu_util::RadixSort;
use crate::shader::{PipelineShader, PipelineShaders, PreparedShaders, ShaderEntry};

/// The name keying the splat sort pass's shader in [`PipelineShaders`] and
/// [`PreparedShaders`].
pub const SPLAT_SORT_SHADER: &str = "splat_sort";

/// The splat sort pass's shader request: `util/radix_sort.slang`, three
/// compute entries, root binding reflected from `histogram` (the prepared
/// reflection covers all three entries). The caller (the app wiring, the
/// same as for the mesh pipeline's shaders) supplies the loaded asset
/// handle.
pub fn splat_sort_shader(shader: Handle<Shader>) -> PipelineShader {
    PipelineShader {
        pipeline: SPLAT_SORT_SHADER,
        shader,
        reflect_entry: "histogram",
        entries: &[
            ShaderEntry {
                name: "histogram",
                capabilities: &[],
            },
            ShaderEntry {
                name: "scan",
                capabilities: &[],
            },
            ShaderEntry {
                name: "scatter",
                capabilities: &[],
            },
        ],
    }
}

/// The view's sort machinery: the radix pipelines and the pair buffers the
/// pass sorts every frame.
pub struct SplatSortPass {
    sort: RadixSort,
    keys_in: GpuAllocation,
    values_in: GpuAllocation,
    keys_out: GpuAllocation,
    values_out: GpuAllocation,
    pairs: u32,
    /// The shader asset the sort was built from.
    shader: Handle<Shader>,
    /// The prepared-shader revision the sort was built from.
    shader_revision: AssetRevision,
}

/// `PrepareViews` set system: build the pass's pipelines and buffers from
/// the prepared `radix_sort.slang` asset, whenever the prepared shader's
/// revision advances past the one the sort was built from (the
/// rebuild-on-advance behavior of `prepare_core_3d_pipeline`).
pub fn prepare_splat_sort(world: &mut World) {
    let Some(request) = world
        .get_resource::<PipelineShaders>()
        .and_then(|requests| requests.get(SPLAT_SORT_SHADER).copied())
    else {
        error_once!("no '{SPLAT_SORT_SHADER}' shader registered; skipping the splat sort pass");
        return;
    };
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|device| (*device).clone())
    else {
        return;
    };

    let pairs = 1024u32;
    // The `Ref` guard is scoped here: the sort and the revision it yields
    // outlive it, so the resource insert below borrows the world cleanly.
    let (sort, shader_revision) = {
        let prepared = world.get_resource::<PreparedShaders>();
        let Some(prepared_shader) = prepared.as_ref().and_then(|p| p.get(SPLAT_SORT_SHADER)) else {
            error_once!("the splat sort shader is not ready; skipping the splat sort pass");
            return;
        };
        let stale = world.get_resource::<SplatSortPass>().is_none_or(|pass| {
            pass.shader != request.shader || pass.shader_revision != prepared_shader.revision()
        });
        if !stale {
            return;
        }
        let Ok(sort) =
            RadixSort::from_prepared(render_device.device(), prepared_shader, pairs as usize)
        else {
            return;
        };
        (sort, prepared_shader.revision())
    };

    let device = render_device.device();
    let bytes = (pairs as usize * std::mem::size_of::<u32>()) as u64;
    let (Ok(keys_in), Ok(values_in), Ok(keys_out), Ok(values_out)) = (
        GpuAllocation::new(device, bytes, Memory::Default),
        GpuAllocation::new(device, bytes, Memory::Default),
        GpuAllocation::new(device, bytes, Memory::Default),
        GpuAllocation::new(device, bytes, Memory::Default),
    ) else {
        return;
    };
    world.insert_resource(SplatSortPass {
        sort,
        keys_in,
        values_in,
        keys_out,
        values_out,
        pairs,
        shader: request.shader,
        shader_revision,
    });
}

/// `Core3d` per-view system, ordered after the opaque pass: record the
/// view's (key, value) sort into the frame command buffer through the
/// [`RenderContext`] compute door. The recording's dispatches carry their
/// own automatic barriers; ordering against the frame's other GPU work is
/// the schedule's.
pub fn sort_splats(world: &mut World) {
    let Some(pass) = world.get_resource::<SplatSortPass>() else {
        return;
    };
    let mut ctx = RenderContext::get(world);
    let Some(mut compute) = ctx.compute() else {
        return;
    };
    pass.sort.record(
        &mut compute,
        &pass.keys_in,
        &pass.values_in,
        &pass.keys_out,
        &pass.values_out,
        pass.pairs,
    );
}

/// The splat sort pass as a plugin — what a feature author writes to add a
/// pass: this type plus the file. The prepare system builds in `PrepareViews`;
/// the sort runs per view in `Core3d`, after the opaque pass. Add it to an
/// app to enable the pass (the acceptance test does; the GS roadmap's M3
/// replaces the synthetic pairs with real ones).
pub struct SplatSortPassPlugin;

impl moonfield_app::Plugin for SplatSortPassPlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::splat::SplatSortPassPlugin"
    }

    fn build(&self, app: &mut App) {
        app.add_render_systems(
            Render,
            prepare_splat_sort.in_set::<render_sets::PrepareViews>(),
        );
        app.add_render_systems(Core3d, sort_splats.after(&opaque_pass_3d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderFeaturePlugin, core_3d::pass::opaque_pass_3d};
    use moonfield_camera::{Camera, PrimaryCamera};
    use moonfield_math::GlobalTransform;

    /// Deterministic xorshift32, the same shape as the radix sort test.
    struct Rng(u32);
    impl Rng {
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// An exclusive system pushing `name` into the shared event log — the
    /// ordering probe.
    fn probe(name: &'static str) -> impl FnMut(&mut World) {
        move |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push(name.to_string());
        }
    }

    /// The pass-machinery acceptance test: the radix-sort dispatch runs
    /// ordered after the opaque pass, recorded into the frame command
    /// buffer by a system that was added as a new file plus one
    /// registration call.
    #[test]
    fn sort_pass_runs_after_the_opaque_pass() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut app = App::new();
        app.insert_resource(events.clone());
        app.render_world_mut().insert_resource(events.clone());
        app.add_plugin(moonfield_render_core::RenderPlugin);
        app.add_plugin(RenderFeaturePlugin);
        // The registration under test: new file plus this call.
        app.add_plugin(SplatSortPassPlugin);

        // The app wiring this pass assumes: load `radix_sort.slang` through
        // the asset server and register the pipeline request, the same shape
        // the editor's `load_pipeline_shaders` gives the mesh pipelines.
        let shader_path = moonfield_asset::assets_dir().join("shaders/util/radix_sort.slang");
        let mut server = AssetServer::default();
        server.register_loader(SlangLoader);
        let handle = {
            let mut assets = app
                .world_mut()
                .get_resource_mut::<Assets<Shader>>()
                .expect("Assets<Shader> registered by RenderFeaturePlugin");
            server
                .load(&mut assets, &shader_path)
                .expect("load radix_sort.slang through the asset server")
        };
        app.world_mut()
            .get_resource_mut::<PipelineShaders>()
            .expect("PipelineShaders registered by RenderFeaturePlugin")
            .push(splat_sort_shader(handle));

        // Ordering probes: "before" precedes the opaque pass; "between"
        // runs after the opaque pass and before the sort. A clean frame
        // carrying both markers proves the sort is ordered after the
        // opaque pass — the between-probe's constraints are
        // unsatisfiable otherwise.
        app.add_render_systems(Core3d, probe("before").before(&opaque_pass_3d));
        app.add_render_systems(
            Core3d,
            probe("between").after(&opaque_pass_3d).before(&sort_splats),
        );

        // One view to run the Core3d schedule against.
        app.world_mut()
            .spawn((Camera::default(), PrimaryCamera, GlobalTransform::IDENTITY));

        // Frame 1 builds the pass (PrepareViews) and records on undefined
        // buffers; the ordering markers land once.
        app.render();
        assert_eq!(events.lock().unwrap().as_slice(), &["before", "between"]);

        // Headless machines (e.g. CI without a Vulkan driver) have no
        // RenderDevice: the ordering probe above already ran, but the
        // recorded-dispatch check below needs a real device.
        if !app.render_world().contains_resource::<RenderDevice>() {
            return;
        }

        // Fill the pair buffers with a seeded shuffle (host-visible
        // allocations) and verify the recorded dispatch really sorts:
        // frame 2 records the sort, the frame loop submits it.
        let (keys, values) = {
            let render_device = app
                .render_world()
                .get_resource::<RenderDevice>()
                .expect("render device")
                .clone();
            let pass = app
                .render_world()
                .get_resource::<SplatSortPass>()
                .expect("sort pass built during frame 1");
            let mut rng = Rng(0x1234_5678);
            let keys: Vec<u32> = (0..pass.pairs).map(|_| rng.next_u32()).collect();
            let values: Vec<u32> = (0..pass.pairs).collect();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    keys.as_ptr(),
                    pass.keys_in.host().unwrap().typed::<u32>(),
                    keys.len(),
                );
                std::ptr::copy_nonoverlapping(
                    values.as_ptr(),
                    pass.values_in.host().unwrap().typed::<u32>(),
                    values.len(),
                );
            }
            drop(pass);
            app.render();
            render_device.device().wait_idle().expect("wait idle");
            (keys, values)
        };

        let pass = app
            .render_world()
            .get_resource::<SplatSortPass>()
            .expect("sort pass");
        let n = pass.pairs as usize;
        let gpu_keys: Vec<u32> = unsafe {
            std::slice::from_raw_parts(pass.keys_out.host().unwrap().typed::<u32>(), n).to_vec()
        };
        let gpu_values: Vec<u32> = unsafe {
            std::slice::from_raw_parts(pass.values_out.host().unwrap().typed::<u32>(), n).to_vec()
        };

        // The one deterministic answer a correct stable sort produces.
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by_key(|&i| keys[i as usize]);
        let expected_keys: Vec<u32> = order.iter().map(|&i| keys[i as usize]).collect();
        let expected_values: Vec<u32> = order.iter().map(|&i| values[i as usize]).collect();
        assert_eq!(gpu_keys, expected_keys, "sorted keys");
        assert_eq!(gpu_values, expected_values, "values (order of equal keys)");
    }
}
