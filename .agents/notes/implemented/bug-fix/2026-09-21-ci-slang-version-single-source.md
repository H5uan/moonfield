# Agent Note: CI links the Slang version the bindings pin

Status: implemented

[中文](2026-09-21-ci-slang-version-single-source.zh.md)

## Problem

`test (windows-latest)` failed intermittently since 2026-09-08: the
`moonfield-render-feature` lib test binary — the first binary in the
workspace run that creates a Slang `GlobalSession` (its gpu_util tests
compile shaders on the CPU; the moonfield-ml tests need a Vulkan device and
skip on the runners) — exited with STATUS_ACCESS_VIOLATION (0xc0000005)
after all its tests passed. Running that binary 20 times directly on a
windows runner crashed 12 times with the Slang runtime CI provided, and
never with the runtime the bindings pin.

CI's `setup-slang` action exported `SLANG_DIR` pointing at Slang 2026.12,
which the `shader-slang-rs-sys` build script prefers over its own pinned
download, Slang 2026.16.1 — the version the crate's handwritten vtables are
pinned against. Slang's global-state cleanup at process exit is an upstream
teardown bug (shader-slang-rs's CI records it as 0xc0000005 on Windows and
SIGBUS on macOS; its own suite avoids the cleanup path with a never-dropped
cached session and `--test-threads=1`). The 2026.12 runtime exhibits the
bug; 2026.16.1 does not. The gpu_util tests create and drop a
`GlobalSession` per test in parallel, driving straight through that
cleanup path.

## Decision

CI sets no `SLANG_DIR` anywhere. The sys build script's `SLANG_VERSION` is
the only Slang version source, and it downloads the prebuilt release into
the cargo git checkout; the compile jobs (clippy, release-check, test)
cache that download, keyed on `Cargo.lock` — the lockfile pins the checkout
revision, which pins `SLANG_VERSION`. The `setup-slang` action is deleted.

## Alternatives considered

- **Bump the action's default version to 2026.16.1.** Rejected: the version
  would live in two places with no mechanical tie — the drift that shipped
  this bug.
- **Adopt shader-slang-rs's `--test-threads=1`.** Rejected: it serializes
  the whole workspace suite to hide an upstream teardown bug; removing the
  stale runtime removes the trigger instead.
- **Align `moonfield-rhi` with the fork's never-dropped shared session.**
  Deferred: the create-and-drop-per-test pattern is 20/20 clean on
  2026.16.1; revisit if Slang regresses.

## Consequences

- CI can no longer link a Slang runtime the bindings were not pinned
  against; version drift across the two repositories is structurally
  impossible.
- Each compile job downloads the Slang release on a cache miss (~100 MB).
- No cheap CI seam exists for the flaky exit-time AV itself — a 20-run
  windows loop per push costs too much. The single-sourced version is the
  guard; a Slang teardown regression would resurface as CI failures.
