# Agent Note: Shaders sourced from assets/shaders files

Status: implemented

[中文](2026-08-26-shader-sourcing-from-files.zh.md)

## Problem

The two production shader modules — the core 3D flat-lit mesh pass and the
egui→Vulkan backend — were spelled inline as `&str` constants in Rust source
(`VERTEX_SHADER` / `FRAGMENT_SHADER` in `moonfield-render-feature::core_3d`,
`SHADER_SOURCE` in `moonfield-editor::egui_vk`).
`Compiler::compile_source_to_spirv` worked around the Slang crate's file-based
API by writing the source to a temp file on every pipeline build. Inline
strings hide the shader from editors and diff review, reuse none of the repo's
asset layout, and force every minor shader tweak through a Rust recompile.

## Decision

Production shaders now live as Slang files under the repository's asset
directory, `<repo root>/assets/shaders/`:

- `core_3d.slang` — the core 3D pass, one module with `vs_main` / `fs_main`.
- `egui.slang` — the egui backend, one module with `vs_main` and the
  `fs_gamma` / `fs_linear` fragment entries.

The pipelines no longer locate or compile the files themselves: both shaders
are `Shader` assets loaded through the asset layer and compiled into
revision-matched prepared artifacts in the render world — see
[Shaders as assets with render-world prepared compilation](2026-09-06-shader-as-asset.md).
The compiler's module names come from the file paths.

`compile_source_to_spirv` stays in the RHI: the headless/offscreen triangle
tests, the bindless compute tests, and the `headless_triangle` example keep
their shaders inline so each test stays self-contained.

## Alternatives considered

- **Keep inline strings.** Rejected: that is the status quo this note replaces
  — a temp-file round trip on every pipeline build, and shader source hidden
  from editors and diff review.
- **`include_str!` embedded files.** Rejected: the file is not editable in
  place — every change still needs a Rust recompile, and the source is copied
  into the binary for no runtime benefit.
- **Per-crate `assets/shaders/` directories.** Rejected: the repo already
  centralizes repository-managed assets at the root `assets/` (`models/`), and
  both consumers resolve the same shared directory with the same relative hop.

## Consequences

- Shader edits are plain file edits: no Rust rebuild, and the diff shows the
  shader itself rather than a string-constant wrapper.
- The editor's startup shader load resolves the repository layout
  `<repo root>/assets/shaders/` through a compile-baked path (the same
  convention as the default-scene mesh); the pipelines themselves are
  path-free.
- Test and example shaders (RHI tests, `headless_triangle`) remain inline by
  design, so `Contract`-style self-containment of each GPU test is preserved.