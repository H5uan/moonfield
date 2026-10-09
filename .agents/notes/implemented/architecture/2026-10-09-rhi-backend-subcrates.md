# Agent Note: RHI backend sub-crates

Status: implemented

[中文](2026-10-09-rhi-backend-subcrates.zh.md)

## Problem

`moonfield-rhi` was one crate whose public API was a concrete Vulkan
implementation: `lib.rs` re-exported `vulkan::*` by glob, so the public surface
was implicit and backend module paths (`moonfield_rhi::device`, ...) leaked
out with it. A second backend (the macOS Metal 4 backend) needs the Vulkan
dependency closure (`ash`, `gpu-allocator`, `shader-slang`) isolated from
whatever the Metal backend links, and downstream crates
(`moonfield-render-core`, `moonfield-render-feature`, `moonfield-editor`,
`moonfield-ml`) consume concrete types (`Device`, `Memory`, `GpuPtr`) directly
— a runtime trait layer would rewrite every one of those signatures.

## Decision

`moonfield-rhi` is a facade over backend sub-crates that live under its own
directory:

- `crates/moonfield-rhi/src` — the facade. `src/lib.rs` carries an explicit
  re-export list per backend feature; the lists are the RHI's entire public
  surface. The `error`, `indirect`, and `types` module paths are re-exported
  so `moonfield_rhi::types::WrapMode` keeps resolving.
- `crates/moonfield-rhi/core` — `moonfield-rhi-core`, the backend-agnostic
  vocabulary (`types.rs`, `error.rs`, `indirect.rs`). Nothing in `core`
  mentions a backend: the Vulkan conversions that used to sit as `to_vk`
  methods next to each type moved to `vulkan/src/formats.rs` behind the
  `ToVk` extension trait, and result-code conversion became the free function
  `from_vk`.
- `crates/moonfield-rhi/vulkan` — `moonfield-rhi-vulkan`, the Vulkan backend:
  all of `ash`, `ash-window`, `gpu-allocator`, `shader-slang`, and the GPU
  tests that verify `pub(crate)` internals.

Dependency direction is `moonfield-rhi` → backend sub-crates → `core`; the
sub-crates are workspace members but intentionally absent from
`[workspace.dependencies]` — only `moonfield-rhi` references them, marking
them as internal. Exactly one backend feature (`vulkan` today) is active in a
build. The workspace dependency carries `default-features = false`, so the
editor and every crate that uses backend types enables the feature explicitly
in its own manifest; the facade's `default = ["vulkan"]` covers only
standalone builds of the facade. The `validation` feature forwards to the
backend crate's layer.

`gpu-allocator` is the allocation substrate shared by backends: the crate
carries a `metal` backend (on `objc2`/`objc2-metal`) alongside `vulkan`, so
each backend sub-crate declares its own feature slice — the Vulkan backend
takes `std` + `vulkan`, which also drops the `d3d12`/`metal` slices from its
build — and allocator types stay out of the public API.

`scripts/verify_rhi_boundary.py` scans the facade, `core`, and every backend
sub-crate, and its forbidden set grew from `ash`/`vk::`/`gpu_allocator` to
also `objc2` and `MTL*`.

## Alternatives considered

**A trait-based shared API layer** (abstract `Device`/`CommandBuffer` traits,
per-backend crates implementing them) would enable runtime backend selection
and is the shape most multi-backend RHIs take. It loses here because every
downstream signature would become generic or `dyn`-erased in the same change
the Metal backend lands, and nothing in the repo needs runtime selection:
the editor is the only binary and the platform picks the backend. The facade
re-export keeps the concrete-type consumption model intact, and a trait layer
can still be introduced inside the facade later without moving code again.

**Sibling top-level crates** (`crates/moonfield-rhi-vulkan` next to
`crates/moonfield-rhi`) would give the same dependency isolation. It loses
because `crates/` is one engine concern per crate: the backends are
implementation details of the RHI, not engine-level modules, and nesting
keeps the workspace listing and the rhi rule file (`crates/moonfield-rhi/
AGENTS.md`) singular.

**cfg-gated modules inside one crate** would avoid the crate split entirely.
It loses because the manifest mixes the dependency closures of both backends
(feature-gated `ash` and `objc2` entries in one file), and the isolation
between backends rests on `pub(crate)` discipline instead of package
boundaries. Sub-crates give compile-time dependency isolation and let the
GPU tests keep testing `pub(crate)` internals in the crate that owns them.

## Consequences

Downstream `use moonfield_rhi::{...}` imports are unchanged — the facade
re-exports the same names. The glob-leaked module paths are gone from the
public API; anything reachable now is on the explicit list. Vocabulary
conversions are backend-owned (`ToVk`, `from_vk`), which required two small
vocabulary adjustments: `Format::bytes_per_pixel` is public, and
`CommandBufferUsage` gained a public `contains`. Building a
backend-consuming crate standalone (`cargo build -p moonfield-render-core`)
works because those four manifests enable the `vulkan` feature explicitly;
crates that need no backend types can stay feature-free. The Metal 4 backend
lands as a `metal/` sibling with a `metal` feature, per
[the Metal 4 backend proposal](../../proposed/architecture/2026-10-09-metal-4-backend.md).
