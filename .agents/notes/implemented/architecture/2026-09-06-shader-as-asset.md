# Agent Note: Shaders as assets with render-world prepared compilation

Status: implemented

[中文](2026-09-06-shader-as-asset.zh.md)

## Problem

Production shaders were files under `assets/shaders/` (see
[Shaders sourced from assets/shaders files](2026-08-26-shader-sourcing-from-files.md)),
but each pipeline still located and compiled its own: `Core3dPipeline` and
the editor's `EguiPipeline` resolved the directory through
`env!("CARGO_MANIFEST_DIR")` joined with `../../assets/shaders` — a
compile-time-baked, non-relocatable path — and drove the RHI `Compiler`
directly. Shaders were the only render input outside the asset layer: no
`Handle`, no `AssetRevision`, no extraction, so nothing downstream could
observe a shader change, and path resolution belonged to renderer internals
instead of the app.

## Decision

A new `moonfield-shader` crate owns the `Shader` asset — the source file's
path plus its Slang source text — and a `SlangLoader` serving `.slang` files
through the `AssetServer` (a synchronous `std::fs` read; the asset layer is
sync-only). The asset carries no entry-point metadata: entry points are
discovered by Slang reflection at compile time. Compilation, reflection, and
root binding stay in `moonfield-rhi`.

The render side mirrors the mesh feature's prepared-asset pattern in
`moonfield-render-feature::shader`:

- A pipeline declares its shader needs as a `PipelineShader` request — the
  asset handle, the entry points with their capabilities, and the entry
  whose reflection drives root binding — registered in the main-world
  `PipelineShaders` resource by whoever loads the shader assets (the editor
  loads `core_3d.slang` and `egui.slang` at startup through its
  `AssetServer`).
- `extract_shader_assets` copies the requested shaders (revision-matched,
  like `extract_mesh_assets`) and the request list into the render world.
- `prepare_shaders` (`RenderPrepare`) compiles each request whose
  `AssetRevision` advanced into the render-world `PreparedShaders` resource,
  which owns the shared `ShaderCache` (created lazily — no Slang session or
  Vulkan device until the first compile). Compilation runs from the asset's
  source text, so the cache's memoization keys observe source changes.
  `PreparedShaders` is keyed by pipeline name because the compiled entry set
  is pipeline-declared; each slot records the prepared artifacts
  (`PreparedShader`: the module reflection plus one `CompiledShader` per
  entry) or the compile error, so a broken source is not recompiled every
  frame and the pass keeps running the pipeline it already built.
- `Core3dPipeline` and `EguiPipeline` hold their `Handle<Shader>` and the
  revision they were built from; their passes rebuild the pipeline when the
  prepared revision advances and skip with a one-shot log while the shader
  isn't ready.

The `CARGO_MANIFEST_DIR` shader lookup in both pipeline sites is deleted.
Path resolution moved to the app wiring: the editor's startup load passes
the repository shader directory to `AssetServer::load`, following the same
convention as the default-scene mesh. The RHI gained one additive method,
`ShaderCache::compile_source_reflection` — the memoized source-text
counterpart of `compile_file_reflection` — so prepared shaders compile from
the asset's source rather than re-reading the file (the file-based cache key
cannot see a source change at the same path).

## Alternatives considered

- **Keep shaders code-internal.** Rejected: that is the status quo this note
  replaces — compile-time-baked paths inside renderer internals, no revision
  tracking, and no way for the asset layer to observe shaders.
- **A bevy_shader-style full asset-graph crate** with import resolution and
  hot reload. Rejected: the asset layer is deliberately sync-only with no
  file watching, and no current shader uses `import`; the machinery would be
  speculative. The `Shader` asset leaves room for it.
- **Compile prepared shaders from the asset's file path** through the
  existing `ShaderCache::compile_file*`. Rejected: the file-based cache keys
  on the path, so an edited or reloaded source at the same path would
  silently reuse stale artifacts; keying compiles on the source text is what
  makes the revision model honest.

## Consequences

- Whoever owns the app loads the pipeline shaders; an app without the editor
  must load them itself or the passes skip with a one-shot log. The editor
  does it at startup, so first-frame behavior matches the previous
  compile-at-creation model.
- `PreparedShaders` is keyed by pipeline name, not by `AssetId` alone: a
  shader asset shared by two pipelines gets one slot per pipeline (the
  underlying `ShaderCache` memoizes the identical compiles).
- Shader compilation moved from pipeline creation (in `Render`) to
  `RenderPrepare` — one schedule earlier within the same frame, on the same
  thread.
- A failed recompile records the error for the new revision; the running
  pipeline is untouched. Fixing the source and reloading the asset (a new
  revision) re-arms compilation — there is still no hot reload.
