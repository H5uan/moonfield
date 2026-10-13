//! Rust mirrors of the `gs/forward.slang` and `gs/composite.slang` data
//! records and root blobs.
//!
//! Every mirror is `#[repr(C)]` and field-for-field identical to its Slang
//! counterpart: the params blobs are pushed as one root blob per dispatch
//! (the `util/radix_sort.slang` convention — pointer fields as GPU
//! addresses, scalars inline), and the records are the AoS buffer element
//! types. All structured data lives behind `Ptr` on the Slang side, where
//! natural layout rules make `#[repr(C)]` exact; the in-module layout test
//! pins every size and offset against Slang reflection.

/// The `Gaussian3D` AoS record: mirrors `Gaussian3D` in `gs/types.slang`
/// field-for-field (log-space scales, logit opacity, (w, x, y, z) rotation).
/// Slang's natural layout aligns `float3` to 16 bytes — the padding is part
/// of the contract, pinned by the layout test below.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Gaussian3D {
    mean: [f32; 3],
    _pad0: f32,
    log_scale: [f32; 3],
    _pad1: f32,
    rotation: [f32; 4],
    logit_opacity: f32,
    _pad2: [f32; 3],
    sh_dc: [f32; 3],
    _pad3: f32,
}

/// The `SplatView` camera record: mirrors `SplatView` in `gs/types.slang`.
/// `w` is the world→view rotation's three rows at a 16-byte stride.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SplatView {
    w: [f32; 12],
    cam_pos: [f32; 3],
    fx: f32,
    fy: f32,
    _pad0: f32,
    pp: [f32; 2],
}

/// One Gaussian's forward-pass record: mirrors `SplatRecord` in
/// `gs/forward.slang`. Culled Gaussians carry the empty tile rectangle
/// (`tile_min` > `tile_max`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SplatRecord {
    screen: [f32; 2],
    _pad0: [f32; 2],
    conic: [f32; 3],
    depth: f32,
    color: [f32; 3],
    alpha: f32,
    tile_min: [u32; 2],
    tile_max: [u32; 2],
}

/// Root blob for the `project_cull` entry: mirrors `ProjectCullParams` in
/// `gs/forward.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProjectCullParams {
    gaussians: u64,
    records: u64,
    view: u64,
    count: u32,
    tiles_x: u32,
    tiles_y: u32,
    _pad: u32,
}

/// Root blob for the `duplicate_keys` entry: mirrors `DuplicateKeysParams`
/// in `gs/forward.slang`. `counters` points at a two-slot buffer: [0] the
/// next free entry slot, [1] the overflow count.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DuplicateKeysParams {
    records: u64,
    counters: u64,
    tile_keys: u64,
    depth_keys: u64,
    values: u64,
    count: u32,
    tiles_x: u32,
    capacity: u32,
    _pad: u32,
}

/// Root blob for the `clear_tile_ranges` entry: mirrors
/// `ClearTileRangesParams` in `gs/forward.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ClearTileRangesParams {
    ranges: u64,
    tile_count: u32,
    _pad: u32,
}

/// Root blob for the `tile_ranges` entry: mirrors `TileRangesParams` in
/// `gs/forward.slang`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TileRangesParams {
    tile_keys: u64,
    ranges: u64,
    entry_count: u32,
    _pad: u32,
}

/// Root blob for the `blend` entry: mirrors `BlendParams` in
/// `gs/forward.slang`. `image` is the RGBA16F splat image's descriptor-heap
/// storage slot, not an address.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlendParams {
    records: u64,
    values: u64,
    ranges: u64,
    transmittance: u64,
    image: u32,
    width: u32,
    height: u32,
    tiles_x: u32,
}

/// Root blob for the `composite` entry: mirrors `CompositeParams` in
/// `gs/composite.slang`. Both images are descriptor-heap storage slots.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositeParams {
    splat_image: u32,
    target_image: u32,
    width: u32,
    height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_rhi::{Compiler, RootBinder, ShaderTarget};
    use std::mem::{offset_of, size_of};

    /// The repository's `gaussian.slang` — the `gaussian` module's primary
    /// source, with the forward kernels pulled in through its `__include`.
    fn gaussian_source() -> (String, String) {
        let path = moonfield_asset::assets_dir().join("shaders/gaussian.slang");
        let source = std::fs::read_to_string(&path).expect("gaussian.slang");
        (path.display().to_string(), source)
    }

    /// The repository's `gs/composite.slang` — a standalone module.
    fn composite_source() -> (String, String) {
        let path = moonfield_asset::assets_dir().join("shaders/gs/composite.slang");
        let source = std::fs::read_to_string(&path).expect("composite.slang");
        (path.display().to_string(), source)
    }

    struct Case {
        entry: &'static str,
        params_struct: &'static str,
        rust_size: usize,
        fields: &'static [(&'static str, usize)],
    }

    const FORWARD_ENTRIES: [&str; 5] = [
        "project_cull",
        "duplicate_keys",
        "clear_tile_ranges",
        "tile_ranges",
        "blend",
    ];

    fn forward_cases() -> [Case; 5] {
        [
            Case {
                entry: "project_cull",
                params_struct: "ProjectCullParams",
                rust_size: size_of::<ProjectCullParams>(),
                fields: &[
                    ("gaussians", offset_of!(ProjectCullParams, gaussians)),
                    ("records", offset_of!(ProjectCullParams, records)),
                    ("view", offset_of!(ProjectCullParams, view)),
                    ("count", offset_of!(ProjectCullParams, count)),
                    ("tiles_x", offset_of!(ProjectCullParams, tiles_x)),
                    ("tiles_y", offset_of!(ProjectCullParams, tiles_y)),
                ],
            },
            Case {
                entry: "duplicate_keys",
                params_struct: "DuplicateKeysParams",
                rust_size: size_of::<DuplicateKeysParams>(),
                fields: &[
                    ("records", offset_of!(DuplicateKeysParams, records)),
                    ("counters", offset_of!(DuplicateKeysParams, counters)),
                    ("tile_keys", offset_of!(DuplicateKeysParams, tile_keys)),
                    ("depth_keys", offset_of!(DuplicateKeysParams, depth_keys)),
                    ("values", offset_of!(DuplicateKeysParams, values)),
                    ("count", offset_of!(DuplicateKeysParams, count)),
                    ("tiles_x", offset_of!(DuplicateKeysParams, tiles_x)),
                    ("capacity", offset_of!(DuplicateKeysParams, capacity)),
                ],
            },
            Case {
                entry: "clear_tile_ranges",
                params_struct: "ClearTileRangesParams",
                rust_size: size_of::<ClearTileRangesParams>(),
                fields: &[
                    ("ranges", offset_of!(ClearTileRangesParams, ranges)),
                    ("tile_count", offset_of!(ClearTileRangesParams, tile_count)),
                ],
            },
            Case {
                entry: "tile_ranges",
                params_struct: "TileRangesParams",
                rust_size: size_of::<TileRangesParams>(),
                fields: &[
                    ("tile_keys", offset_of!(TileRangesParams, tile_keys)),
                    ("ranges", offset_of!(TileRangesParams, ranges)),
                    ("entry_count", offset_of!(TileRangesParams, entry_count)),
                ],
            },
            Case {
                entry: "blend",
                params_struct: "BlendParams",
                rust_size: size_of::<BlendParams>(),
                fields: &[
                    ("records", offset_of!(BlendParams, records)),
                    ("values", offset_of!(BlendParams, values)),
                    ("ranges", offset_of!(BlendParams, ranges)),
                    ("transmittance", offset_of!(BlendParams, transmittance)),
                    ("image", offset_of!(BlendParams, image)),
                    ("width", offset_of!(BlendParams, width)),
                    ("height", offset_of!(BlendParams, height)),
                    ("tiles_x", offset_of!(BlendParams, tiles_x)),
                ],
            },
        ]
    }

    fn composite_cases() -> [Case; 1] {
        [Case {
            entry: "composite",
            params_struct: "CompositeParams",
            rust_size: size_of::<CompositeParams>(),
            fields: &[
                ("splat_image", offset_of!(CompositeParams, splat_image)),
                ("target_image", offset_of!(CompositeParams, target_image)),
                ("width", offset_of!(CompositeParams, width)),
                ("height", offset_of!(CompositeParams, height)),
            ],
        }]
    }

    /// Assert one case against its entry's reflection: the Rust mirror's size
    /// must equal the reflected root blob size, and every field offset must
    /// match the shader struct layout. Same queries the pipeline build uses
    /// (see `gpu_util::tests::root_blobs_match_entry_point_layout`).
    fn assert_case(reflection: &moonfield_rhi::Reflection, case: &Case) {
        let entry = case.entry;
        let binder = RootBinder::new(reflection, entry).expect("root binder");
        let place = binder.uniform_param("params").expect("params placement");
        assert_eq!(
            case.rust_size, place.size,
            "{entry}: Rust mirror struct size must equal the reflected root blob size"
        );
        let layout = reflection
            .struct_layout(case.params_struct)
            .expect("struct layout");
        for &(name, offset) in case.fields {
            let shader_offset = layout.field_offset(name).expect("field offset");
            assert_eq!(
                offset, shader_offset,
                "{entry}.{name}: Rust mirror offset must match the shader struct offset"
            );
        }
    }

    #[test]
    fn root_blobs_match_entry_point_layout() {
        let compiler = Compiler::new().expect("slang compiler");
        let (path, source) = gaussian_source();
        let reflection = compiler
            .compile_source_to_reflection(&path, &source, &FORWARD_ENTRIES, ShaderTarget::Spirv)
            .expect("gaussian reflection");
        for case in &forward_cases() {
            assert_case(&reflection, case);
        }

        let (path, source) = composite_source();
        let reflection = compiler
            .compile_source_to_reflection(&path, &source, &["composite"], ShaderTarget::Spirv)
            .expect("composite reflection");
        for case in &composite_cases() {
            assert_case(&reflection, case);
        }
    }

    #[test]
    fn root_params_is_a_single_uniform_blob() {
        let compiler = Compiler::new().expect("slang compiler");
        let (path, source) = gaussian_source();
        let reflection = compiler
            .compile_source_to_reflection(&path, &source, &FORWARD_ENTRIES, ShaderTarget::Spirv)
            .expect("gaussian reflection");
        for entry in FORWARD_ENTRIES {
            let params = reflection.root_parameters(entry).expect("root params");
            assert_eq!(
                params.len(),
                1,
                "{entry}: expected exactly one root parameter (uniform params)"
            );
            assert_eq!(params[0].name, "params");
        }

        let (path, source) = composite_source();
        let reflection = compiler
            .compile_source_to_reflection(&path, &source, &["composite"], ShaderTarget::Spirv)
            .expect("composite reflection");
        let params = reflection
            .root_parameters("composite")
            .expect("root params");
        assert_eq!(
            params.len(),
            1,
            "composite: expected exactly one root parameter (uniform params)"
        );
        assert_eq!(params[0].name, "params");
    }

    /// The buffer records: the AoS `Gaussian3D`, the camera `SplatView`, and
    /// the per-Gaussian `SplatRecord` cross the CPU/GPU boundary behind
    /// `Ptr`, where natural layout must match the `#[repr(C)]` mirrors
    /// exactly.
    #[test]
    fn buffer_records_match_natural_layout() {
        let compiler = Compiler::new().expect("slang compiler");
        let (path, source) = gaussian_source();
        let reflection = compiler
            .compile_source_to_reflection(&path, &source, &FORWARD_ENTRIES, ShaderTarget::Spirv)
            .expect("gaussian reflection");

        struct Record {
            name: &'static str,
            rust_size: usize,
            fields: &'static [(&'static str, usize)],
        }
        let records = [
            Record {
                name: "Gaussian3D",
                rust_size: size_of::<Gaussian3D>(),
                fields: &[
                    ("mean", offset_of!(Gaussian3D, mean)),
                    ("log_scale", offset_of!(Gaussian3D, log_scale)),
                    ("rotation", offset_of!(Gaussian3D, rotation)),
                    ("logit_opacity", offset_of!(Gaussian3D, logit_opacity)),
                    ("sh_dc", offset_of!(Gaussian3D, sh_dc)),
                ],
            },
            Record {
                name: "SplatView",
                rust_size: size_of::<SplatView>(),
                fields: &[
                    ("w", offset_of!(SplatView, w)),
                    ("cam_pos", offset_of!(SplatView, cam_pos)),
                    ("fx", offset_of!(SplatView, fx)),
                    ("fy", offset_of!(SplatView, fy)),
                    ("pp", offset_of!(SplatView, pp)),
                ],
            },
            Record {
                name: "SplatRecord",
                rust_size: size_of::<SplatRecord>(),
                fields: &[
                    ("screen", offset_of!(SplatRecord, screen)),
                    ("conic", offset_of!(SplatRecord, conic)),
                    ("depth", offset_of!(SplatRecord, depth)),
                    ("color", offset_of!(SplatRecord, color)),
                    ("alpha", offset_of!(SplatRecord, alpha)),
                    ("tile_min", offset_of!(SplatRecord, tile_min)),
                    ("tile_max", offset_of!(SplatRecord, tile_max)),
                ],
            },
        ];

        for record in &records {
            let layout = reflection
                .struct_layout(record.name)
                .expect("struct layout");
            assert_eq!(
                record.rust_size,
                layout.size(),
                "{}: Rust mirror size must equal the reflected natural size",
                record.name
            );
            for &(name, offset) in record.fields {
                let shader_offset = layout.field_offset(name).expect("field offset");
                assert_eq!(
                    offset, shader_offset,
                    "{}.{name}: Rust mirror offset must match the shader struct offset",
                    record.name
                );
            }
        }
    }
}
