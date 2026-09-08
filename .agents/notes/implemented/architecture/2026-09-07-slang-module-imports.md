# Agent Note: Slang module imports resolve through the module-name path hint

Status: implemented

[中文](2026-09-07-slang-module-imports.zh.md)

## Problem

The Gaussian Splatting roadmap makes `assets/shaders/gs/gaussian.slang` a shared library — training kernels and rendering shaders import its covariance/projection math — and no shader in the workspace used `import` before. The RHI's source-string compilation passed Slang a fake filename hint (`{module_name}.slang`), so an asset-compiled source had no directory context and its imports could not resolve.

## Decision

`compile_source*` passes `module_name` straight through as Slang's file-path hint: a module name that is a real path lets the source import sibling modules from that file's directory; a plain name resolves imports against the process working directory. The hint file does not need to exist — only its directory does — so consumers compile wrapper sources under virtual paths (`assets/shaders/gs/__gs_math_test.slang`) with no fixture on disk. The shader cache key already contains the module name, so import participation in memoization is correct. Cross-directory imports are not covered; `SessionDesc::search_paths` (already exposed by shader-slang-rs) is the escape hatch when a consumer needs them.

`gaussian.slang` is a library module, not an asset: pipeline shaders are assets that import it, and the compiler's module system — not the asset layer — owns its resolution. The file's header declares the view-space convention (camera looks down +z, x right, y down — the COLMAP/3DGS shape); converting from the engine camera's matrices happens where `SplatView` is assembled, not in the file.

Verification is `render-feature`'s `gs_math` test: 64 seeded Gaussians through `cov3d`/`project`/`eval_color` on the GPU, compared component-wise against an independent glam reference (glam's own quaternion and matrix operations, never a transcription of the Slang formulas) within 1e-4 relative.

## Alternatives considered

- **Session search paths now.** Lost: no consumer needs cross-directory imports today; the path hint covers same-directory use (the gs kernels, the test wrapper) with no API surface. Search paths remain the fallback.
- **A transliterated CPU reference (a Rust copy of the Slang formulas).** Lost: it would agree with any transcription bug; the glam path is what made the matrix-semantics errors below visible.
- **Scalar-only math (spike style).** Lost: hand-expanded trigonometry does not scale to covariance/EWA; the matrix form needed its semantics fixed once instead.

## Consequences

- Slang matrix semantics, probe-verified and reference-confirmed: `float3x3(v0, v1, v2)` takes rows; `m[i][j]` is [row][col]; `*` between matrices is componentwise (HLSL) — matrix products, like matrix×vector, go through `mul()`. The componentwise trap is the dangerous one: types check, compilation passes, and `A·Aᵀ` silently degenerates to `A⊙Aᵀ`.
- `no_diff` struct parameters (`SplatView`) are accepted in `[Differentiable]` functions; non-`public` struct fields are not visible to importing modules, so shared structs carry explicit `public`.
- Imported modules load from disk at each compile; the rhi cache keys on the importing source, so editing only a library module does not invalidate an importer's cached compile within a process. No current consumer edits shader files at runtime, and hot reload stays out of scope per the asset decision.
- The rhi import probe (`source_import_resolves_through_module_name_path`) compiles a wrapper that imports `gaussian` and calls all three functions, keeping the mechanism under test.
