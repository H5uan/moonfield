# Agent Note: The editor is the workspace's only binary

Status: implemented

[中文](2026-08-20-editor-as-sole-binary.zh.md)

## Problem

Two runnable targets competed for the entry-point role: `cargo run` built the
`moonfield` binary crate — a demo that printed FPS and never loaded the
editor — while the product entry lived in
`crates/moonfield-editor/examples/editor.rs`. The crate named after the
project was not the product, and the editor could only be launched through an
example target.

## Decision

`moonfield-editor` is the workspace's only binary crate: the former
`examples/editor.rs` moved to `src/main.rs` (the binary takes the package
name), and the `moonfield` crate is deleted — its demo main covered nothing
the editor binary and the test suite do not. The root `Cargo.toml` sets
`default-members = ["crates/moonfield-editor"]` so a bare `cargo run`
launches the editor.

## Alternatives considered

- **Keep `moonfield` as a thin binary that only composes `EditorPlugin`.**
  Rejected: a crate whose entire content is one plugin-composing main adds an
  indirection with no owner; the binary belongs with the editor crate.
- **Move the demo main into another crate's `examples/`.** Rejected: it
  exercised `LogPlugin` + `TimePlugin` + `print_fps`, nothing the editor
  binary does not already cover; keeping it would preserve a second runnable
  surface nobody maintains.

## Consequences

- `cargo run` at the workspace root builds and launches the editor; no
  non-editor binary target exists.
- The editor binary owns the startup demo scene (camera + parent/child
  cubes), which doubles as the smoke-test vehicle via
  `MOONFIELD_EDITOR_AUTO_CLOSE`.
- `EditorPlugin` stays a pure plugin — the binary only composes plugins, so
  embedding the editor in another app remains possible.
- The deleted crate was a binary-only leaf: nothing in the workspace depended
  on it and it exposed no library surface.
