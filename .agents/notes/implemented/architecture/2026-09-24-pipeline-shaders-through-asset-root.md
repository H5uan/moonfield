# Agent Note: Pipeline shaders load through the asset pipeline, one reflection per pipeline

Status: implemented

[中文](2026-09-24-pipeline-shaders-through-asset-root.zh.md)

## Problem

Three defects shared one root: the shader-asset pipeline existed but not
every pipeline rode it.

1. The splat sort pass embedded `radix_sort.slang` with `include_str!` and
   a fake path label (`crates/moonfield-render-feature/src/splat/sort_pass.rs`),
   bypassing `AssetServer` loading, path dedup, revision tracking, and the
   `PipelineShaders` → extract → `prepare_shaders` flow the mesh and egui
   pipelines use. The same file had three load paths (two `include_str!`
   sites, one asset-server test).
2. `PreparedShaders` linked its reflection program with the module and the
   request's `reflect_entry` only, so the reflection answered root-binding
   queries for that single entry. A multi-entry pipeline — the radix sort's
   `histogram`/`scan`/`scatter` compute entries, each with its own
   `uniform Params` root blob — could not bind from one prepared shader.
3. Every consumer that needed a repository asset at runtime or in tests
   hand-rolled `env!("CARGO_MANIFEST_DIR")` + `"../../assets"` — the editor's
   shader loads and default mesh, four integration tests, and three rhi
   reflection probes — with no override for a binary run outside the
   checkout.

## Decision

- `moonfield-rhi` links one program per pipeline:
  `Compiler::compile_source_to_reflection` and
  `ShaderCache::compile_source_reflection` take `entry_points: &[&str]` and
  build the composite of the module plus every listed entry
  (`ISession::createCompositeComponentType` unions their entry points), so
  one `Reflection` answers per-entry queries — `root_parameters`,
  thread-group sizes — for each. A one-element slice reproduces the old
  single-entry link exactly.
- `PreparedShaders::compile_request` reflects every entry the request
  declares (plus `reflect_entry` when a request leaves it outside `entries`,
  so no valid request compiles less than before).
- `RadixSort` gains `from_prepared` — the three pipelines and root
  placements built from a `PreparedShader` — and `new` shares the same
  builder, compiling one multi-entry reflection instead of three.
- The splat sort pass declares `SPLAT_SORT_SHADER` /
  `splat_sort_shader(handle)` like `core_3d_shader`, and
  `prepare_splat_sort` follows `prepare_core_3d_pipeline`: skip with a
  one-shot log while the request or prepared shader is missing, rebuild
  when the prepared revision advances. No `include_str!` remains in
  production code; the acceptance test registers its request through an
  `AssetServer` + `SlangLoader`, the same shape as `tests/radix_sort.rs`.
- The editor loads `util/radix_sort.slang` under the `splat` feature
  (alongside `core_3d.slang` and `egui.slang`); the plugin itself stays
  opt-in — the shipping editor does not run the synthetic sort.
- `moonfield_asset::assets_dir()` is the one definition of the repository's
  asset root: the `MOONFIELD_ASSETS_DIR` environment variable when set,
  then the path compiled in from `moonfield-asset`'s place in the workspace.
  The editor, the integration tests, and the rhi probes (virtual module
  names for import resolution) all resolve through it; `moonfield-rhi`
  takes `moonfield-asset` as a test-only dev-dependency for the probes.
  After the sweep, no `env!("CARGO_MANIFEST_DIR")` asset reference or
  `include_str!` of the shader tree remains anywhere outside
  `assets_dir()` itself.

## Alternatives considered

- **Per-entry reflections through the existing single-entry API** (link the
  module once per entry, keep `&str` signatures): rejected — it compiles and
  links the same module three times for one pipeline and diverges from
  Slang's intended shape, where a program's reflection covers all its entry
  points.
- **Keep `include_str!` in the sort pass, dedupe the constant**: rejected —
  embedding still bypasses revision tracking and the prepare flow, and a
  shader edit would not rebuild the sort until the next process start.
- **A shader search-path / include-root mechanism in the rhi compiler** for
  user `import`s outside the shader's directory: deferred — the probes'
  virtual module names still assume the repository layout, but that is the
  documented known-debt of import resolution generally; this change does
  not alter it.
- **Embedding the asset tree into release binaries**: deferred —
  `MOONFIELD_ASSETS_DIR` is the deployable escape hatch; embedding is a
  packaging decision with its own note when it lands.

## Consequences

- The `ShaderCache` reflection key joins entry names with `','` — lossless
  because Slang entry-point names are identifiers.
- One linked program per pipeline means global-scope shader parameters are
  laid out once, shared across entries; none of the built-in pipelines
  declare global-scope parameters, so behavior is unchanged for them.
- `moonfield-rhi` gained a `[dev-dependencies]` entry on `moonfield-asset`
  (a zero-dependency leaf) — test-only, so the rhi's published dependency
  graph and the boundary check are unaffected.
- The editor loads two shaders by default and three with `splat`; the
  pipeline-shader test asserts the count per feature set.
- Verified locally: all 42 `moonfield-render-feature` tests under `splat`
  (including the GPU acceptance test, which now sorts through the prepared
  asset), 49 `moonfield-rhi` tests, `moonfield-ml`, and `moonfield-editor`
  under `splat` pass; `cargo clippy --workspace --all-targets -- -D warnings`
  is clean with and without the `splat` feature.
