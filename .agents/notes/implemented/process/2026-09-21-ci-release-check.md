# Agent Note: CI checks the workspace without debug_assertions

Status: implemented

[中文](2026-09-21-ci-release-check.zh.md)

## Problem

Both CI jobs that compile Rust — clippy and test — run in the dev profile,
where `debug_assertions` is on. Code that only compiles with
`debug_assertions` off, such as a panic message reading a field gated by
`#[cfg(debug_assertions)]`, fails `cargo build --release` while the gate stays
green; that is how
[the ComponentMeta type_name fix](../bug-fix/2026-09-21-componentmeta-debug-only-field-accessor.md)
shipped. Compiling the workspace needs `SLANG_DIR` (shader-slang-sys's build
script) and libclang (its bindgen), so any job that compiles it carries the
same setup the clippy job has.

## Decision

A `release-check` job runs `cargo check --release --workspace --all-targets
--locked` on both supported runners (ubuntu-latest, windows-latest), with the
clippy job's libclang and Slang setup. `check`, not `build`: no codegen or
linking, so every target is type-checked under `debug_assertions = false`
without paying release compile cost.

## Alternatives considered

- **A release-check step inside the clippy job.** Rejected: saves the setup
  duplication but couples lint and profile coverage in one job status, and
  grows the clippy cache with release artifacts.
- **`cargo test --release`.** Rejected: full release codegen on every push for
  no additional cfg coverage; check already type-checks every target.

## Consequences

- Release-only compile breakage fails CI on both supported targets.
- The job checks without linking or running, so runtime behavior of optimized
  builds stays untested.
