# Agent Note: Splat forward pass

Status: implemented

[中文](2026-10-14-splat-forward-pass.zh.md)

## Problem

The [Gaussian splatting roadmap](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md)'s M3 needed the tile-based forward pass; nothing splat-related touched the GPU. The splat modules held scene data, a loader, and a rasterizer stub — no upload path, no extraction, no kernels — so no splat cloud could appear in a frame.

## Decision

The forward is a chain of compute dispatches recorded per view in the `Core3d` schedule after the opaque pass ([render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md)), followed by a composite into the view target. The kernels live in `assets/shaders/gs/forward.slang` and `gs/composite.slang`; the Rust side is `render-feature`'s `splat::{forward, prepare, forward_pass}` modules. Five decisions shape it:

- **Tile bucketing is atomic append, no prefix sum.** `duplicate_keys` takes one slot per covered tile from a global counter (`InterlockedAdd`) and writes the (tile, depth, Gaussian) triple there. Overflow past the entry capacity (N×16) is dropped and counted, observed but not fatal.
- **The (tile, depth) composite order is two stable u32 `RadixSort` runs over the full entry capacity.** A `clear` kernel sentinel-fills the tile keys with `0xFFFFFFFF` and zeroes the counters each frame, so both sorts and `tile_ranges` run over the capacity and the appended entry count never leaves the GPU — no mid-frame readback. The `gather` entry re-reads tile keys and Gaussian ids through the depth sort's slot permutation between the two sorts.
- **The composite is a compute pass, not a graphics pass.** `OffscreenTarget` carries `STORAGE` usage and exposes `storage_handle()`; the kernel reads the splat texel and the view target's own texel through their descriptor-heap storage slots and writes back `dst * (1 - alpha) + reinhard(color)` — same-texel read-then-write within one thread, no feedback copy ([rhi storage image](2026-09-09-rhi-storage-image.md) for the heap-slot shape). A `recorded` flag on the per-view artifacts makes the composite exactly-once per blend, so a stale splat image never reaches the view target.
- **The kernels are shared-ready for the M4 trainer.** Each entry takes one `uniform` params blob of pointer fields and inline scalars over caller-owned buffers, so the trainer's raw-`CommandBuffer` surface can record the same dispatches; M3 wires only the renderer side (through the [render recording context](2026-09-09-render-recording-context.md)'s compute door, whose automatic `COMPUTE → COMPUTE` barriers serialize every link).
- **The COLMAP ↔ moonfield camera flip lives in one place.** `forward_pass::splat_view` premultiplies the world→view rotation by `diag(1, -1, -1)` and derives focal lengths from the projection matrix's cotangent terms; the gs module keeps the +z-forward, y-down convention throughout.

Two contract lessons the implementation pinned down:

- **`Ptr<T>` pointees use the tight natural layout** (a `float3` is 12 bytes, 4-aligned) — not what `Reflection::struct_layout`'s default rules report (16-aligned, std430-flavored). The rhi gained `Reflection::field_pointee_layout`, which reflects through a pointer field to the layout actually baked into the pointer arithmetic; the record mirrors (`Gaussian3D`, `SplatView`, `SplatRecord` in `splat::forward`) pin their layouts against it ([bindless rhi gpu pointer model](2026-08-19-bindless-rhi-gpu-pointer-model.md) for the pointer convention).
- **`eval_color` returns `sh_dc` verbatim** — the `0.282095·c + 0.5` SH bias has no implementation in the forward path, so display-ready RGB is stored as the DC term (the synthetic asset does this). M4 decides whether the bias lands; if it does, `assets/models/synth_splats.glb` and the `gs_forward` test colors need regenerating.

## Alternatives considered

- **Extending `RadixSort` to u64 keys.** Lost: it touches `radix_sort.slang` and every layout test for a marginal pass-count saving; two stable u32 runs need zero shader changes and cost four extra scatter passes.
- **The reference implementation's per-Gaussian prefix-sum bucketing.** Lost: it needs the scan pass split out of `RadixSort` for standalone use; atomic append plus the sort reaches the same per-tile order with one kernel family fewer.
- **A graphics composite (fullscreen triangle, alpha blend state).** Lost: sampling the view target while rendering into it is a feedback loop, and a copy of the target costs more than the compute read-modify-write.
- **Blending directly into the view target.** Lost: the target is 8-bit and offers no tonemap point — the [render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md) already declined it.

## Consequences

- The viewing path pays artifact-write bandwidth every frame (the records, entries, and per-pixel transmittance), per the roadmap's shared-forward decision; the transmittance buffer is the M4 backward's input.
- Entry capacity is N×16 per view; a splat covering more tiles than its share drops entries and bumps the overflow counter rather than corrupting the sort.
- The editor's default features include `splat`, so `cargo run` renders splat clouds; the synthetic reference asset is generated by `cargo run -p moonfield-render-feature --example gen_splat_asset --features splat` (the example round-trips its output through `load_splat_gltf`).
- M3's acceptance is met: `tests/gs_forward.rs` holds two GPU tests with pixel assertions (depth-ordered blending into the RGBA16F image; tonemapped composite into the view target over a known clear color), and the editor viewport renders `assets/models/synth_splats.glb` — smoke-verified with a `MOONFIELD_EDITOR_DUMP_VIEWPORT` dump showing the position-colored cloud over the teapot.
