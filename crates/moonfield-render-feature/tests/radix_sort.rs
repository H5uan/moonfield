//! Determinism acceptance for the GPU radix sort: seeded shuffled keys
//! must round-trip to the exact stable CPU sort — keys non-decreasing,
//! values the matching permutation, equal keys keeping their input order.
//! Exact equality is possible because the sort is deterministic, so the
//! comparison doubles as the determinism proof.

use std::path::PathBuf;

use moonfield_asset::{AssetServer, Assets};
use moonfield_render_feature::gpu_util::RadixSort;
use moonfield_rhi::{CommandBufferUsage, CommandPool, Device, GpuAllocation, Instance, Memory};
use moonfield_shader::{Shader, SlangLoader};

/// Deterministic xorshift32, the same shape as the ml tests.
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

/// Write `values` through a host-visible allocation's persistent mapping.
fn write_u32s(alloc: &GpuAllocation, values: &[u32]) {
    // SAFETY: the allocation is host-visible, persistently mapped, and sized
    // `values.len()` u32s.
    unsafe {
        std::ptr::copy_nonoverlapping(
            values.as_ptr(),
            alloc
                .host()
                .expect("allocation must have a host view")
                .typed::<u32>(),
            values.len(),
        );
    }
}

/// Read `len` u32s back from a host-visible allocation.
fn read_u32s(alloc: &GpuAllocation, len: usize) -> Vec<u32> {
    // SAFETY: host-visible and sized `len` u32s; read after
    // `submit_and_wait`, so the GPU is done writing.
    unsafe {
        std::slice::from_raw_parts(
            alloc
                .host()
                .expect("allocation must have a host view")
                .typed::<u32>(),
            len,
        )
        .to_vec()
    }
}

/// Sort one case on the GPU and compare against Rust's stable sort.
fn run_case(device: &Device, sort: &RadixSort, pool: &CommandPool, name: &str, keys: &[u32]) {
    let n = keys.len();
    let values: Vec<u32> = (0..n as u32).collect();
    let bytes = size_of_val(keys) as u64;
    let keys_in = GpuAllocation::new(device, bytes, Memory::Default).expect("keys_in");
    let values_in = GpuAllocation::new(device, bytes, Memory::Default).expect("values_in");
    let keys_out = GpuAllocation::new(device, bytes, Memory::Default).expect("keys_out");
    let values_out = GpuAllocation::new(device, bytes, Memory::Default).expect("values_out");
    write_u32s(&keys_in, keys);
    write_u32s(&values_in, &values);

    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    sort.record(&cmd, &keys_in, &values_in, &keys_out, &values_out, n as u32);
    cmd.end().expect("end");
    device.submit_and_wait(&[&cmd]).expect("submit");

    let gpu_keys = read_u32s(&keys_out, n);
    let gpu_values = read_u32s(&values_out, n);

    // Rust's sort_by_key is stable, so this is the one deterministic
    // answer a correct stable sort can produce.
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_by_key(|&i| keys[i as usize]);
    let expected_keys: Vec<u32> = order.iter().map(|&i| keys[i as usize]).collect();
    let expected_values: Vec<u32> = order.iter().map(|&i| values[i as usize]).collect();
    assert_eq!(gpu_keys, expected_keys, "case {name}: sorted keys");
    assert_eq!(
        gpu_values, expected_values,
        "case {name}: values (order of equal keys)"
    );
}

#[test]
fn radix_sort_matches_stable_cpu_sort() {
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

    let mut server = AssetServer::default();
    server.register_loader(SlangLoader);
    let mut assets = Assets::<Shader>::default();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/shaders/util/radix_sort.slang");
    let handle = server
        .load(&mut assets, &path)
        .expect("load radix_sort.slang through the asset server");
    let shader = assets.get(&handle).expect("shader asset");

    let sort = RadixSort::new(&device, shader, 4096).expect("radix sort");
    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");

    // Full-range keys, whole groups.
    let mut rng = Rng(0x1234_5678);
    let shuffled: Vec<u32> = (0..4096).map(|_| rng.next_u32()).collect();
    run_case(&device, &sort, &pool, "shuffled-4096", &shuffled);

    // A partial final group.
    let partial: Vec<u32> = (0..257).map(|_| rng.next_u32()).collect();
    run_case(&device, &sort, &pool, "partial-group-257", &partial);

    // A single element.
    run_case(&device, &sort, &pool, "single-1", &[0x89AB_CDEF]);

    // Heavy duplicates — the stability battlefield.
    let duplicates: Vec<u32> = (0..1024).map(|_| rng.next_u32() % 16).collect();
    run_case(&device, &sort, &pool, "duplicates-1024", &duplicates);
}
