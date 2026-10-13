# Agent Note: Closing the rhi public API — no backend types escape

Status: implemented

[中文](2026-09-05-rhi-public-api-boundary-closure.zh.md)

## Problem

`moonfield-rhi`'s AGENTS.md banned leaking `Vk*` handles through public APIs,
but the rule was prose over a different reality: ~15 escape hatches
(`raw()`/`from_raw`/`borrow_raw` accessors, `vk::` types in public signatures,
`From<ash::vk::Result> for Error`). The engine layer reached through them for
three operations the RHI never grew first-class APIs for (present-support
validation, swapchain recreation with the old handle, borrowing a swapchain
image view), and the crate's own integration tests — external users of the
public API — called `.raw()` 34 times. A rule that code ignores is worse than
no rule: nobody can tell which edge is sanctioned.

## Decision

Close the boundary completely; make legitimate needs first-class APIs.

1. **First-class APIs** for the engine layer's three operations:
   `Instance::supports_present(&Device, &Surface)`, `Swapchain::recreate`
   (passes its own old handle internally), and `Swapchain::image_view(index)`
   returning a borrowed `TextureView`. `render-core`'s window layer uses only
   these now.
2. **Public-surface cleanup**: every `raw()` accessor narrowed to
   `pub(crate)`; genuinely dead items deleted (`CommandPool::raw`, the unused
   `compute_queue` field/getter, `Instance::surface_instance`,
   `Swapchain::images`/`format` getters and the `images` field). Signatures
   de-ashed: `Device::new` takes `Option<&Surface>`, `Surface::from_window`
   drops its `ash::Entry` parameter, `Swapchain::extent` returns the crate's
   `Extent2d`, `CompiledShader`'s fields became `pub(crate)`,
   `FrameUploader::upload_image` and `QueueFamilyIndices::find` went
   `pub(crate)`, and the `From<ash::vk::Result>`/`From<ash::LoadingError>`
   impls were replaced by a `pub(crate) fn Error::from_vk` plus explicit
   `map_err` at the five conversion sites.
3. **Tests moved in-crate**: the 18 integration test files (3400 lines) moved
   from `tests/` to `src/gpu_tests/` as `#[cfg(test)]` modules, where they can
   verify `pub(crate)` internals directly — a better fit for what they assert
   (descriptor slots, barriers, readback pointers). `tests/common/mod.rs`
   became `src/gpu_tests/common.rs`; the `headless_triangle` example was a
   trimmed duplicate of the test and was deleted. The graceful-skip behavior
   on driverless machines is unchanged.
4. **Mechanical enforcement**: `scripts/verify_rhi_boundary.py` fails CI (new
   `rhi-boundary` job) if any public item in `moonfield-rhi/src` mentions
   `ash::`/`vk::`/`gpu_allocator`. The rule lives in
   `crates/moonfield-rhi/AGENTS.md`; the smoke-test command is now
   `cargo test -p moonfield-rhi gpu_tests::headless_triangle`.

## Alternatives considered

- **Keep the hatches and document them as sanctioned interop.** Rejected: the
  prose rule had already failed to hold; a documented-hatch policy still
  leaves consumers free to build on raw handles, and every such use makes the
  boundary harder to restore.
- **A feature-gated or `#[doc(hidden)]` internal API for the tests.**
  Rejected: still an escape hatch — public is public, and "hidden" items
  become load-bearing the moment a downstream crate discovers them.
- **Narrow only what nothing uses, leave the rest public.** Rejected: a
  partial boundary invites the next leak; the full inventory showed every
  remaining hatch was used only in-crate anyway, so closure cost little.

## Consequences

- `moonfield-rhi`'s public API contains no `ash`, `vk::`, or `gpu_allocator`
  types; `scripts/verify_rhi_boundary.py` proves it on every push.
- The RHI is reusable standalone: its dependency cone is `moonfield-math` and
  external crates, and consumers cannot take a dependency on Vulkan internals.
- Adding a capability now means growing a first-class API — deliberate
  friction that keeps the boundary pure.
- GPU tests run as in-crate unit tests (42 tests, still skip gracefully
  without a compatible driver); the crate has no `tests/` or `examples/` left.
- `Stage`'s `pub const` initializers mention `vk::` in their *values* — not a
  leak (the public type ascription is the crate's own `Stage`); the guard
  script treats const/static initializers as implementation details.
