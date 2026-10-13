# Agent Note: validation layers toggled by a Cargo feature

Status: implemented

[中文](2026-08-24-vk-validation-via-cargo-feature.zh.md)

## Problem

The Khronos validation layer was requested when the
`MOONFIELD_VK_VALIDATION` environment variable was set, checked at runtime
inside `Instance::new`. The switch was hidden, global state: nothing in the
crate's manifest or API advertised it, a release build could silently keep
the code path, and CI could not express the choice declaratively.

## Decision

`moonfield-render` gains a `validation` Cargo feature (off by default) in its
`Cargo.toml`, and `crates/moonfield-render/src/vulkan/instance.rs` appends
`VK_LAYER_KHRONOS_validation` under `#[cfg(feature = "validation")]`. The
decision is now compile-time: `cargo run --features moonfield-render/validation`
for the editor, `cargo test --features moonfield-render/validation` for the
headless tests. The feature only decides whether to request the layer; the
Vulkan SDK must still be installed at runtime, because layers load as shared
libraries at instance creation. Any future `VK_EXT_debug_utils` messenger
belongs under the same feature.

## Alternatives considered

- **Keep the `MOONFIELD_VK_VALIDATION` environment variable.** Rejected: it is
  a runtime toggle with no manifest or API surface — hidden state that release
  builds can retain and CI cannot declare.
- **`#[cfg(debug_assertions)]` auto-on in debug builds.** Rejected: zero-config
  and follows engine convention, but gives up per-run and per-CI control — no
  way to disable validation in a debug build or enable it in a release build.
- **Cargo feature with an env-var override layered on top.** Rejected: two
  knobs for one switch; the feature alone is simpler and the override would
  recreate the hidden-state problem at a second level.

## Consequences

- Toggling validation now requires a rebuild — the switch is a compile-time,
  per-profile decision.
- Release builds compile the layer request out entirely, so a
  `cargo run --release` can never accidentally request validation.
- The opt-in is explicit and declarative, so CI jobs and editors can express
  it in their own configuration; the CLI still needs the Vulkan SDK installed
  regardless of the feature.