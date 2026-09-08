//! Shared GPU compute utilities.
//!
//! [`RadixSort`] is the deterministic LSD radix sort over `(u32 key, u32
//! value)` pairs, compiled from the `assets/shaders/util/radix_sort.slang`
//! asset. Splatting sorts Gaussians by view-space depth through it; the
//! f32→u32 order-preserving key mapping happens at the call site, keeping
//! this module key-agnostic.

use moonfield_rhi::{
    BarrierHazard, CommandBuffer, ComputePipeline, Device, GpuAllocation, Memory, Result,
    RootBinder, RootParamPlace, ShaderModule, Stage,
};
use moonfield_shader::Shader;

/// Root-parameter placements for the histogram entry.
struct HistogramPlaces {
    keys: RootParamPlace,
    hist: RootParamPlace,
    count: RootParamPlace,
    groups: RootParamPlace,
    shift: RootParamPlace,
}

/// Root-parameter placements for the scan entry.
struct ScanPlaces {
    hist: RootParamPlace,
    offsets: RootParamPlace,
    total: RootParamPlace,
}

/// Root-parameter placements for the scatter entry.
struct ScatterPlaces {
    keys_in: RootParamPlace,
    values_in: RootParamPlace,
    keys_out: RootParamPlace,
    values_out: RootParamPlace,
    offsets: RootParamPlace,
    count: RootParamPlace,
    groups: RootParamPlace,
    shift: RootParamPlace,
}

/// All three entries' placements.
struct SortPlaces {
    histogram: HistogramPlaces,
    scan: ScanPlaces,
    scatter: ScatterPlaces,
}

/// A deterministic LSD radix sort over `(u32 key, u32 value)` pairs.
///
/// Eight-bit digits, four passes, one ping-pong through internally owned
/// temporaries — the caller's input is read once and the output written
/// once, never used as scratch. The sort is stable and deterministic:
/// scatter ranks derive from scanned offsets and lane order, never from
/// atomic arrival order, so equal keys keep their input order and reruns
/// reproduce the output bit for bit.
pub struct RadixSort {
    histogram: ComputePipeline,
    scan: ComputePipeline,
    scatter: ComputePipeline,
    places: SortPlaces,
    /// One histogram and one offsets slot set per 256-item group.
    hist: GpuAllocation,
    offsets: GpuAllocation,
    /// The ping-pong pair between the caller's input and output.
    tmp_keys: [GpuAllocation; 2],
    tmp_values: [GpuAllocation; 2],
}

impl RadixSort {
    /// Builds the three pipelines from the `radix_sort.slang` asset and
    /// allocates scratch for up to `max_count` items.
    ///
    /// The scan is a single-workgroup implementation; at million-element
    /// scale it is the optimization point (a two-level scan), with this
    /// API unchanged.
    pub fn new(device: &Device, shader: &Shader, max_count: usize) -> Result<Self> {
        let cache = device.shader_cache();
        let build = |entry: &str| -> Result<(ComputePipeline, RootBinder)> {
            let reflection =
                cache.compile_source_reflection(shader.path(), shader.source(), entry)?;
            let binder = RootBinder::new(&reflection, entry)?;
            let compiled = cache.compile_source(shader.path(), shader.source(), entry, &[], &[])?;
            let module = ShaderModule::from_compiled(device, &compiled)?;
            let pipeline = ComputePipeline::new(device, &module)?;
            Ok((pipeline, binder))
        };
        let (histogram, histogram_binder) = build("histogram")?;
        let (scan, scan_binder) = build("scan")?;
        let (scatter, scatter_binder) = build("scatter")?;
        let places = SortPlaces {
            histogram: HistogramPlaces {
                keys: histogram_binder.pointer_param("keys")?,
                hist: histogram_binder.pointer_param("hist")?,
                count: histogram_binder.uniform_param("count")?,
                groups: histogram_binder.uniform_param("groups")?,
                shift: histogram_binder.uniform_param("shift")?,
            },
            scan: ScanPlaces {
                hist: scan_binder.pointer_param("hist")?,
                offsets: scan_binder.pointer_param("offsets")?,
                total: scan_binder.uniform_param("total")?,
            },
            scatter: ScatterPlaces {
                keys_in: scatter_binder.pointer_param("keys_in")?,
                values_in: scatter_binder.pointer_param("values_in")?,
                keys_out: scatter_binder.pointer_param("keys_out")?,
                values_out: scatter_binder.pointer_param("values_out")?,
                offsets: scatter_binder.pointer_param("offsets")?,
                count: scatter_binder.uniform_param("count")?,
                groups: scatter_binder.uniform_param("groups")?,
                shift: scatter_binder.uniform_param("shift")?,
            },
        };

        let max_groups = max_count.div_ceil(256);
        let hist_bytes = (max_groups * 256 * size_of::<u32>()) as u64;
        let item_bytes = (max_count * size_of::<u32>()) as u64;
        Ok(Self {
            histogram,
            scan,
            scatter,
            places,
            hist: GpuAllocation::new(device, hist_bytes, Memory::Default)?,
            offsets: GpuAllocation::new(device, hist_bytes, Memory::Default)?,
            tmp_keys: [
                GpuAllocation::new(device, item_bytes, Memory::Default)?,
                GpuAllocation::new(device, item_bytes, Memory::Default)?,
            ],
            tmp_values: [
                GpuAllocation::new(device, item_bytes, Memory::Default)?,
                GpuAllocation::new(device, item_bytes, Memory::Default)?,
            ],
        })
    }

    /// Records the full four-pass sort of `count` pairs: `keys_in` /
    /// `values_in` are read once, `keys_out` / `values_out` receive the
    /// sorted pairs.
    ///
    /// `count` must not exceed the `max_count` the sort was built for.
    /// Nothing is recorded for `count == 0`. The dispatches are separated
    /// by memory barriers; no trailing barrier is recorded — ordering
    /// against the caller's other work is the caller's.
    pub fn record(
        &self,
        cmd: &CommandBuffer,
        keys_in: &GpuAllocation,
        values_in: &GpuAllocation,
        keys_out: &GpuAllocation,
        values_out: &GpuAllocation,
        count: u32,
    ) {
        if count == 0 {
            return;
        }
        let groups = count.div_ceil(256);
        for pass in 0..4u32 {
            let shift = pass * 8;
            let (src_keys, src_values, dst_keys, dst_values) = match pass {
                0 => (keys_in, values_in, &self.tmp_keys[0], &self.tmp_values[0]),
                1 => (
                    &self.tmp_keys[0],
                    &self.tmp_values[0],
                    &self.tmp_keys[1],
                    &self.tmp_values[1],
                ),
                2 => (
                    &self.tmp_keys[1],
                    &self.tmp_values[1],
                    &self.tmp_keys[0],
                    &self.tmp_values[0],
                ),
                _ => (&self.tmp_keys[0], &self.tmp_values[0], keys_out, values_out),
            };

            cmd.bind_compute_pipeline(&self.histogram);
            push_ptr(cmd, &self.places.histogram.keys, src_keys);
            push_ptr(cmd, &self.places.histogram.hist, &self.hist);
            push_u32(cmd, &self.places.histogram.count, count);
            push_u32(cmd, &self.places.histogram.groups, groups);
            push_u32(cmd, &self.places.histogram.shift, shift);
            cmd.dispatch(groups, 1, 1);
            cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);

            cmd.bind_compute_pipeline(&self.scan);
            push_ptr(cmd, &self.places.scan.hist, &self.hist);
            push_ptr(cmd, &self.places.scan.offsets, &self.offsets);
            push_u32(cmd, &self.places.scan.total, groups * 256);
            cmd.dispatch(1, 1, 1);
            cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);

            cmd.bind_compute_pipeline(&self.scatter);
            push_ptr(cmd, &self.places.scatter.keys_in, src_keys);
            push_ptr(cmd, &self.places.scatter.values_in, src_values);
            push_ptr(cmd, &self.places.scatter.keys_out, dst_keys);
            push_ptr(cmd, &self.places.scatter.values_out, dst_values);
            push_ptr(cmd, &self.places.scatter.offsets, &self.offsets);
            push_u32(cmd, &self.places.scatter.count, count);
            push_u32(cmd, &self.places.scatter.groups, groups);
            push_u32(cmd, &self.places.scatter.shift, shift);
            cmd.dispatch(groups, 1, 1);

            if pass < 3 {
                cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
            }
        }
    }
}

/// Push one pointer root parameter at its reflected place.
fn push_ptr(cmd: &CommandBuffer, place: &RootParamPlace, alloc: &GpuAllocation) {
    let bytes = place
        .pointer_bytes(alloc.gpu().as_raw())
        .expect("radix sort root pointer placement");
    cmd.push_data(place.offset as u32, &bytes);
}

/// Push one u32 uniform root parameter at its reflected place.
fn push_u32(cmd: &CommandBuffer, place: &RootParamPlace, value: u32) {
    cmd.push_data(place.offset as u32, &value.to_le_bytes());
}
