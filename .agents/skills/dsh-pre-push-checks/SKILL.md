---
name: dsh-pre-push-checks
description: Use before pushing to master or opening a pull request in the moonfield repo — the ordered local checks that match the diff's surface, plus the Agent Note gate
---

# Pre-push checks

Checks that must pass before a push or PR. CI owns the exhaustive matrix; run
the relevant subset locally and report only what you actually ran.

## Always

1. `cargo fmt --all -- --check` — formatting.
2. `cargo clippy --workspace --all-targets -- -D warnings` — warnings.
3. `python3 scripts/verify_agents.py` — the Agent Notes gate (format,
   classification, lifecycle consistency, bilingual pairing, links).

## Depends on the surface

- Behavior change in a crate → that crate's tests:
  `cargo test -p <crate>`.
- Vulkan/RHI change → `cargo check -p moonfield-render` and
  `cargo test -p moonfield-render --test headless_triangle`.
- Docs or Agent Notes → already covered by `verify_agents.py`.

Match evidence to the change; do not default to `cargo test --workspace` unless
asked or the change is irreducibly repo-wide.

## Agent Note rule

A non-trivial change MUST add or update an Agent Note in the same commit
(scope: .agents/notes/README.md). Check the diff for this before pushing — the
CI gate only verifies existing notes; the author owns whether one is required.