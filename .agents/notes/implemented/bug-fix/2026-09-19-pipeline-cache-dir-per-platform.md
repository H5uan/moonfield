# Agent Note: Per-platform pipeline cache directory

Status: implemented

[中文](2026-09-19-pipeline-cache-dir-per-platform.zh.md)

## Problem

`pipeline_cache_path` (crates/moonfield-rhi/src/vulkan/device.rs) resolved the
Vulkan pipeline cache location from `XDG_CACHE_HOME` with a `HOME/.cache`
fallback and `unwrap_or_default()` past that. On Windows neither variable is
normally set, so the root came out empty and the path
`moonfield/pipeline_cache.bin` became relative to the process working
directory: the cache landed wherever the editor happened to be launched from
(the repository root accumulated one such file), was never shared between
runs, and polluted any directory the process ran in.

## Decision

The cache root is per-platform, resolved with `std::env` only (the workspace
does not depend on `dirs`):

- Windows: `%LOCALAPPDATA%`, falling back to `%APPDATA%`.
- Other platforms: `$XDG_CACHE_HOME`, falling back to `~/.cache` — unchanged
  behavior.
- Both platforms: when no cache root is set, the temp dir
  (`std::env::temp_dir`), which is always absolute, so the path can never
  silently become working-directory-relative again.

The result is always `<root>/moonfield/pipeline_cache.bin`. All failure modes
on top of the path were already non-fatal and stay so: a missing or corrupt
file seeds an empty cache, a driver-rejected blob falls back to a cold cache,
and write-back failures in `Device::drop` log a warning — a cache miss never
fails device creation.

## Alternatives considered

- **Depend on the `dirs` crate.** Rejected: it is not in the dependency graph
  today, and the two platforms the workspace supports (Windows and Linux) need
  exactly the environment lookups above; a dependency would add a supply-chain
  edge for three `std::env::var_os` calls.
- **Anchor the fallback to the executable or asset directory.** Rejected: an
  installed binary directory is often read-only, and "next to the exe" couples
  a per-user, per-driver cache to the install layout; the temp dir is the only
  location guaranteed writable and absolute.
- **Disable the cache when no root resolves.** Rejected: silently dropping the
  cache costs pipeline-compile time on every run for a configuration error
  that the temp-dir fallback already covers.

## Consequences

- On Windows the cache lives under `%LOCALAPPDATA%\moonfield\pipeline_cache.bin`
  and is shared across runs and working directories; driver-level pipeline
  reuse now works there.
- Cache files written by earlier runs under stray working directories are
  orphaned; they are not cleaned up by the code (each is a few KB, and the
  owning directory is unknowable in general).
- The temp-dir fallback means cache reads/writes may target the temp dir on
  minimally configured systems; contents remain disposable by construction.
