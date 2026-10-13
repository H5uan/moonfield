# Agent Note: moonfield tracks shader-slang-rs master

Status: implemented

[中文](2026-08-21-shader-slang-master-follow.zh.md)

## Problem

Running the editor failed at startup with
`libslang-compiler.so.0.2026.14.1: cannot open shared object file`. The
`shader-slang-rs-sys` build script copied the Slang runtime libraries into
`target/debug/build/`, which the dynamic loader never searches. Its
`copy_runtime_libs_to_profile_dir` computed the profile directory as
`out_dir.ancestors().nth(3)`, an assumption that only holds for the legacy
target layout `<profile>/build/<pkg>-<hash>/out`; the current Cargo layout
(`<profile>/build/<name>/<hash>/out`) resolves to one level too high.
Separately, the fork's master branch had moved ahead of the pinned commit
with breaking API changes (`Reflection::find_type_by_name` now returns
`Result<Option<&Type>>`).

## Decision

- The dependency tracks the `shader-slang-rs` git master branch (no
  `rev` pin). Bug fixes and improvements land on that fork's mainline
  first; moonfield only follows mainline.
- The `find_type_by_name` call site in `vulkan/shader.rs` adapted to the
  `Result<Option<&Type>>` contract: map the error, then require the
  `Option`.
- The one-off manual copy of `libslang*` into the profile directory is
  no longer needed after the upstream fix (`fix(sys): resolve profile dir
  for current cargo target layouts`, on the fork's master).

## Alternatives considered

- **Pin a `rev` to a dedicated fix branch.** Rejected: mainline is the
  single source of truth; a pinned branch adds lock maintenance and
  drifts from the fork's default branch.
- **Keep a local `path` dependency.** Rejected: an absolute machine-local
  path breaks builds for anyone else and does not reflect the fork's
  reality.
- **Forks the fix locally with a `[patch]`.** Rejected: it would diverge
  the codebase from the fork's mainline and duplicate a fix that belongs
  upstream.

## Consequences

- `cargo run` / `cargo test` load the Slang runtime libraries without
  manual setup on both legacy and current Cargo target layouts.
- `vulkan/shader.rs::struct_layout` now surfaces a lookup error from
  `find_type_by_name` instead of collapsing it into a bare not-found
  message.
- Dependency moves forward with the fork's master; future upstream API
  changes are absorbed here when they arrive.