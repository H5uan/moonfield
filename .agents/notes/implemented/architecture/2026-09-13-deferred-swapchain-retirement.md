# Agent Note: Deferred swapchain retirement on recreate

Status: implemented

[中文](2026-09-13-deferred-swapchain-retirement.zh.md)

## Problem

`WindowSurfaceData::recreate` began with `device.wait_idle()` before
rebuilding the swapchain. Resize and surface-lost events land on that path,
so dragging a window edge stalled the whole device once per frame —
present, extraction, and rendering all froze behind one call. The wait
existed for exactly one Vulkan rule: a swapchain may not be destroyed while
a frame the GPU is still processing presents it. A full-device idle is the
bluntest possible witness for that rule.

## Decision

`recreate` no longer idles the device
(`crates/moonfield-render-core/src/window.rs`):

- The old swapchain is passed to the driver as `oldSwapchain` through the
  rhi's `Swapchain::succeed`, which creates the replacement while leaving
  the old object in the caller's hands. The call *retires* the old
  swapchain (even on failure): it may no longer be acquired from, but
  already-acquired images may still be presented, and a surface may carry
  any number of retired swapchains — a native window is constrained to one
  *non-retired* swapchain, so creating without the hint while the old one
  lives is not legal. Because the old swapchain is retired even when
  `succeed` fails, a `swapchain_retired` flag steers retries to a
  hint-less `Swapchain::new` (then legal: no non-retired swapchain remains
  that would be passed twice).
- The old swapchain and depth buffer move into a per-window retirement
  list (`RetiredSwapchain`) stamped with the frame loop's submitted-frame
  counter (`FrameContext::presented_frames`).
- `create_window_surfaces` drains the list every tick: an entry drops once
  `presented_frames >= retired_at + MAX_FRAMES_IN_FLIGHT`. The frame loop's
  in-flight wait in `begin_frame` (timeline value
  `frame_submitted - MAX_FRAMES_IN_FLIGHT`) guarantees that by then every
  frame submitted at retirement time — the only frames that can still
  present the old swapchain — has completed on the queue, which is the
  guarantee `wait_idle` provided, scoped to exactly the frames that matter.
- The retired objects hold `Arc<DeviceShared>` keepalives (see
  [Shared device context for GPU object lifetimes](2026-09-13-shared-device-context-lifecycle.md)),
  so their eventual destruction is safe no matter when it happens.

Recreation is sequenced at the one moment a window is guaranteed quiet:
`create_window_surfaces` runs *before* `acquire_window_frames`, when no
window holds an acquired image (last tick's present took it). The swap
therefore never crosses an in-flight present, the same tick's acquire
targets the fresh swapchain, and a resize costs no blank tick. Acquire
treats a suboptimal result as usable — the present goes ahead and the
driver scales the image — so the window keeps presenting every tick of a
drag; only a hard `ERROR_OUT_OF_DATE` acquire produces a present-less
tick, and the swapchain is rebuilt at the next tick's start.

Evolution: the first version created the replacement with a bare
`Swapchain::new` (no `oldSwapchain`) — two non-retired swapchains on one
surface, illegal per spec, and drivers yank the old images (black flash).
The second version kept the recreate after the acquire and skipped
acquiring size-mismatched windows; measured with the editor's
`MOONFIELD_EDITOR_SIM_RESIZE` hook (which rewrites the primary window's
cached size every tick, emulating a drag), that blanked **every** tick
under continuous resize (900/900 frame ticks without any window present) —
the OS compositor fills the unpresented window with its background, which
is the flash. Moving recreation ahead of acquire and presenting suboptimal
images brings the same measurement to 0/900.

## Alternatives considered

- **Extend the rhi so `recreate` returns the old swapchain.** Equivalent
  to `succeed` in power, but mutates an existing signature; a separate
  constructor keeps the idle-first `recreate` (still valid for callers
  that idle) untouched and reads better at the call site.
- **Keep `Swapchain::new` for the replacement (no hint).** Illegal per
  spec while the old swapchain is alive — two non-retired swapchains on
  one surface — and drivers yank the old images, the first flicker cause.
- **Skip acquiring windows pending recreation.** Produces present-less
  ticks that the compositor fills with the window background; measured as
  a 100% blank rate under continuous resize (the second flicker cause).
  Suboptimal presents are spec-legal and scaled by the driver, so there is
  no reason to skip.
- **Recreate mid-frame, after acquire.** Presents the image index acquired
  from the old swapchain through the new swapchain handle (and records
  into a never-acquired image of it) — undefined content.
- **Reuse the rhi `RetirementRing` for whole swapchains.** The ring's
  `RetireAction` vocabulary is crate-internal and models teardown steps
  (destroy view/image/buffer), not whole public objects; pushing a live
  `Swapchain` into it would have required new public surface. A per-window
  list keyed by the frame loop's own counter is simpler and needs no rhi
  change.
- **Keep `wait_idle` only on the surface-lost path.** Surface-lost and
  resize share `recreate` and the same destroy-while-presenting rule;
  partial waits would keep the stall on one of the two hot paths for no
  correctness gain.

## Consequences

- Window resize neither stalls the device nor blanks a tick: the driver
  gets the `oldSwapchain` transition hint, frames never cross swapchains,
  and suboptimal frames present normally. The per-resize cost is one
  swapchain and one depth buffer allocation.
- A rapidly resizing window can hold a handful of retired swapchains
  (bounded by recreate rate × `MAX_FRAMES_IN_FLIGHT` frames); each is
  destroyed deterministically within two submitted frames.
- The rhi gains one public constructor, `Swapchain::succeed`; the
  boundary gate passes (the signature carries only crate types).
  `Swapchain::recreate` keeps its "caller must idle first" contract and
  has no workspace caller.
- `create_window_surfaces` now runs before `acquire_window_frames`; its
  recreate guard `!frame_in_progress()` states the invariant rather than
  deferring work.
- The editor binary keeps the `MOONFIELD_EDITOR_SIM_RESIZE=1` hook (next
  to `MOONFIELD_EDITOR_AUTO_CLOSE`) as the resize reproducer for future
  regression checks.
- render-core's public API is unchanged: `recreate`'s `presented_frames`
  parameter and the `swapchain_retired` flag are private, and no
  downstream crate needed edits.
