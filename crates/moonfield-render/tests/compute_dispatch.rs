//! Headless compute smoke test for Lunar Mare Vulkan RHI.
//!
//! Verifies the compute pipeline path end-to-end: dispatch a compute shader
//! that writes a known pattern into a storage buffer, copy it back to a
//! host-visible buffer, and assert the contents. This is the Phase 0
//! verification gate for GPU-driven physics
//! (see `~/.claude/plans/physics-engine-foundation.md`).
//!
//! Skips gracefully on machines without a Vulkan driver (Windows/macOS CI
//! without lavapipe), mirroring `headless_triangle.rs`.

use ash::vk;
use moonfield_render::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    Buffer, BufferUsage, CommandPool, Compiler, ComputePipeline, Device, Instance,
    PipelineLayout, ShaderModule, ShaderStage,
};

/// Uniform params pushed to the compute shader (cross-backend: wgpu has no
/// push constants, so per-dispatch params travel via a uniform buffer).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    /// Number of elements to write.
    count: u32,
    /// Base value added to the element index.
    base: u32,
}

#[test]
fn compute_dispatch_writes_storage_buffer() {
    // CI runners without a GPU/Vulkan driver (Windows, macOS) skip this test;
    // Linux CI runs it against lavapipe (Mesa software Vulkan).
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let compiler = Compiler::new().expect("compiler creation");

    // A compute shader that writes `base + index` into a storage buffer.
    // RWStructuredBuffer is the Slang term for a read/write storage buffer;
    // ConstantBuffer<T> maps to a uniform buffer binding.
    let compute_source = r#"
struct Params
{
    uint count;
    uint base;
};

[shader("compute")]
[numthreads(64, 1, 1)]
void main(
    uint3 dtid : SV_DispatchThreadID,
    uniform RWStructuredBuffer<uint> output,
    uniform ConstantBuffer<Params> params)
{
    if (dtid.x >= params.count)
        return;
    output[dtid.x] = params.base + dtid.x;
}
"#;

    let compute_spirv = compiler
        .compile_source_to_spirv("compute_fill", compute_source, "main")
        .expect("compute shader compilation");
    let compute_shader =
        ShaderModule::from_spirv(&device, &compute_spirv).expect("compute shader module");

    // Bind group layout: set 0 = { binding 0: storage buffer (uint),
    // binding 1: uniform buffer (Params) }. Both compute-visible.
    let bind_group_layout = BindGroupLayout::new(
        &device,
        &[
            BindGroupLayoutEntry {
                binding: 0,
                ty: BindingType::StorageBuffer,
                visibility: ShaderStage::Compute,
            },
            BindGroupLayoutEntry {
                binding: 1,
                ty: BindingType::UniformBuffer,
                visibility: ShaderStage::Compute,
            },
        ],
    )
    .expect("bind group layout");

    let pipeline_layout =
        PipelineLayout::new(&device, &[&bind_group_layout]).expect("pipeline layout");
    let pipeline =
        ComputePipeline::new(&device, &pipeline_layout, &compute_shader).expect("compute pipeline");

    const ELEMENT_COUNT: u32 = 256;
    const BASE: u32 = 1000;

    // Device-local output buffer; also COPY_SRC so we can read it back.
    let output_buffer = Buffer::new(
        &device,
        (ELEMENT_COUNT as u64) * (std::mem::size_of::<u32>() as u64),
        BufferUsage::STORAGE | BufferUsage::COPY_SRC,
        gpu_allocator::MemoryLocation::GpuOnly,
    )
    .expect("output buffer");

    let params = Params {
        count: ELEMENT_COUNT,
        base: BASE,
    };
    let params_buffer = Buffer::new(
        &device,
        std::mem::size_of::<Params>() as u64,
        BufferUsage::UNIFORM,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("params buffer");
    params_buffer
        .upload(&device, std::slice::from_ref(&params))
        .expect("params upload");

    let bind_group = BindGroup::new(
        &device,
        &bind_group_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer {
                    buffer: &output_buffer,
                    offset: 0,
                    size: (ELEMENT_COUNT as u64) * (std::mem::size_of::<u32>() as u64),
                },
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer {
                    buffer: &params_buffer,
                    offset: 0,
                    size: std::mem::size_of::<Params>() as u64,
                },
            },
        ],
    )
    .expect("bind group");

    // Host-visible readback buffer (COPY_DST).
    let readback_buffer = Buffer::new(
        &device,
        (ELEMENT_COUNT as u64) * (std::mem::size_of::<u32>() as u64),
        BufferUsage::COPY_DST,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("readback buffer");

    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(&device, queue_family_index).expect("command pool");
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .expect("command buffer");

    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin command buffer");
    command_buffer.bind_compute_pipeline(pipeline.raw());
    command_buffer.bind_descriptor_sets_compute(
        pipeline_layout.raw(),
        0,
        &[bind_group.raw_vk()],
        &[],
    );
    // ceil(256 / 64) = 4 workgroups in X.
    command_buffer.dispatch(ELEMENT_COUNT.div_ceil(64), 1, 1);

    // Barrier: compute write must finish before the copy reads it.
    command_buffer.pipeline_barrier(
        vk::PipelineStageFlags::COMPUTE_SHADER,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        &[],
        &[vk::BufferMemoryBarrier::default()
            .buffer(output_buffer.raw())
            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .offset(0)
            .size(vk::WHOLE_SIZE)],
        &[],
    );

    // Copy device-local output → host-visible readback.
    unsafe {
        device.raw().cmd_copy_buffer(
            command_buffer.raw(),
            output_buffer.raw(),
            readback_buffer.raw(),
            &[vk::BufferCopy::default()
                .src_offset(0)
                .dst_offset(0)
                .size((ELEMENT_COUNT as u64) * (std::mem::size_of::<u32>() as u64))],
        );
    }
    command_buffer.end().expect("end command buffer");

    let command_buffers = [command_buffer.raw()];
    let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
    unsafe {
        device
            .raw()
            .queue_submit(
                device.graphics_queue(),
                std::slice::from_ref(&submit_info),
                vk::Fence::null(),
            )
            .expect("queue submit");
        device
            .raw()
            .queue_wait_idle(device.graphics_queue())
            .expect("queue wait idle");
    }

    // Read the host-visible readback buffer back to the CPU and assert.
    let bytes = (ELEMENT_COUNT as u64) * (std::mem::size_of::<u32>() as u64);
    let readback = readback_buffer
        .read(&device, bytes)
        .expect("readback read");
    let readback_slice: &[u32] = bytemuck::cast_slice(&readback);
    for (i, &value) in readback_slice.iter().enumerate() {
        assert_eq!(
            value,
            BASE + i as u32,
            "element {i}: expected {}, got {}",
            BASE + i as u32,
            value
        );
    }
}
