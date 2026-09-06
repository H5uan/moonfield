# Agent Note: Split vulkan/shader.rs into a shader module directory

Status: implemented

[中文](2026-09-06-shader-module-split.zh.md)

## Problem

`crates/moonfield-rhi/src/vulkan/shader.rs` had grown to 1479 lines mixing
three responsibilities — Slang→SPIR-V compilation and caching, Slang
reflection queries, and root-parameter binding for the descriptor-heap
pipelines. The file's most delicate item sat buried mid-file: `Reflection`,
a self-referential struct that owns a Slang session and a linked
`ComponentType` while holding a raw pointer into them, with hand-rolled
`unsafe impl Send/Sync` whose invariants were recorded only in a two-line
comment next to the impls. Finding the unsafe contract required reading the
whole file.

## Decision

The file became the module directory
`crates/moonfield-rhi/src/vulkan/shader/`:

- `mod.rs` — module docs, the private `map_slang_error` helper shared by all
  three submodules, and re-exports; every `vulkan::shader::{...}` path
  resolves exactly as before, so `vulkan/mod.rs` and the downstream crates
  (`moonfield-render-feature`, `moonfield-editor`) are untouched.
- `compile.rs` — `CompiledShader`, the SPIR-V entry-point name extraction,
  the Slang stage → `vk::ShaderStageFlags` mapping, `Compiler`, and
  `ShaderCache` (with `ShaderCacheKey`, the private `get_or_reflect` helper,
  and all `compile_*` methods including `compile_source_reflection`).
- `reflection.rs` — `Reflection`, `UserAttributeRef`/`UserAttributeArg`,
  `Layout`, and the field type/byte-size helper. The module doc states the
  `Reflection` invariants plainly: the session and the linked component type
  are owned by the wrapper and outlive the raw pointer by construction, and
  every access is read-only through `&self`.
- `root_binder.rs` — `RootParamKind`, `RootParam`, `RootParamPlace`,
  `RootBinder`, plus the `Reflection::root_parameters` impl block that
  produces the `RootParam` list the binder consumes. Keeping that impl here
  (instead of in `reflection.rs`) is what makes the dependency chain one-way:
  `compile` ← `reflection` ← `root_binder`.

`Reflection`'s fields became `pub(super)` so `compile.rs` can construct the
wrapper and `root_binder.rs` can dereference the pointer; no name left the
module that was not already re-exported. Tests moved with the types they
cover; the codegen test's `include_str!` path gained one `../` for the extra
directory depth.

## Alternatives considered

- **Keep everything in one file.** Rejected: the file had passed the size
  where the unsafe `Reflection` contract could be found by browsing; the
  split is what lets that contract own a module doc.
- **Move reflection and root binding into a separate crate.** Rejected: the
  machinery is the rhi's private binding contract — `CompiledShader`'s
  fields are `pub(crate)` and the wrapper construction is module-private —
  so extraction would force widening the public API the boundary check
  (`scripts/verify_rhi_boundary.py`) guards, for no reuse consumer.
- **Keep `Reflection::root_parameters` in `reflection.rs`.** Rejected: it
  returns `RootParam`s, which would make `reflection` depend on
  `root_binder` and invert the intended one-way chain; an impl block can
  live in the consuming file without changing the public path.

## Consequences

- The `Reflection` invariants now live in `reflection.rs`'s module doc,
  directly above the `unsafe impl Send/Sync` they justify.
- `vulkan::shader::Layout` and the other re-exports keep their exact paths;
  the only mechanical fallout was the one `include_str!` depth change.
- New shader-side work has an obvious home: compilation changes go to
  `compile.rs`, reflection queries to `reflection.rs`, push-data binding to
  `root_binder.rs`.
