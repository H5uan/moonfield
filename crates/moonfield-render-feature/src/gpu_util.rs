//! Shared GPU compute utilities.
//!
//! [`RadixSort`] is the deterministic LSD radix sort over `(u32 key, u32
//! value)` pairs, compiled from the `assets/shaders/util/radix_sort.slang`
//! asset. Splatting sorts Gaussians by view-space depth through it; the
//! f32→u32 order-preserving key mapping happens at the call site, keeping
//! this module key-agnostic.

use moonfield_render_core::ComputeRecording;
use moonfield_rhi::{
    CompiledShader, ComputePipeline, Device, GpuAllocation, Memory, Reflection, Result, RootBinder,
    RootParamPlace, ShaderModule,
};
use moonfield_shader::Shader;

use crate::shader::PreparedShader;

/// Root blob for the histogram entry: mirrors `HistogramParams` in
/// `radix_sort.slang` field-for-field. Pointer fields are GPU addresses;
/// scalar fields are inline u32s.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HistogramParams {
    keys: u64,
    hist: u64,
    count: u32,
    groups: u32,
    shift: u32,
    _pad: u32,
}

/// Root blob for the scan entry: mirrors `ScanParams` in
/// `radix_sort.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScanParams {
    hist: u64,
    offsets: u64,
    total: u32,
    _pad: u32,
}

/// Root blob for the scatter entry: mirrors `ScatterParams` in
/// `radix_sort.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterParams {
    keys_in: u64,
    values_in: u64,
    keys_out: u64,
    values_out: u64,
    offsets: u64,
    count: u32,
    groups: u32,
    shift: u32,
    _pad: u32,
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
    histogram_place: RootParamPlace,
    scan_place: RootParamPlace,
    scatter_place: RootParamPlace,
    /// One histogram and one offsets slot set per 256-item group.
    ///
    /// The temporaries are pure-GPU scratch — read and written by the
    /// passes, never mapped — so they live in device-local memory; only the
    /// caller's in/out buffers (written and read back on the host) stay
    /// host-visible.
    hist: GpuAllocation,
    offsets: GpuAllocation,
    /// The ping-pong pair between the caller's input and output.
    tmp_keys: [GpuAllocation; 2],
    tmp_values: [GpuAllocation; 2],
}

/// The radix sort's three entry points, in the order the pipelines build.
const RADIX_SORT_ENTRIES: [&str; 3] = ["histogram", "scan", "scatter"];

impl RadixSort {
    /// Builds the three pipelines from the `radix_sort.slang` asset and
    /// allocates scratch for up to `max_count` items.
    ///
    /// The scan is a single-workgroup implementation; at million-element
    /// scale it is the optimization point (a two-level scan), with this
    /// API unchanged.
    pub fn new(device: &Device, shader: &Shader, max_count: usize) -> Result<Self> {
        let cache = device.shader_cache();
        // One linked program covers all three entries, so one reflection
        // drives every root binder.
        let reflection =
            cache.compile_source_reflection(shader.path(), shader.source(), &RADIX_SORT_ENTRIES)?;
        let compiled = RADIX_SORT_ENTRIES
            .iter()
            .map(|&entry| cache.compile_source(shader.path(), shader.source(), entry, &[], &[]))
            .collect::<Result<Vec<_>>>()?;
        let artifacts: Vec<&CompiledShader> = compiled.iter().map(|arc| &**arc).collect();
        Self::from_reflection(device, &reflection, &artifacts, max_count)
    }

    /// Builds the three pipelines from a prepared shader — the artifacts
    /// [`crate::shader::PreparedShaders`] compiled from the extracted asset
    /// — so the sort rides the same extract → prepare flow as the graphics
    /// pipelines.
    pub fn from_prepared(
        device: &Device,
        prepared: &PreparedShader,
        max_count: usize,
    ) -> Result<Self> {
        let artifacts = RADIX_SORT_ENTRIES
            .iter()
            .map(|&entry| {
                prepared.entry(entry).ok_or_else(|| {
                    moonfield_rhi::Error::Backend(format!(
                        "prepared radix sort shader is missing '{entry}'"
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::from_reflection(device, prepared.reflection(), &artifacts, max_count)
    }

    /// Shared builder: the pipelines and root placements from one
    /// multi-entry reflection plus the per-entry compiled artifacts.
    fn from_reflection(
        device: &Device,
        reflection: &Reflection,
        artifacts: &[&CompiledShader],
        max_count: usize,
    ) -> Result<Self> {
        let build =
            |entry: &str, artifact: &CompiledShader| -> Result<(ComputePipeline, RootParamPlace)> {
                let binder = RootBinder::new(reflection, entry)?;
                // Each entry takes a single `uniform Params` struct; reflect its
                // one root placement and push it as one POD blob per dispatch.
                let place = binder.uniform_param("params")?;
                let module = ShaderModule::from_compiled(device, artifact)?;
                let pipeline = ComputePipeline::new(device, &module)?;
                Ok((pipeline, place))
            };
        let (histogram, histogram_place) = build("histogram", artifacts[0])?;
        let (scan, scan_place) = build("scan", artifacts[1])?;
        let (scatter, scatter_place) = build("scatter", artifacts[2])?;

        let max_groups = max_count.div_ceil(256);
        let hist_bytes = (max_groups * 256 * size_of::<u32>()) as u64;
        let item_bytes = (max_count * size_of::<u32>()) as u64;
        Ok(Self {
            histogram,
            scan,
            scatter,
            histogram_place,
            scan_place,
            scatter_place,
            hist: GpuAllocation::new(device, hist_bytes, Memory::Gpu)?,
            offsets: GpuAllocation::new(device, hist_bytes, Memory::Gpu)?,
            tmp_keys: [
                GpuAllocation::new(device, item_bytes, Memory::Gpu)?,
                GpuAllocation::new(device, item_bytes, Memory::Gpu)?,
            ],
            tmp_values: [
                GpuAllocation::new(device, item_bytes, Memory::Gpu)?,
                GpuAllocation::new(device, item_bytes, Memory::Gpu)?,
            ],
        })
    }

    /// Records the full four-pass sort of `count` pairs: `keys_in` /
    /// `values_in` are read once, `keys_out` / `values_out` receive the
    /// sorted pairs.
    ///
    /// `count` must not exceed the `max_count` the sort was built for.
    /// Nothing is recorded for `count == 0`. The dispatch chain's read-after-
    /// write hazards are the [`ComputeRecording`]'s automatic
    /// `COMPUTE → COMPUTE` barriers; no trailing barrier is recorded —
    /// ordering against the caller's other work is the caller's.
    pub fn record(
        &self,
        compute: &mut ComputeRecording,
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

            let hist = self.hist.gpu().as_raw();
            let offsets = self.offsets.gpu().as_raw();

            compute.bind_pipeline(&self.histogram);
            let hist_params = HistogramParams {
                keys: src_keys.gpu().as_raw(),
                hist,
                count,
                groups,
                shift,
                _pad: 0,
            };
            compute.push_data(
                self.histogram_place.offset as u32,
                bytemuck::bytes_of(&hist_params),
            );
            compute.dispatch(groups, 1, 1);

            compute.bind_pipeline(&self.scan);
            let scan_params = ScanParams {
                hist,
                offsets,
                total: groups * 256,
                _pad: 0,
            };
            compute.push_data(
                self.scan_place.offset as u32,
                bytemuck::bytes_of(&scan_params),
            );
            compute.dispatch(1, 1, 1);

            compute.bind_pipeline(&self.scatter);
            let scatter_params = ScatterParams {
                keys_in: src_keys.gpu().as_raw(),
                values_in: src_values.gpu().as_raw(),
                keys_out: dst_keys.gpu().as_raw(),
                values_out: dst_values.gpu().as_raw(),
                offsets,
                count,
                groups,
                shift,
                _pad: 0,
            };
            compute.push_data(
                self.scatter_place.offset as u32,
                bytemuck::bytes_of(&scatter_params),
            );
            compute.dispatch(groups, 1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The repository's `radix_sort.slang` source — the same file the sort
    /// pass loads through the asset server.
    fn radix_sort_source() -> String {
        let path = moonfield_asset::assets_dir().join("shaders/util/radix_sort.slang");
        std::fs::read_to_string(path).expect("radix_sort.slang")
    }

    /// The Rust mirror structs must exactly match the root blobs the three
    /// entry points actually push: same total size and same per-field byte
    /// offset, read from Slang's entry-parameter reflection — not from the
    /// natural `struct` layout. `uniform_param("params")` is the same
    /// pipeline-build query `RadixSort::new` uses, so a mismatch here fails
    /// at test time instead of corrupting GPU memory.
    #[test]
    fn root_blobs_match_entry_point_layout() {
        let compiler = moonfield_rhi::Compiler::new().expect("slang compiler");

        struct Case {
            entry: &'static str,
            rust_size: usize,
            fields: &'static [(&'static str, usize)],
        }
        let cases = [
            Case {
                entry: "histogram",
                rust_size: size_of::<HistogramParams>(),
                fields: &[
                    ("keys", offset_of!(HistogramParams, keys)),
                    ("hist", offset_of!(HistogramParams, hist)),
                    ("count", offset_of!(HistogramParams, count)),
                    ("groups", offset_of!(HistogramParams, groups)),
                    ("shift", offset_of!(HistogramParams, shift)),
                ],
            },
            Case {
                entry: "scan",
                rust_size: size_of::<ScanParams>(),
                fields: &[
                    ("hist", offset_of!(ScanParams, hist)),
                    ("offsets", offset_of!(ScanParams, offsets)),
                    ("total", offset_of!(ScanParams, total)),
                ],
            },
            Case {
                entry: "scatter",
                rust_size: size_of::<ScatterParams>(),
                fields: &[
                    ("keys_in", offset_of!(ScatterParams, keys_in)),
                    ("values_in", offset_of!(ScatterParams, values_in)),
                    ("keys_out", offset_of!(ScatterParams, keys_out)),
                    ("values_out", offset_of!(ScatterParams, values_out)),
                    ("offsets", offset_of!(ScatterParams, offsets)),
                    ("count", offset_of!(ScatterParams, count)),
                    ("groups", offset_of!(ScatterParams, groups)),
                    ("shift", offset_of!(ScatterParams, shift)),
                ],
            },
        ];

        for case in &cases {
            let entry = case.entry;
            let rust_size = case.rust_size;
            let fields = case.fields;
            let reflection = compiler
                .compile_source_to_reflection(
                    "radix_sort_blob_layout",
                    &radix_sort_source(),
                    &[entry],
                )
                .expect(entry);
            let binder = RootBinder::new(&reflection, entry).expect("root binder");
            let place = binder.uniform_param("params").expect("params placement");
            assert_eq!(
                rust_size, place.size,
                "{entry}: Rust mirror struct size must equal the reflected root blob size"
            );
            for &(name, offset) in fields {
                // Field offsets come from the shader's struct layout via the
                // same reflection source `RadixSort::new` uses to build the
                // root binder.
                let layout = reflection
                    .struct_layout(match entry {
                        "histogram" => "HistogramParams",
                        "scan" => "ScanParams",
                        _ => "ScatterParams",
                    })
                    .expect("struct layout");
                let shader_offset = layout.field_offset(name).expect("field offset");
                assert_eq!(
                    offset, shader_offset,
                    "{entry}.{name}: Rust mirror offset must match the shader struct offset"
                );
            }
        }
    }

    #[test]
    fn root_params_is_a_single_uniform_blob() {
        let compiler = moonfield_rhi::Compiler::new().expect("slang compiler");
        for entry in ["histogram", "scan", "scatter"] {
            let reflection = compiler
                .compile_source_to_reflection(
                    "radix_sort_blob_layout",
                    &radix_sort_source(),
                    &[entry],
                )
                .expect(entry);
            let params = reflection.root_parameters(entry).expect("root params");
            assert_eq!(
                params.len(),
                1,
                "{entry}: expected exactly one root parameter (uniform params)"
            );
            assert_eq!(params[0].name, "params");
        }
    }
}
