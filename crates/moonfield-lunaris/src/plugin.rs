//! Bevy-style plugin for Lunaris.
//!
//! Provides a `LunarisPlugin` that registers the core rendering services and
//! exercises the Vulkan and Slang backends on startup.

use crate::{Compiler, Device, Instance};
use moonfield_app::{App, Plugin};
use moonfield_log::{error, info};
use moonfield_ecs::World;

/// Runtime plugin.
pub struct LunarisPlugin;

impl Plugin for LunarisPlugin {
    fn name(&self) -> &str {
        "Lunaris"
    }

    fn build(&self, app: &mut App) {
        app.add_startup_system(|_world: &mut World| {
            init_vulkan();
            compile_test_shader();
        });
        app.add_shutdown_system(|_world: &mut World| {
            info!("Lunaris shutdown system");
        });
    }
}

fn init_vulkan() {
    match Instance::new_headless() {
        Ok(instance) => match Device::new(&instance, None) {
            Ok(device) => {
                let props = device.physical_device();
                let device_name = unsafe {
                    std::ffi::CStr::from_ptr(
                        instance.physical_device_properties(props).device_name.as_ptr(),
                    )
                    .to_string_lossy()
                };
                info!("Lunaris initialized Vulkan on device: {}", device_name);
            }
            Err(e) => {
                error!("Lunaris could not create Vulkan device: {}", e);
            }
        },
        Err(e) => {
            error!("Lunaris could not create Vulkan instance: {}", e);
        }
    }
}

fn compile_test_shader() {
    match Compiler::new() {
        Ok(compiler) => {
            let source = r#"
struct VsInput { float3 position : POSITION; };
struct VsOutput { float4 position : SV_POSITION; };

[shader("vertex")]
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 1.0);
    return output;
}
"#;
            match compiler.compile_source_to_spirv("triangle", source, "main") {
                Ok(bytecode) => {
                    info!(
                        "Lunaris compiled test Slang shader to {} bytes of SPIR-V",
                        bytecode.len()
                    );
                }
                Err(e) => {
                    error!("Lunaris could not compile test shader: {}", e);
                }
            }
        }
        Err(e) => {
            error!("Lunaris could not create Slang compiler: {}", e);
        }
    }
}
