# crates/ — Rust workspace rules

Rules for writing moonfield crates. Read the [root AGENTS.md](../AGENTS.md) for
standing repo-wide rules and [docs/architecture.md](../docs/architecture.md) for
runtime mechanisms.

## Naming

- `snake_case` for modules, functions, and variables; `PascalCase` for types and
  enums.
- Module files mirror their logical grouping (e.g. `device.rs`, `swapchain.rs`,
  `pipeline.rs` in `moonfield-rhi`).
- The workspace math single entry is `moonfield-math` (glam re-export + domain
  types); other crates import math from it, not from glam directly. Exception:
  `moonfield-reflect` depends on glam directly to avoid a math↔reflect cycle
  (`moonfield-math` derives `Reflect` for `Transform`).

## Style

- Follow standard `rustfmt` formatting — run `cargo fmt` before committing.
- Run `cargo clippy` and resolve all warnings before opening a PR.
- No runtime overhead beyond what the pattern already costs; prefer plain types
  and explicit flow over clever abstractions.

## Dependency layering

- `moonfield-log` is framework-layer (it depends on `moonfield-app` for
  `LogPlugin`). Crates that must stay below the framework — `moonfield-rhi`,
  `moonfield-math` and other leaves — depend on `tracing`
  directly and never on `moonfield-log`. The log format is unaffected: the
  macros are `tracing` re-exports and rendering belongs to the global
  subscriber `LogPlugin` installs.

## Testing

- Tests are written alongside source using Rust's built-in `#[cfg(test)]` module
  convention.
- When adding a feature, add a corresponding test module in the same file or a
  `tests/` directory within the crate.
- Use descriptive test function names prefixed with `test_` (e.g.
  `test_window_control_defaults`).
- Run the full suite with `cargo test` before pushing.