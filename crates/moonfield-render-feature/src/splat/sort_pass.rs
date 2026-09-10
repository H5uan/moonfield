//! The per-view splat sort pass — the splat chain's first system on the
//! render schedule.
//!
//! The pass-machinery acceptance slice: a per-view system in the `Core3d`
//! schedule, ordered after the opaque pass, records GPU work into the
//! frame command buffer — a new file plus one registration call, with no
//! edits to render-feature core. The sort machinery is
//! [`crate::gpu_util::RadixSort`]; the pairs are synthetic until splat
//! extraction produces real ones.

#[cfg(test)]
use std::sync::{Arc, Mutex};

use moonfield_app::prelude::{App, IntoSystemConfigs, Render, World};
use moonfield_render_core::RenderContext;
use moonfield_render_core::schedule as render_sets;
use moonfield_rhi::{GpuAllocation, Memory, RenderDevice};
use moonfield_shader::Shader;

use crate::core_3d::Core3d;
use crate::core_3d::pass::opaque_pass_3d;
use crate::gpu_util::RadixSort;

/// The view's sort machinery: the radix pipelines and the pair buffers the
/// pass sorts every frame.
pub struct SplatSortPass {
    sort: RadixSort,
    keys_in: GpuAllocation,
    values_in: GpuAllocation,
    keys_out: GpuAllocation,
    values_out: GpuAllocation,
    pairs: u32,
}

/// `PrepareViews` set system: build the pass's pipelines and buffers once,
/// when a render device exists. The shader source is embedded — the GS
/// integration routes shaders through the asset pipeline like the mesh
/// pipeline's.
pub fn prepare_splat_sort(world: &mut World) {
    if world.contains_resource::<SplatSortPass>() {
        return;
    }
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|device| (*device).clone())
    else {
        return;
    };
    let device = render_device.device();
    let shader = Shader::new(
        include_str!("../../../../assets/shaders/util/radix_sort.slang").to_string(),
        "assets/shaders/util/radix_sort.slang".to_string(),
    );
    let pairs = 1024u32;
    let Ok(sort) = RadixSort::new(device, &shader, pairs as usize) else {
        return;
    };
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

/// Register the sort pass — what a feature author writes to add a pass:
/// this call plus the file. The prepare system builds in `PrepareViews`;
/// the sort runs per view, after the opaque pass.
pub fn register_sort_pass(app: &mut App) {
    app.add_render_systems(
        Render,
        prepare_splat_sort.in_set::<render_sets::PrepareViews>(),
    );
    app.add_render_systems(Core3d, sort_splats.after(&opaque_pass_3d));
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
        register_sort_pass(&mut app);

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
