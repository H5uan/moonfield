//! Pipeline teardown rides the retirement ring.
//!
//! Command buffers reference pipelines through binds, so a pipeline dropped
//! mid-frame-loop (the render layer rebuilds pipelines on shader revision)
//! must defer destruction past the in-flight frames. These tests exercise
//! the defer path end to end: create, drop, drain.

use super::common;

use crate::{Compiler, ComputePipeline, Device, Instance, ShaderModule};

const NOOP_KERNEL: &str = r#"
[shader("compute")]
[numthreads(1, 1, 1)]
void main()
{
}
"#;

/// Create a headless instance + device, skipping on machines without one
/// (mirrors `depth_buffer.rs`).
fn setup() -> Option<(Instance, Device)> {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return None;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return None;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return None;
        }
    };
    Some((instance, device))
}

#[test]
fn dropped_pipelines_retire_through_the_ring() {
    let Some((_instance, device)) = setup() else {
        return;
    };

    let compiler = Compiler::new().expect("compiler creation");
    let spirv = compiler
        .compile_source_to_spirv("noop", NOOP_KERNEL, "main")
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");

    // A dropped pipeline defers its destruction: nothing is in flight, so
    // the explicit drain destroys it, and the device stays usable after.
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");
    drop(pipeline);
    device.flush_retirements();

    // A pipeline still queued at teardown drains with the device idle.
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");
    drop(pipeline);
}
